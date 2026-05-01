use std::path::Path;
use std::time::Instant;

use crate::internal::actor;
use crate::internal::clock::SystemClock;
use crate::internal::create;
use crate::internal::error::ForumResult;
use crate::internal::event::{self, Lifecycle, NodeType, ThreadKind};
use crate::internal::evidence;
use crate::internal::git_ops::GitOps;
use crate::internal::index::{self, ThreadRow};
use crate::internal::node::Node;
use crate::internal::refs;
use crate::internal::reindex;
use crate::internal::show;
use crate::internal::thread;
use crate::internal::write_ops;

use super::{App, LinkOrigin, LinkTargetKind, NodeStatusAction, TreeEntry, View};

pub(super) fn open_thread_detail(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    selected_node_id: Option<&str>,
) -> ForumResult<()> {
    let ref_name = format!("refs/forum/threads/{thread_id}");
    let tip_sha = git.resolve_ref(&ref_name)?.unwrap_or_default();
    let state = thread::replay_thread(git, thread_id)?;

    app.thread_title = state.title.clone();
    app.thread_lifecycle = Some(state.lifecycle().as_str().to_string());
    // SPEC-2.0 §2.3.3: unmigrated 1.x threads (no facet_set) display the
    // conventional tag derived from kind; migrated threads use replayed tags.
    app.thread_tags = if state.lifecycle.is_none() && state.tags.is_empty() {
        super::render::conventional_tags_for_kind(state.kind)
    } else {
        state.tags.clone()
    };
    app.thread_status = state.status.to_string();
    app.thread_text = show::render_show(&state, &show::ShowOptions::default());
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
    app.node_detail_text = show::render_node_show(&lookup, &show::ShowOptions::default());
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
) -> ForumResult<()> {
    let title = app.thread_form.title.trim();
    if title.is_empty() {
        return Ok(());
    }

    // Parse + validate tags (§2.3.5 grammar). Form blocks submission on error.
    let parsed_tags = match parse_tag_input(&app.thread_form.tags) {
        Ok(tags) => tags,
        Err(msg) => {
            app.thread_form.tag_error = Some(msg);
            return Ok(());
        }
    };
    app.thread_form.tag_error = None;

    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let lifecycle = thread_lifecycle_values()[app.thread_form.lifecycle_index];
    let body = if app.thread_form.body.trim().is_empty() {
        None
    } else {
        Some(app.thread_form.body.trim())
    };

    // §2.3.3 / §9.1: when (lifecycle, tags) matches a kind preset shape, the
    // form invokes the preset path (no facet_set). Otherwise it falls back to
    // a kind-by-lifecycle default and writes a facet_set for the user's tags.
    let preset = match_kind_preset(lifecycle, &parsed_tags);
    let kind = preset.unwrap_or_else(|| default_kind_for_lifecycle(lifecycle));

    let thread_id = create::create_thread(git, kind, title, body, &actor, &clock)?;
    if preset.is_none() {
        write_ops::write_facet_set(git, &thread_id, None, &parsed_tags, &[], &actor, &clock)?;
    }
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    if let Some(pos) = app.threads.iter().position(|row| row.id == thread_id) {
        app.table_state.select(Some(pos));
    }
    open_thread_detail(app, git, &thread_id, None)
}

pub(super) fn submit_create_node(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    thread_id: &str,
) -> ForumResult<()> {
    let body = app.node_form.body.trim();
    if body.is_empty() {
        return Ok(());
    }

    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let node_type = node_type_values()[app.node_form.node_type_index];
    let node_id = write_ops::say_node(git, thread_id, node_type, body, &actor, &clock, None)?;
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    open_thread_detail(app, git, thread_id, Some(&node_id))
}

pub(super) fn submit_create_link(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
) -> ForumResult<()> {
    let Some(target_thread_id) = selected_link_target(app, thread_id) else {
        return Ok(());
    };

    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let relation = link_relation_labels()[app.link_form.relation_index];
    evidence::add_thread_link(git, thread_id, &target_thread_id, relation, &actor, &clock)?;
    return_from_link_form(app, git, thread_id, origin)
}

pub(super) fn return_from_link_form(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
) -> ForumResult<()> {
    match origin {
        LinkOrigin::ThreadDetail { selected_node_id } => {
            open_thread_detail(app, git, thread_id, selected_node_id.as_deref())
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

pub(super) fn thread_lifecycle_values() -> [Lifecycle; 3] {
    [Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record]
}

pub(super) fn thread_lifecycle_labels() -> [&'static str; 3] {
    ["proposal", "execution", "record"]
}

pub(super) fn default_thread_lifecycle_index(lifecycle_filter: Option<&str>) -> usize {
    match lifecycle_filter {
        Some("proposal") => 0,
        Some("execution") => 1,
        Some("record") => 2,
        _ => 1, // execution default — most common preset target
    }
}

/// SPEC-2.0 §2.5: the four canonical 2.0 node types offered for creation.
/// Legacy prose-only types (claim, question, summary, risk, review,
/// alternative, assumption) are no longer offered; pre-existing nodes still
/// render with their stored label.
pub(super) fn node_type_values() -> [NodeType; 4] {
    [
        NodeType::Comment,
        NodeType::Approval,
        NodeType::Objection,
        NodeType::Action,
    ]
}

pub(super) fn node_type_labels() -> [&'static str; 4] {
    ["comment", "approval", "objection", "action"]
}

/// Parse a comma-separated tag list per §2.3.5 grammar.
/// Empty / whitespace-only entries are dropped; remaining tags are validated.
pub(super) fn parse_tag_input(input: &str) -> Result<Vec<String>, String> {
    let mut tags = Vec::new();
    for raw in input.split(',') {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        event::validate_tag(t).map_err(|e| e.to_string())?;
        if !tags.iter().any(|x: &String| x == t) {
            tags.push(t.to_string());
        }
    }
    Ok(tags)
}

/// SPEC-2.0 §2.3.3 + §9.1: detect whether (lifecycle, tags) matches one of
/// the four kind-preset shapes so the form can invoke the preset path.
pub(super) fn match_kind_preset(lifecycle: Lifecycle, tags: &[String]) -> Option<ThreadKind> {
    match (lifecycle, tags) {
        (Lifecycle::Proposal, t) if t == ["cross-cutting"] => Some(ThreadKind::Rfc),
        (Lifecycle::Record, []) => Some(ThreadKind::Dec),
        (Lifecycle::Execution, t) if t == ["task"] => Some(ThreadKind::Task),
        (Lifecycle::Execution, t) if t == ["bug"] => Some(ThreadKind::Issue),
        _ => None,
    }
}

/// Fallback kind when the form's (lifecycle, tags) combination doesn't match
/// a §2.3.3 preset. The kind only needs to share the chosen lifecycle.
pub(super) fn default_kind_for_lifecycle(lifecycle: Lifecycle) -> ThreadKind {
    match lifecycle {
        Lifecycle::Proposal => ThreadKind::Rfc,
        Lifecycle::Execution => ThreadKind::Issue,
        Lifecycle::Record => ThreadKind::Dec,
    }
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
                        super::render::display_thread_id(&row.id),
                        super::render::single_line_preview(&row.title, 28)
                    )
                })
                .unwrap_or_else(|| "(no matching threads)".to_string())
        }
    }
}

/// Incremental list refresh: compare git ref SHAs against the SQLite index and
/// only replay threads whose SHA has changed. Returns true if any changes were made.
fn incremental_refresh(git: &GitOps, conn: &rusqlite::Connection) -> ForumResult<bool> {
    let current_refs = git.list_refs_with_shas(refs::THREADS_PREFIX)?;
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
            if let Ok(state) = thread::replay_thread(git, thread_id) {
                let _ = index::upsert_thread(conn, &state)
                    .and_then(|_| index::replace_nodes_for_thread(conn, &state))
                    .and_then(|_| index::replace_evidence_for_thread(conn, &state))
                    .and_then(|_| index::replace_links_for_thread(conn, &state));
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
) -> ForumResult<()> {
    match &app.view {
        View::ThreadDetail(thread_id) => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            if let Ok(Some(current_sha)) = git.resolve_ref(&ref_name) {
                let changed = app
                    .thread_tip_sha
                    .as_ref()
                    .is_none_or(|prev| *prev != current_sha);
                if changed {
                    let selected = app.selected_node_id();
                    let thread_id = thread_id.clone();
                    open_thread_detail(app, git, &thread_id, selected.as_deref())?;
                }
            }
        }
        View::NodeDetail { thread_id, node_id } => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            if let Ok(Some(current_sha)) = git.resolve_ref(&ref_name) {
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
            let changed = incremental_refresh(git, conn)?;
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
