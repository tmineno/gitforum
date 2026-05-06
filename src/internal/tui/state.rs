use std::time::Instant;

use crate::internal::actor;
use crate::internal::clock::{Clock, SystemClock};
use crate::internal::commands::show;
use crate::internal::error::{ForumError, ForumResult};
use crate::internal::evidence::EvidenceFile;
use crate::internal::git_ops::GitOps;
use crate::internal::id_alloc;
use crate::internal::node::{NodeKind, NodeRecord, NodeStatus};
use crate::internal::policy;
use crate::internal::snapshot::history;
use crate::internal::snapshot::list::{self as snapshot_list, ThreadRow};
use crate::internal::snapshot::{self, store::write_snapshot, Link, Links, NodeWithBody};
use crate::internal::thread::{self, ThreadSnapshot};

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
    app.thread_lifecycle =
        Some(policy::lifecycle_label_for(&state.category, &state.tags).to_string());
    // 3.0 snapshots always carry `lifecycle_explicit = true` (set by
    // `materialize_thread_state_from_snapshot`) and a populated tag
    // set; the v3.1 step 3n removal of the typed `state.kind` made
    // the previous "kind-derived conventional tags" fallback dead
    // code.
    app.thread_tags = state.tags.clone();
    app.thread_status = state.status.clone();
    // Phase 4 Step 1d (RFC `7ymtc4b2`): per-thread timeline panel reads
    // the snapshot ref's git history (SPEC-3.0 §5.4). `read_log` returns
    // `None` on legacy event-chain refs — the renderer falls back to its
    // "_(timeline unavailable)_" placeholder, matching the same shape
    // CLI `git forum show` produces on those refs.
    let timeline_entries = history::read_log(git, &ref_name).ok();
    app.thread_text = show::render_show(
        &state,
        &show::ShowOptions {
            timeline_entries,
            ..show::ShowOptions::default()
        },
    );
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
    // Phase 4 Step 1d (RFC `7ymtc4b2`): per-node history panel uses the
    // same snapshot ref log as the thread view. `render_node_show` does
    // the path-filter to commits that touched `nodes/<id>.{toml,md}`.
    let ref_name = format!("refs/forum/threads/{thread_id}");
    let timeline_entries = history::read_log(git, &ref_name).ok();
    app.node_detail_text = show::render_node_show(
        &lookup,
        &show::ShowOptions {
            timeline_entries,
            ..show::ShowOptions::default()
        },
    );
    app.node_detail_scroll = 0;
    app.view = View::NodeDetail {
        thread_id: thread_id.to_string(),
        node_id: lookup.node.record.id,
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

    let cur = lookup.node.record.status;
    let new_status = match action {
        NodeStatusAction::Resolve
            if cur != NodeStatus::Resolved && cur != NodeStatus::Retracted =>
        {
            Some(NodeStatus::Resolved)
        }
        NodeStatusAction::Reopen if cur != NodeStatus::Open => Some(NodeStatus::Open),
        NodeStatusAction::Retract if cur != NodeStatus::Retracted => Some(NodeStatus::Retracted),
        _ => None,
    };
    if let Some(status) = new_status {
        snapshot_update_node_status(
            git,
            thread_id,
            &lookup.node.record.id,
            status,
            &actor,
            &clock,
        )?;
    }

    open_node_detail(app, git, thread_id, &lookup.node.record.id)
}

pub(super) fn submit_create_thread(app: &mut App, git: &GitOps) -> ForumResult<()> {
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

    // SPEC-3.0 §8.3: lifecycle drives the canonical category + decision tag;
    // user-supplied tags merge in. The legacy preset / write_facet_set
    // fork is gone — slot 10c writes the snapshot directly.
    let thread_id =
        snapshot_create_thread(git, title, body, lifecycle, &parsed_tags, &actor, &clock)?;
    refresh_thread_list(app, git)?;
    if let Some(pos) = app.threads.iter().position(|row| row.id == thread_id) {
        app.table_state.select(Some(pos));
    }
    open_thread_detail(app, git, &thread_id, None)
}

pub(super) fn submit_create_node(app: &mut App, git: &GitOps, thread_id: &str) -> ForumResult<()> {
    let body = app.node_form.body.trim();
    if body.is_empty() {
        return Ok(());
    }

    let actor = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    let node_type = node_type_values()[app.node_form.node_type_index];
    let node_id = snapshot_append_node(git, thread_id, node_type, body, &actor, &clock)?;
    refresh_thread_list(app, git)?;
    open_thread_detail(app, git, thread_id, Some(&node_id))
}

/// Reload the snapshot-derived thread list and re-cache tip SHAs.
/// Called after any TUI mutation that may have changed a ref.
fn refresh_thread_list(app: &mut App, git: &GitOps) -> ForumResult<()> {
    app.threads = snapshot_list::list_threads(git)?;
    app.list_tip_shas = snapshot_list::thread_tip_shas(git)?;
    Ok(())
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
    snapshot_append_link(git, thread_id, &target_thread_id, relation, &actor, &clock)?;
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
pub fn load_threads(git: &GitOps) -> ForumResult<Vec<ThreadRow>> {
    snapshot_list::list_threads(git)
}

/// Re-export of `snapshot::list::thread_tip_shas` so `tui::run` can
/// seed `App.list_tip_shas` without itself depending on the snapshot
/// listing module.
pub fn snapshot_list_tip_shas(
    git: &GitOps,
) -> ForumResult<std::collections::HashMap<String, String>> {
    snapshot_list::thread_tip_shas(git)
}

/// Build tree-ordered entries from a flat list of nodes using reply_to relationships.
///
/// Returns entries in depth-first order with tree connector prefixes. Siblings
/// at every depth are ordered by `created_at` (then by id as a stable
/// tiebreaker) so the timeline reads chronologically — `read_snapshot` loads
/// nodes id-sorted for determinism, but random node IDs carry no temporal
/// meaning, so surfacing that order in the TUI looks scrambled to users.
pub(super) fn build_tree_entries(nodes: &[NodeWithBody]) -> Vec<TreeEntry> {
    use std::collections::HashMap;

    // Build index: node_id -> position
    let id_to_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.record.id.as_str(), i))
        .collect();

    let sort_key = |idx: &usize| {
        let r = &nodes[*idx].record;
        (r.created_at, r.id.clone())
    };

    // Build children map: parent_id -> [child indices]
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut has_parent = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if let Some(ref parent_id) = node.record.reply_to {
            if let Some(&parent_idx) = id_to_idx.get(parent_id.as_str()) {
                children.entry(parent_idx).or_default().push(i);
                has_parent[i] = true;
            }
        }
    }
    for child_indices in children.values_mut() {
        child_indices.sort_by_key(&sort_key);
    }

    // Roots are nodes without a parent (or whose parent is not in this thread)
    let mut roots: Vec<usize> = (0..nodes.len()).filter(|&i| !has_parent[i]).collect();
    roots.sort_by_key(&sort_key);

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

pub(super) fn thread_lifecycle_values() -> [&'static str; 3] {
    ["proposal", "execution", "record"]
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

/// SPEC-3.0 §2.2: the four canonical node kinds offered for creation.
/// Legacy 1.x prose-only types (claim, question, summary, risk, review,
/// alternative, assumption) are no longer offered; pre-existing nodes
/// still render with their stored `legacy_subtype` label.
pub(super) fn node_type_values() -> [NodeKind; 4] {
    [
        NodeKind::Comment,
        NodeKind::Approval,
        NodeKind::Objection,
        NodeKind::Action,
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
        thread::validate_tag(t).map_err(|e| e.to_string())?;
        if !tags.iter().any(|x: &String| x == t) {
            tags.push(t.to_string());
        }
    }
    Ok(tags)
}

// `match_kind_preset` / `default_kind_for_lifecycle` removed at slot 10c
// (RFC `7ymtc4b2`). The TUI form's create-thread path now writes a
// SPEC-3.0 snapshot directly via `snapshot_create_thread`; the legacy
// preset / write_facet_set fork (kind axis selection) is gone.

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

/// Compare current `refs/forum/threads/*` tip SHAs against the
/// last-seen snapshot map. Returns `true` when any ref has been
/// added, removed, or moved since the previous tick.
///
/// Phase 4 Step 1c (RFC `7ymtc4b2`, task `913c4s9v`): replaces the
/// SQLite-backed `incremental_refresh` that compared against
/// `index::thread_tip_shas`. The TUI tracks the seen SHA set
/// in-process via `App.list_tip_shas` instead of round-tripping
/// through SQLite — per RFC Exceptions, no index is reintroduced
/// for v3.0.0.
fn list_changed_since(
    git: &GitOps,
    seen: &std::collections::HashMap<String, String>,
) -> ForumResult<(bool, std::collections::HashMap<String, String>)> {
    let current = snapshot_list::thread_tip_shas(git)?;
    let mut changed = current.len() != seen.len();
    if !changed {
        for (id, sha) in &current {
            if seen.get(id).map(|s| s.as_str()) != Some(sha.as_str()) {
                changed = true;
                break;
            }
        }
    }
    Ok((changed, current))
}

/// Auto-refresh: check if the currently viewed thread or list has changed, and refresh if so.
pub(super) fn auto_refresh(app: &mut App, git: &GitOps) -> ForumResult<()> {
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
            let (changed, current_shas) = list_changed_since(git, &app.list_tip_shas)?;
            if changed {
                let threads = snapshot_list::list_threads(git)?;
                let sel = app.table_state.selected().unwrap_or(0);
                app.threads = threads;
                app.list_tip_shas = current_shas;
                let n = app.visible_threads().len();
                app.table_state
                    .select(if n > 0 { Some(sel.min(n - 1)) } else { None });
            }
        }
        _ => {}
    }
    Ok(())
}

// ============================================================
//  Slot 10c: snapshot-write helpers (TUI mutation cutover)
// ============================================================
//
// The CLI orchestration layer's run() helpers print to stdout and
// `std::process::exit` on failure, both of which collide with
// ratatui's terminal session. Slot 10c keeps the TUI shape and
// inlines the snapshot-tip mutations here. ADR-011 Decision 3:
// the TUI is a non-migrate consumer, so reads of legacy event
// chains bail with LegacyEventChain — the user runs
// `git forum migrate` from the CLI before reopening the TUI.

fn snapshot_update_node_status(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    target: NodeStatus,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let now = clock.now();
    let resolved = thread::resolve_node_id_in_thread(git, thread_id, node_id)?;
    if let Some(node) = doc.nodes.iter_mut().find(|n| n.record.id == resolved) {
        node.record.status = target;
        node.record.updated_at = Some(now);
        node.record.updated_by = Some(actor.to_string());
    } else {
        return Err(ForumError::Repo(format!(
            "node {resolved} not found in snapshot"
        )));
    }
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.to_string();
    write_snapshot(git, thread_id, &doc, "tui: node status update")?;
    Ok(())
}

fn snapshot_append_node(
    git: &GitOps,
    thread_id: &str,
    kind: NodeKind,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let now = clock.now();
    let id = id_alloc::alloc_bare_thread_id(actor, body, &now.to_rfc3339());
    doc.nodes.push(NodeWithBody {
        record: NodeRecord {
            id: id.clone(),
            kind,
            status: NodeStatus::Open,
            created_at: now,
            created_by: actor.to_string(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: body.to_string(),
    });
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.to_string();
    write_snapshot(git, thread_id, &doc, "tui: node add")?;
    Ok(id)
}

pub(super) fn snapshot_revise_body(
    git: &GitOps,
    thread_id: &str,
    new_body: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let now = clock.now();
    doc.body = Some(new_body.to_string());
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.to_string();
    write_snapshot(git, thread_id, &doc, "tui: revise body")?;
    Ok(())
}

fn snapshot_append_link(
    git: &GitOps,
    thread_id: &str,
    target_thread_id: &str,
    rel: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    let now = clock.now();
    doc.links.entries.push(Link {
        target: target_thread_id.to_string(),
        rel: rel.to_string(),
        created_at: now,
        created_by: actor.to_string(),
    });
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.to_string();
    write_snapshot(git, thread_id, &doc, "tui: link add")?;
    Ok(())
}

fn snapshot_create_thread(
    git: &GitOps,
    title: &str,
    body: Option<&str>,
    lifecycle_label: &str,
    tags: &[String],
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    use crate::internal::commands::thread_new::{
        augment_tags_for_lifecycle_label, lifecycle_label_to_category,
    };
    let now = clock.now();
    let mut tags = tags.to_vec();
    augment_tags_for_lifecycle_label(lifecycle_label, &mut tags);
    let category = lifecycle_label_to_category(lifecycle_label).to_string();
    let thread_id = id_alloc::alloc_bare_thread_id(actor, title, &now.to_rfc3339());
    let doc = snapshot::ThreadDocument {
        snapshot: ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: thread_id.clone(),
            title: title.to_string(),
            category,
            status: "draft".to_string(),
            tags,
            created_at: now,
            created_by: actor.to_string(),
            updated_at: now,
            updated_by: actor.to_string(),
            branch: None,
            supersedes: Vec::new(),
        },
        body: body.map(|b| b.to_string()),
        nodes: Vec::new(),
        links: Links {
            entries: Vec::new(),
        },
        evidence: EvidenceFile {
            entries: Vec::new(),
        },
    };
    write_snapshot(git, &thread_id, &doc, "tui: create thread")?;
    Ok(thread_id)
}
