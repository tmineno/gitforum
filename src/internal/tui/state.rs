use std::path::Path;
use std::time::Instant;

use crate::internal::actor;
use crate::internal::clock::SystemClock;
use crate::internal::create;
use crate::internal::error::ForumResult;
use crate::internal::event::{NodeType, ThreadKind};
use crate::internal::evidence_ops;
use crate::internal::git_ops::GitOps;
use crate::internal::index::{self, ThreadRow};
use crate::internal::node::Node;
use crate::internal::refs;
use crate::internal::reindex;
use crate::internal::show;
use crate::internal::thread;
use crate::internal::write_ops;

use super::perf::Perf;
use super::{App, LinkOrigin, LinkTargetKind, NodeStatusAction, TreeEntry, View};

pub(super) fn open_thread_detail(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    selected_node_id: Option<&str>,
    perf: &mut Perf,
) -> ForumResult<()> {
    // Resolve the tip SHA for cache lookup and change detection
    let ref_name = format!("refs/forum/threads/{thread_id}");
    let tip_sha = git.resolve_ref(&ref_name)?.unwrap_or_default();

    // Try the replay cache first
    let state = if let Some(cached) = app.replay_cache.get(thread_id, &tip_sha) {
        perf.record(
            "replay_thread_cached",
            Some(thread_id),
            std::time::Duration::ZERO,
        );
        cached.clone()
    } else {
        let t = Instant::now();
        let replayed = thread::replay_thread(git, thread_id)?;
        perf.record("replay_thread", Some(thread_id), t.elapsed());
        app.replay_cache
            .insert(thread_id.to_string(), tip_sha.clone(), replayed.clone());
        replayed
    };

    app.thread_title = state.title.clone();
    app.thread_kind = state.kind.to_string();
    app.thread_status = state.status.to_string();
    app.thread_text = show::render_show(&state, false);
    app.thread_scroll = 0;
    app.thread_nodes = state.nodes;
    app.tree_entries = build_tree_entries(&app.thread_nodes);
    app.recompute_visible_tree();
    app.node_detail_text.clear();
    app.node_detail_scroll = 0;
    app.thread_tip_sha = Some(tip_sha);
    app.last_refresh = Instant::now();
    app.select_node_by_id(selected_node_id);
    app.view = View::ThreadDetail(thread_id.to_string());
    Ok(())
}

pub(super) fn open_node_detail(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
) -> ForumResult<()> {
    let lookup = thread::find_node_in_thread(git, thread_id, node_id)?;
    app.node_detail_text = show::render_node_show(&lookup);
    app.node_detail_scroll = 0;
    app.view = View::NodeDetail {
        thread_id: thread_id.to_string(),
        node_id: lookup.node.node_id,
    };
    Ok(())
}

pub(super) fn apply_node_status_action(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    action: NodeStatusAction,
) -> ForumResult<()> {
    let lookup = thread::find_node_in_thread(git, thread_id, node_id)?;
    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;

    match action {
        NodeStatusAction::Resolve if !lookup.node.resolved && !lookup.node.retracted => {
            write_ops::resolve_node(git, thread_id, &lookup.node.node_id, &actor, &clock)?;
        }
        NodeStatusAction::Reopen
            if lookup.node.resolved || lookup.node.retracted || lookup.node.incorporated =>
        {
            write_ops::reopen_node(git, thread_id, &lookup.node.node_id, &actor, &clock)?;
        }
        NodeStatusAction::Retract if !lookup.node.retracted => {
            write_ops::retract_node(git, thread_id, &lookup.node.node_id, &actor, &clock)?;
        }
        _ => {}
    }

    open_node_detail(app, git, thread_id, &lookup.node.node_id)
}

pub(super) fn submit_create_thread(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    perf: &mut Perf,
) -> ForumResult<()> {
    let title = app.thread_form.title.trim();
    if title.is_empty() {
        return Ok(());
    }

    let t = Instant::now();
    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let kind = thread_kind_values()[app.thread_form.kind_index];
    let body = if app.thread_form.body.trim().is_empty() {
        None
    } else {
        Some(app.thread_form.body.trim())
    };

    let thread_id = create::create_thread(git, kind, title, body, &actor, &clock)?;
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    perf.record("submit_create", Some(&thread_id), t.elapsed());
    if let Some(pos) = app.threads.iter().position(|row| row.id == thread_id) {
        app.table_state.select(Some(pos));
    }
    open_thread_detail(app, git, &thread_id, None, perf)
}

pub(super) fn submit_create_node(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    thread_id: &str,
    perf: &mut Perf,
) -> ForumResult<()> {
    let body = app.node_form.body.trim();
    if body.is_empty() {
        return Ok(());
    }

    let t = Instant::now();
    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let node_type = node_type_values()[app.node_form.node_type_index];
    let node_id = write_ops::say_node(git, thread_id, node_type, body, &actor, &clock, None)?;
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    perf.record("submit_create", Some(thread_id), t.elapsed());
    open_thread_detail(app, git, thread_id, Some(&node_id), perf)
}

pub(super) fn submit_create_link(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
    perf: &mut Perf,
) -> ForumResult<()> {
    let Some(target_thread_id) = selected_link_target(app, thread_id) else {
        return Ok(());
    };

    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let relation = link_relation_labels()[app.link_form.relation_index];
    evidence_ops::add_thread_link(git, thread_id, &target_thread_id, relation, &actor, &clock)?;
    return_from_link_form(app, git, thread_id, origin, perf)
}

pub(super) fn return_from_link_form(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
    perf: &mut Perf,
) -> ForumResult<()> {
    match origin {
        LinkOrigin::ThreadDetail { selected_node_id } => {
            open_thread_detail(app, git, thread_id, selected_node_id.as_deref(), perf)
        }
        LinkOrigin::NodeDetail { node_id } => open_node_detail(app, git, thread_id, node_id),
    }
}

#[doc(hidden)]
pub fn load_threads(git: &GitOps, db_path: &Path) -> ForumResult<Vec<ThreadRow>> {
    reindex::run_reindex(git, db_path)?;
    let conn = index::open_db(db_path)?;
    index::list_threads(&conn)
}

/// Build tree-ordered entries from a flat list of nodes using reply_to relationships.
///
/// Returns entries in depth-first order with tree connector prefixes.
pub(super) fn build_tree_entries(nodes: &[Node]) -> Vec<TreeEntry> {
    use std::collections::HashMap;

    // Build index: node_id -> position
    let id_to_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.node_id.as_str(), i))
        .collect();

    // Build children map: parent_id -> [child indices]
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut has_parent = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if let Some(ref parent_id) = node.reply_to {
            if let Some(&parent_idx) = id_to_idx.get(parent_id.as_str()) {
                children.entry(parent_idx).or_default().push(i);
                has_parent[i] = true;
            }
        }
    }

    // Roots are nodes without a parent (or whose parent is not in this thread)
    let roots: Vec<usize> = (0..nodes.len()).filter(|&i| !has_parent[i]).collect();

    let mut entries = Vec::with_capacity(nodes.len());

    // DFS with prefix tracking
    fn walk(
        idx: usize,
        depth: u16,
        prefix: &str,
        is_last: bool,
        children: &HashMap<usize, Vec<usize>>,
        entries: &mut Vec<TreeEntry>,
    ) {
        let connector = if depth == 0 {
            String::new()
        } else if is_last {
            format!("{prefix}└─")
        } else {
            format!("{prefix}├─")
        };
        entries.push(TreeEntry {
            node_index: idx,
            depth,
            prefix: connector,
            has_children: children.contains_key(&idx),
        });

        if let Some(child_indices) = children.get(&idx) {
            let n = child_indices.len();
            for (i, &child_idx) in child_indices.iter().enumerate() {
                let child_prefix = if depth == 0 {
                    String::new()
                } else if is_last {
                    format!("{prefix}  ")
                } else {
                    format!("{prefix}│ ")
                };
                walk(
                    child_idx,
                    depth + 1,
                    &child_prefix,
                    i == n - 1,
                    children,
                    entries,
                );
            }
        }
    }

    let n = roots.len();
    for (i, &root_idx) in roots.iter().enumerate() {
        walk(root_idx, 0, "", i == n - 1, &children, &mut entries);
    }

    entries
}

pub(super) fn thread_kind_values() -> [ThreadKind; 4] {
    [ThreadKind::Issue, ThreadKind::Rfc, ThreadKind::Dec, ThreadKind::Task]
}

pub(super) fn thread_kind_labels() -> [&'static str; 4] {
    ["issue", "rfc", "dec", "task"]
}

pub(super) fn default_thread_kind_index(kind_filter: Option<&str>) -> usize {
    match kind_filter {
        Some("issue") | Some("ask") => 0,
        Some("rfc") => 1,
        Some("dec") => 2,
        Some("task") | Some("job") => 3,
        _ => 0,
    }
}

pub(super) fn node_type_values() -> [NodeType; 7] {
    [
        NodeType::Claim,
        NodeType::Question,
        NodeType::Objection,
        NodeType::Evidence,
        NodeType::Summary,
        NodeType::Action,
        NodeType::Risk,
    ]
}

pub(super) fn node_type_labels() -> [&'static str; 7] {
    [
        "claim",
        "question",
        "objection",
        "evidence",
        "summary",
        "action",
        "risk",
    ]
}

pub(super) fn link_relation_labels() -> [&'static str; 4] {
    ["implements", "relates-to", "depends-on", "blocks"]
}

pub(super) fn link_target_kind_values() -> [LinkTargetKind; 5] {
    [
        LinkTargetKind::Issue,
        LinkTargetKind::Rfc,
        LinkTargetKind::Dec,
        LinkTargetKind::Task,
        LinkTargetKind::Manual,
    ]
}

pub(super) fn link_target_kind_labels() -> [&'static str; 5] {
    ["issue", "rfc", "dec", "task", "manual"]
}

pub(super) fn thread_kind_matches_target(kind: &str, target_kind: LinkTargetKind) -> bool {
    match target_kind {
        LinkTargetKind::Issue => kind == "issue",
        LinkTargetKind::Rfc => kind == "rfc",
        LinkTargetKind::Dec => kind == "dec",
        LinkTargetKind::Task => kind == "task",
        LinkTargetKind::Manual => false,
    }
}

pub(super) fn auto_link_candidates<'a>(app: &'a App, source_thread_id: &str) -> Vec<&'a ThreadRow> {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    if target_kind == LinkTargetKind::Manual {
        return Vec::new();
    }

    app.threads
        .iter()
        .filter(|row| {
            row.id != source_thread_id && thread_kind_matches_target(&row.kind, target_kind)
        })
        .collect()
}

pub(super) fn selected_link_target(app: &App, source_thread_id: &str) -> Option<String> {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    match target_kind {
        LinkTargetKind::Manual => {
            let target = app.link_form.manual_target.trim();
            (!target.is_empty()).then(|| target.to_string())
        }
        _ => auto_link_candidates(app, source_thread_id)
            .get(app.link_form.target_index)
            .map(|row| row.id.clone()),
    }
}

pub(super) fn selected_link_target_label(app: &App, source_thread_id: &str) -> String {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    match target_kind {
        LinkTargetKind::Manual => {
            let target = app.link_form.manual_target.trim();
            if target.is_empty() {
                "(enter thread id)".to_string()
            } else {
                target.to_string()
            }
        }
        _ => {
            let candidates = auto_link_candidates(app, source_thread_id);
            candidates
                .get(app.link_form.target_index)
                .map(|row| {
                    format!(
                        "{}  {}",
                        row.id,
                        super::render::single_line_preview(&row.title, 28)
                    )
                })
                .unwrap_or_else(|| "(no matching threads)".to_string())
        }
    }
}

/// Incremental list refresh: compare git ref SHAs against the SQLite index and
/// only replay threads whose SHA has changed. Returns true if any changes were made.
fn incremental_refresh(
    git: &GitOps,
    conn: &rusqlite::Connection,
    perf: &mut Perf,
) -> ForumResult<bool> {
    let t = Instant::now();
    let current_refs = git.list_refs_with_shas(refs::THREADS_PREFIX)?;
    perf.record("incremental_ref_scan", None, t.elapsed());

    let stored_shas = index::thread_tip_shas(conn)?;

    // Build map of current thread_id -> ref tip SHA
    let mut current: std::collections::HashMap<String, String> =
        std::collections::HashMap::with_capacity(current_refs.len());
    for (refname, sha) in &current_refs {
        if let Some(thread_id) = refs::thread_id_from_ref(refname) {
            current.insert(thread_id.to_string(), sha.clone());
        }
    }

    let mut changed = false;

    // Replay new or changed threads
    for (thread_id, sha) in &current {
        let needs_update = match stored_shas.get(thread_id) {
            Some(stored) => stored != sha,
            None => true,
        };
        if needs_update {
            let rt = Instant::now();
            if let Ok(state) = thread::replay_thread(git, thread_id) {
                perf.record("replay_thread", Some(thread_id), rt.elapsed());
                let _ = index::upsert_thread(conn, &state)
                    .and_then(|_| index::replace_nodes_for_thread(conn, &state))
                    .and_then(|_| index::replace_evidence_for_thread(conn, &state));
            }
            changed = true;
        }
    }

    // Remove deleted threads
    for stored_id in stored_shas.keys() {
        if !current.contains_key(stored_id) {
            let _ = index::delete_thread(conn, stored_id);
            changed = true;
        }
    }

    Ok(changed)
}

/// Auto-refresh: check if the currently viewed thread or list has changed, and refresh if so.
pub(super) fn auto_refresh(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    _db_path: &Path,
    perf: &mut Perf,
) -> ForumResult<()> {
    match &app.view {
        View::ThreadDetail(thread_id) => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            let t = Instant::now();
            let resolved = git.resolve_ref(&ref_name);
            perf.record("resolve_ref", Some(thread_id), t.elapsed());
            if let Ok(Some(current_sha)) = resolved {
                let changed = app
                    .thread_tip_sha
                    .as_ref()
                    .is_none_or(|prev| *prev != current_sha);
                if changed {
                    let selected = app.selected_node_id();
                    let thread_id = thread_id.clone();
                    open_thread_detail(app, git, &thread_id, selected.as_deref(), perf)?;
                }
            }
        }
        View::NodeDetail { thread_id, node_id } => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            let t = Instant::now();
            let resolved = git.resolve_ref(&ref_name);
            perf.record("resolve_ref", Some(thread_id), t.elapsed());
            if let Ok(Some(current_sha)) = resolved {
                let changed = app
                    .thread_tip_sha
                    .as_ref()
                    .is_none_or(|prev| *prev != current_sha);
                if changed {
                    let thread_id = thread_id.clone();
                    let node_id = node_id.clone();
                    app.thread_tip_sha = Some(current_sha);
                    open_node_detail(app, git, &thread_id, &node_id)?;
                }
            }
        }
        View::List => {
            let t = Instant::now();
            let changed = incremental_refresh(git, conn, perf)?;
            perf.record("list_refresh", None, t.elapsed());
            if changed {
                let threads = index::list_threads(conn)?;
                let sel = app.table_state.selected().unwrap_or(0);
                app.threads = threads;
                let n = app.visible_threads().len();
                app.table_state
                    .select(if n > 0 { Some(sel.min(n - 1)) } else { None });
            }
        }
        _ => {}
    }
    Ok(())
}
