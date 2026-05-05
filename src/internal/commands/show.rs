//! Renderers for `git forum show`, `node show`, `log`, `status`, `ls`,
//! `shortlog`, and search results. Per RFC-lmr3wfcm Track E, the thread-detail
//! renderers all collapse to a single [`render_show`] driven by [`ShowOptions`].
//!
//! Phase 2 slot 7c (RFC `7ymtc4b2`): the `Show` arm body relocates from
//! `main.rs` to [`run`] in this module. The `--tree` advisory's
//! [`collect_implements_children`] / [`fallback_scan_implements`]
//! helpers also move here — they are now exclusively a `show --tree`
//! concern, and per ADR-011 Decision 6 the SQLite reverse-link index
//! is on the Phase 4 DELETE list, so the tree-scan fallback is the
//! only path.

use super::super::error::ForumError;
use super::super::git_ops::GitOps;
use super::super::policy::{self, CategoryRegistry, Policy};
use super::super::refs::thread_ref;
use super::super::snapshot::history::{self, SnapshotLogEntry};
use super::super::thread::{self, NodeLookup, ThreadState};
use super::context::Context;
use super::shared::resolve_tid;

// ============================================================
//  Public surface — ShowOptions and the unified entry point
// ============================================================

/// Drives [`render_show`] and [`render_node_show`].
///
/// `mode` selects which block of the canonical thread view to emit. The
/// remaining fields are applied within whichever block is selected.
#[derive(Debug, Clone, Default)]
pub struct ShowOptions {
    /// Truncate node bodies and conversation details to single-line previews.
    pub compact: bool,
    /// Omit the timeline section entirely.
    pub no_timeline: bool,
    /// When set, the full view shows next-states with guard results and the
    /// state diagram; the WhatNext / ActionHint modes require it.
    pub policy: Option<Policy>,
    /// Which subset of the thread view to render.
    pub mode: ShowMode,
    /// Pre-loaded git-history view of the snapshot ref (SPEC-3.0 §5.4).
    /// `None` means the renderer skips the timeline section with a
    /// placeholder hint — callers that want the table populated must
    /// load entries via `snapshot::history::read_log` before rendering.
    /// (Phase 4 Step 1a, RFC `7ymtc4b2`: replaces `state.events`-driven
    /// timeline rendering. Subsequent step 1d wires the TUI per-thread
    /// timeline panel through the same surface.)
    pub timeline_entries: Option<Vec<SnapshotLogEntry>>,
}

/// Sub-views of the canonical thread renderer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShowMode {
    /// Default `git forum show` output.
    #[default]
    Full,
    /// `git forum status` — open items only.
    Status,
    /// `git forum show --what-next` — transitions + guard checks + op-checks.
    WhatNext,
    /// Post-state-change hint emitted to stderr (legacy `render_next_actions`).
    ActionHint,
}

/// Render `git forum show` output for a thread.
///
/// Output is deterministic given deterministic event timestamps and IDs.
/// Snapshot strategy: tests use fixed synthetic events; integration tests
/// avoid asserting exact Git OIDs.
pub fn render_show(state: &ThreadState, options: &ShowOptions) -> String {
    match options.mode {
        ShowMode::Full => render_full(state, options),
        ShowMode::Status => render_status_block(state),
        ShowMode::WhatNext => {
            let policy = options.policy.as_ref().expect("WhatNext requires policy");
            render_what_next_block(state, policy)
        }
        ShowMode::ActionHint => {
            let policy = options.policy.as_ref().expect("ActionHint requires policy");
            render_action_hint(state, policy)
        }
    }
}

/// Render `git forum node show` output for a single node.
///
/// `options.compact` truncates the body preview; `options.no_timeline` omits
/// the per-node history table.
pub fn render_node_show(lookup: &NodeLookup, options: &ShowOptions) -> String {
    let mut lines: Vec<String> = Vec::new();
    let node = &lookup.node;

    lines.push(format!(
        "## {} {}",
        short_oid(&node.node_id),
        node_display_label(node)
    ));
    lines.push(String::new());
    lines.push(format!(
        "**thread:**    {} {}",
        lookup.thread_id, lookup.thread_title
    ));
    // Phase 2b: lifecycle + tags, not kind.
    lines.push(format!("**lifecycle:** {}", lookup.thread_lifecycle));
    if !lookup.thread_tags.is_empty() {
        lines.push(format!("**tags:**      {}", lookup.thread_tags.join(", ")));
    }
    lines.push(format!("**status:**    {}", node_status(node)));
    lines.push(format!(
        "**created:**   {}",
        node.created_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(format!("**by:**        {}", node.actor));
    if let Some(ref parent_id) = node.reply_to {
        lines.push(format!("**reply-to:**  {}", short_oid(parent_id)));
    }
    lines.push(String::new());
    lines.push("---".into());
    lines.push(String::new());
    push_body_lines(&mut lines, &node.body);
    lines.push(String::new());

    if !lookup.links.is_empty() {
        lines.push("---".into());
        lines.push(String::new());
        lines.push(format!("### thread links ({})", lookup.links.len()));
        lines.push(String::new());
        for link in &lookup.links {
            lines.push(format!("- {}  {}", link.target_thread_id, link.rel));
        }
        lines.push(String::new());
    }

    if !options.no_timeline {
        lines.push("---".into());
        lines.push(String::new());
        lines.push("### history".into());
        lines.push(String::new());
        lines.extend(node_history_lines(&lookup.node.node_id, options));
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Thread-level snapshot history table — uses pre-loaded entries from
/// `ShowOptions::timeline_entries`. Falls back to a placeholder when
/// entries weren't loaded; callers that want the full table must
/// populate via `snapshot::history::read_log` first.
fn thread_history_lines(options: &ShowOptions) -> Vec<String> {
    let Some(entries) = &options.timeline_entries else {
        return vec!["_(timeline unavailable; no snapshot ref loaded)_".into()];
    };
    history::render_markdown(entries)
}

/// Per-node history slice: filter the pre-loaded snapshot log to
/// commits whose tree changed `nodes/<id>.{toml,md}`. Falls back to a
/// placeholder when entries weren't loaded.
fn node_history_lines(node_id: &str, options: &ShowOptions) -> Vec<String> {
    let Some(entries) = &options.timeline_entries else {
        return vec!["_(history unavailable; no snapshot ref loaded)_".into()];
    };
    let toml_path = format!("nodes/{node_id}.toml");
    let md_path = format!("nodes/{node_id}.md");
    let touching = history::entries_touching(entries, &[&toml_path, &md_path]);
    if touching.is_empty() {
        return vec!["_(no snapshot commits touched this node)_".into()];
    }
    history::render_markdown_refs(&touching)
}

// ============================================================
//  Mode: Full
// ============================================================

fn render_full(state: &ThreadState, options: &ShowOptions) -> String {
    let compact = options.compact;
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("## {} {}", state.id, state.title));
    lines.push(String::new());
    // Phase 2b (Finding 1): lifecycle + tags are the canonical 2.0
    // classification axis. The legacy `kind` field stays in storage for
    // backward compatibility (per ADR-002) but is no longer surfaced as
    // a primary display label.
    lines.push(format!("**lifecycle:** {}", state.lifecycle));
    if !state.tags.is_empty() {
        lines.push(format!("**tags:**      {}", state.tags.join(", ")));
    }
    lines.push(format!("**status:**    {}", state.status));
    if let Some(policy) = &options.policy {
        if let Some(line) = next_states_line(state, policy) {
            lines.push(line);
        }
        if !compact {
            lines.push("transitions:".into());
            let category = policy::lifecycle_to_category(state.lifecycle);
            for diagram_line in render_state_diagram(category, state.status.as_str()) {
                lines.push(diagram_line);
            }
        }
    }
    lines.push(format!(
        "**created:**   {}",
        state.created_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(format!("**by:**        {}", state.created_by));
    if let Some(branch) = &state.branch {
        lines.push(format!("**branch:**    {}", branch));
    }
    if let Some(body) = &state.body {
        lines.push(String::new());
        lines.push("---".into());
        lines.push(String::new());
        push_thread_body(&mut lines, state, body, compact);
    }
    lines.push(String::new());

    if !compact && state.body_revision_count > 0 {
        lines.push(format!("**body revisions:** {}", state.body_revision_count));
    }

    if !compact {
        let incorporated: Vec<&super::super::node::Node> =
            state.nodes.iter().filter(|n| n.incorporated).collect();
        render_item_list(&mut lines, "incorporated nodes", &incorporated, |node| {
            format!(
                "  - {} {} {}",
                short_oid(&node.node_id),
                node_display_label(node),
                body_or_truncated(&node.body, 60, false)
            )
        });
    }

    push_open_items(
        &mut lines,
        &state.id,
        "open objections",
        &state.open_objections(),
        compact,
        true,
    );
    push_open_items(
        &mut lines,
        &state.id,
        "open actions",
        &state.open_actions(),
        compact,
        false,
    );

    if let Some(summary) = state.latest_summary() {
        lines.push("**latest summary:**".into());
        lines.push(format!("  {}", summary.body));
        lines.push(String::new());
    }

    if !compact {
        render_item_list(&mut lines, "evidence", &state.evidence_items, |ev| {
            let id_short = &ev.evidence_id[..ev.evidence_id.len().min(8)];
            format!("  - {}  {}  {}", id_short, ev.kind, ev.ref_target)
        });
    }

    render_item_list(&mut lines, "links", &state.links, |link| {
        format!("  - {}  {}", link.target_thread_id, link.rel)
    });

    push_conversations(&mut lines, state, compact);

    if !options.no_timeline {
        lines.push("---".into());
        lines.push(String::new());
        if compact {
            let count = options
                .timeline_entries
                .as_ref()
                .map(|e| e.len())
                .unwrap_or(0);
            lines.push(format!(
                "**timeline:** {} commits (use 'log {}' for full view)",
                count, state.id
            ));
        } else {
            lines.push("### timeline".into());
            lines.push(String::new());
            lines.extend(thread_history_lines(options));
        }
        lines.push(String::new());
    }

    if !compact && (!state.open_objections().is_empty() || !state.open_actions().is_empty()) {
        lines.push(format!(
            "tip: run `git forum show {} --what-next` for action guidance",
            state.id
        ));
        lines.push(String::new());
    }

    lines.join("\n")
}

fn push_thread_body(lines: &mut Vec<String>, state: &ThreadState, body: &str, compact: bool) {
    if !compact {
        push_body_lines(lines, body);
        return;
    }
    let body_lines: Vec<&str> = body
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .collect();
    let shown = body_lines.len().min(5);
    for line in &body_lines[..shown] {
        lines.push((*line).to_string());
    }
    if body_lines.len() > 5 {
        let rev_hint = if state.body_revision_count > 0 {
            format!(", {} revision(s)", state.body_revision_count)
        } else {
            String::new()
        };
        lines.push(format!(
            "... ({}{rev_hint} — use 'show {}' for full body)",
            size_summary(body.len()),
            state.id
        ));
    }
}

fn push_body_lines(lines: &mut Vec<String>, body: &str) {
    for line in body.lines() {
        lines.push(line.to_string());
    }
    if body.is_empty() {
        lines.push(String::new());
    }
}

fn push_open_items(
    lines: &mut Vec<String>,
    thread_id: &str,
    label: &str,
    items: &[&super::super::node::Node],
    compact: bool,
    with_reply: bool,
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("**{label}:** {}", items.len()));
    for node in items {
        let nid = short_oid(&node.node_id);
        lines.push(format!(
            "  - {nid} {}",
            body_or_truncated(&node.body, 60, compact)
        ));
        lines.push(format!("    resolve: git forum resolve {thread_id} {nid}"));
        if with_reply {
            lines.push(format!(
                "    reply:   git forum claim {thread_id} --reply-to {nid} --body \"...\""
            ));
        }
    }
    lines.push(String::new());
}

fn push_conversations(lines: &mut Vec<String>, state: &ThreadState, compact: bool) {
    let conversations = build_conversations(&state.nodes);
    if conversations.is_empty() {
        return;
    }
    if compact {
        let inc_count = state.nodes.iter().filter(|n| n.incorporated).count();
        let inc_hint = if inc_count > 0 {
            format!(" ({inc_count} incorporated)")
        } else {
            String::new()
        };
        lines.push(format!(
            "conversations: {}{}",
            conversations.len(),
            inc_hint
        ));
        for convo in &conversations {
            let root = convo[0];
            lines.push(format!(
                "  {} {} [{}] {} — {}",
                short_oid(&root.node_id),
                node_display_label(root),
                node_status(root),
                root.actor,
                single_line_preview(&root.body, 60)
            ));
        }
    } else {
        lines.push(format!("conversations: {}", conversations.len()));
        for convo in &conversations {
            let root = convo[0];
            lines.push(format!(
                "  {} {} [{}] {}",
                short_oid(&root.node_id),
                node_display_label(root),
                node_status(root),
                body_or_truncated(&root.body, 50, false)
            ));
            render_indented_body(lines, &root.body, "      ");
            for reply in &convo[1..] {
                lines.push(format!(
                    "    -> {} {} {} {}",
                    short_oid(&reply.node_id),
                    node_display_label(reply),
                    reply.actor,
                    body_or_truncated(&reply.body, 50, false)
                ));
                render_indented_body(lines, &reply.body, "        ");
            }
            let last = convo.last().unwrap();
            if last.is_open() {
                lines.push(format!(
                    "    reply: git forum claim {} --reply-to {} --body \"...\"",
                    state.id,
                    short_oid(&last.node_id)
                ));
            }
        }
    }
    lines.push(String::new());
}

/// SPEC-3.0 §3.1: derive the per-thread category-driven transition list.
/// `ThreadState` carries the v2 `lifecycle` facet only; `category_for_state`
/// folds it into one of the built-in category names so policy overrides on
/// `rfc`/`task` take effect on the displayed next-states.
fn next_targets_for_state(state: &ThreadState, policy: &Policy) -> Vec<String> {
    let category = policy::category_for_state(state);
    policy
        .effective_registry()
        .get(category)
        .map(|d| d.valid_targets(state.status.as_str()))
        .unwrap_or_default()
}

/// Build the `**next:**` line shown under the status field. Returns `None` if
/// the current state has no outgoing transitions.
fn next_states_line(state: &ThreadState, policy: &Policy) -> Option<String> {
    let targets = next_targets_for_state(state, policy);
    if targets.is_empty() {
        return None;
    }
    let parts: Vec<String> = targets
        .iter()
        .map(|target| format_target_with_blockers(state, policy, target, /*verbose=*/ false))
        .collect();
    Some(format!("**next:**     {}", parts.join(", ")))
}

fn format_target_with_blockers(
    state: &ThreadState,
    policy: &Policy,
    target: &str,
    verbose: bool,
) -> String {
    let violations = policy::check_guards(policy, state, state.status.as_str(), target);
    if violations.is_empty() {
        target.to_string()
    } else {
        let blockers: Vec<String> = if verbose {
            violations
                .iter()
                .map(|v| format!("{}: {}", v.rule, v.reason))
                .collect()
        } else {
            violations.iter().map(|v| v.rule.clone()).collect()
        };
        let sep = if verbose { "; " } else { ", " };
        format!("{target} (blocked: {})", blockers.join(sep))
    }
}

// ============================================================
//  Mode: Status
// ============================================================

fn render_status_block(state: &ThreadState) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<12} {} ({})",
        state.id, state.title, state.status
    ));

    let open_obj = state.open_objections();
    let open_act = state.open_actions();
    let open_questions: Vec<&super::super::node::Node> = state
        .nodes
        .iter()
        .filter(|n| n.legacy_subtype.as_deref() == Some("question") && n.is_open())
        .collect();

    if open_obj.is_empty() && open_act.is_empty() && open_questions.is_empty() {
        lines.push("  no open items".into());
    } else {
        push_status_group(&mut lines, "objections", &open_obj);
        push_status_group(&mut lines, "actions", &open_act);
        push_status_group(&mut lines, "questions", &open_questions);
    }
    lines.push(String::new());
    lines.join("\n")
}

fn push_status_group(lines: &mut Vec<String>, label: &str, items: &[&super::super::node::Node]) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("  {label} ({})", items.len()));
    for node in items {
        lines.push(format!(
            "    - {} {}",
            short_oid(&node.node_id),
            truncate_body(&node.body, 60)
        ));
    }
}

// ============================================================
//  Mode: WhatNext
// ============================================================

fn render_what_next_block(state: &ThreadState, policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{} ({})", state.id, state.status));
    lines.push(String::new());

    let targets = next_targets_for_state(state, policy);
    if targets.is_empty() {
        lines.push("valid transitions: (none)".to_string());
    } else {
        lines.push(format!("valid transitions: {}", targets.join(", ")));
    }
    lines.push(String::new());

    for target in &targets {
        let violations = policy::check_guards(policy, state, state.status.as_str(), target);
        if violations.is_empty() {
            lines.push(format!("guard check ({} -> {target}): PASS", state.status));
        } else {
            lines.push(format!("guard check ({} -> {target}):", state.status));
            for v in &violations {
                lines.push(format!("  [FAIL] {} -- {}", v.rule, v.reason));
            }
        }
    }
    if !targets.is_empty() {
        lines.push(String::new());
    }

    let lookahead = super::verify::build_lookahead(state, policy);
    for entry in &lookahead {
        lines.push(format!("lookahead ({}):", entry.path));
        for v in &entry.violations {
            lines.push(format!("  [{}] {}", v.rule, v.reason));
        }
        lines.push(String::new());
    }

    lines.push(format!(
        "open objections: {}",
        state.open_objections().len()
    ));
    lines.push(format!("open actions:    {}", state.open_actions().len()));
    lines.push(format!("nodes:           {}", state.nodes.len()));
    lines.push(format!("evidence:        {}", state.evidence_items.len()));
    lines.push(format!("links:           {}", state.links.len()));
    lines.push(format!(
        "has summary:     {}",
        if state.latest_summary().is_some() {
            "yes"
        } else {
            "no"
        }
    ));
    lines.push(String::new());

    let op_lines = op_check_lines(state, policy);
    if !op_lines.is_empty() {
        lines.push(format!("operation checks (state: {}):", state.status));
        lines.extend(op_lines);
        lines.push(String::new());
    }

    lines.join("\n")
}

fn op_check_lines(state: &ThreadState, policy: &Policy) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let category = crate::internal::policy::category_for_state(state);
    let status = state.status.as_str();

    if let Some(allowed) = policy.allowed_node_types(category, status) {
        if allowed.is_empty() {
            out.push("  node types: (none allowed)".into());
        } else {
            let types: Vec<String> = allowed
                .iter()
                .map(|n| match n {
                    crate::internal::node::NodeKind::Comment => "comment".to_string(),
                    crate::internal::node::NodeKind::Approval => "approval".to_string(),
                    crate::internal::node::NodeKind::Objection => "objection".to_string(),
                    crate::internal::node::NodeKind::Action => "action".to_string(),
                })
                .collect();
            out.push(format!("  node types: {}", types.join(", ")));
        }
    }

    if let Some(revise) = policy.revise_rules_for(category) {
        if let Some(list) = &revise.allow_body_revise {
            let allowed = list.iter().any(|s| s.as_str() == status);
            out.push(format!(
                "  body revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
        if let Some(list) = &revise.allow_node_revise {
            let allowed = list.iter().any(|s| s.as_str() == status);
            out.push(format!(
                "  node revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
    }

    if let Some(evidence) = policy.evidence_rules_for(category) {
        if let Some(list) = &evidence.allow_evidence {
            let allowed = list.iter().any(|s| s.as_str() == status);
            out.push(format!(
                "  evidence:    {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
    }
    out
}

// ============================================================
//  Mode: ActionHint (post-state-change)
// ============================================================

fn render_action_hint(state: &ThreadState, policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    let targets = next_targets_for_state(state, policy);
    if targets.is_empty() {
        lines.push("  next: (no transitions available)".to_string());
    } else {
        let parts: Vec<String> = targets
            .iter()
            .map(|t| format_target_with_blockers(state, policy, t, /*verbose=*/ true))
            .collect();
        lines.push(format!("  next: {}", parts.join(", ")));
    }

    let open_obj = state.open_objections();
    let open_act = state.open_actions();
    if !open_obj.is_empty() || !open_act.is_empty() {
        let mut items = Vec::new();
        if !open_obj.is_empty() {
            items.push(format!("{} open objection(s)", open_obj.len()));
        }
        if !open_act.is_empty() {
            items.push(format!("{} open action(s)", open_act.len()));
        }
        lines.push(format!("  open: {}", items.join(", ")));
        for node in &open_obj {
            lines.push(format!(
                "    objection {} — resolve with: resolve {} {}",
                short_oid(&node.node_id),
                state.id,
                short_oid(&node.node_id)
            ));
        }
        for node in &open_act {
            lines.push(format!(
                "    action {} — resolve with: resolve {} {}",
                short_oid(&node.node_id),
                state.id,
                short_oid(&node.node_id)
            ));
        }
    }
    lines.join("\n")
}

// ============================================================
//  State diagram — table-driven from CategoryDefinition
// ============================================================

/// SPEC-3.0 §3.1: per-category "happy-path" spine for the built-in
/// categories. Every other reachable status is rendered as a branch off
/// whichever spine status it leaves from.
///
/// Custom categories (no entry here) render as a flat list of edges
/// without a spotlighted spine.
fn category_spine(category: &str) -> &'static [&'static str] {
    match category {
        "rfc" => &["draft", "open", "review", "done", "deprecated"],
        "task" => &["open", "working", "review", "done", "deprecated"],
        _ => &[],
    }
}

/// Render a compact Unicode state diagram for `category` with `current`
/// highlighted in `[brackets]`. Forward edges use `→`; branch edges use
/// `├→` / `└→`. Driven entirely by the SPEC-3.0 §3.1
/// [`CategoryDefinition::transitions`] of the built-in registry.
///
/// Replaces the legacy lifecycle-keyed renderer (which read
/// `WorkflowSpec::unified_transitions()` filtered by
/// `Lifecycle::allows_state`) per RFC `7ymtc4b2` v3.1 follow-up task
/// `1v400j3l` step 3c.
pub fn render_state_diagram(category: &str, current: &str) -> Vec<String> {
    let registry = CategoryRegistry::built_in();
    let cat_def = match registry.get(category) {
        Some(d) => d,
        None => return vec![format!("  (unknown category `{category}`)")],
    };
    let edges: Vec<(&str, &str)> = cat_def
        .transitions
        .iter()
        .filter_map(|t| t.split_once("->"))
        .collect();
    let spine = category_spine(category);
    let fmt_state = |s: &str| -> String {
        if s == current {
            format!("[{s}]")
        } else {
            s.to_string()
        }
    };

    let mut lines: Vec<String> = Vec::new();
    let spine_parts: Vec<String> = spine.iter().map(|s| fmt_state(s)).collect();
    lines.push(format!("  {}", spine_parts.join(" → ")));

    for &src in spine {
        let branches: Vec<&str> = edges
            .iter()
            .filter_map(|&(s, d)| (s == src && !is_spine_edge(spine, s, d)).then_some(d))
            .collect();
        if branches.is_empty() {
            continue;
        }
        let indent = spine_indent(spine, src, current);
        for (i, target) in branches.iter().enumerate() {
            let connector = if i + 1 < branches.len() {
                "├→"
            } else {
                "└→"
            };
            lines.push(format!("{indent}{connector} {}", fmt_state(target)));
        }
    }

    // Branches whose source is not on the spine (e.g. rejected → open).
    for &(src, dst) in &edges {
        if spine.contains(&src) {
            continue;
        }
        lines.push(format!("  {} → {}", fmt_state(src), fmt_state(dst)));
    }

    lines
}

/// Display label for a node: prefer the rhetorical `legacy_subtype`
/// (e.g. `summary`, `question`, `claim`) when set, else the canonical
/// SPEC-3.0 NodeKind name. Migrated v1 nodes carry their original
/// rhetorical label here even though the persisted `node_type`
/// collapses to one of the four canonical kinds.
fn node_display_label(node: &super::super::node::Node) -> String {
    node.legacy_subtype
        .clone()
        .unwrap_or_else(|| node.node_type.to_string())
}

fn is_spine_edge(spine: &[&str], src: &str, dst: &str) -> bool {
    spine.windows(2).any(|w| w[0] == src && w[1] == dst)
}

fn spine_indent(spine: &[&str], src: &str, current: &str) -> String {
    let mut offset = 2;
    for &s in spine {
        if s == src {
            break;
        }
        let w = if s == current { s.len() + 2 } else { s.len() };
        offset += w + 3; // " → "
    }
    " ".repeat(offset)
}

// ============================================================
//  Timeline — single markdown-table renderer
// ============================================================

// ============================================================
//  Conversations / shared body helpers
// ============================================================

/// A conversation is a root node (no in-thread parent) plus all transitive
/// replies, in chronological order. Only nodes participating in a reply
/// chain appear; standalone nodes are shown elsewhere.
fn build_conversations(nodes: &[super::super::node::Node]) -> Vec<Vec<&super::super::node::Node>> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let node_ids: HashSet<&str> = nodes.iter().map(|n| n.node_id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&super::super::node::Node>> = HashMap::new();
    let mut has_parent: HashSet<&str> = HashSet::new();
    for node in nodes {
        if let Some(ref parent_id) = node.reply_to {
            if node_ids.contains(parent_id.as_str()) {
                children.entry(parent_id.as_str()).or_default().push(node);
                has_parent.insert(node.node_id.as_str());
            }
        }
    }

    let mut conversations = Vec::new();
    for node in nodes {
        if has_parent.contains(node.node_id.as_str()) {
            continue;
        }
        // SPEC-3.0 transitional (RFC `7ymtc4b2`): in the v2 event-chain
        // model the timeline carried Say bodies inline, so a leaf
        // comment with no replies could be elided from the
        // conversations section without losing visibility. Snapshot
        // threads have no event timeline, so leaf nodes must surface
        // here too. The pre-Phase-2 leaf-only filter is removed.
        let mut chain = vec![node];
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(node.node_id.as_str());
        while let Some(parent_id) = queue.pop_front() {
            if let Some(replies) = children.get(parent_id) {
                for reply in replies {
                    chain.push(reply);
                    queue.push_back(reply.node_id.as_str());
                }
            }
        }
        conversations.push(chain);
    }
    conversations
}

fn render_item_list<T, F>(lines: &mut Vec<String>, label: &str, items: &[T], formatter: F)
where
    F: Fn(&T) -> String,
{
    if items.is_empty() {
        return;
    }
    lines.push(format!("{}: {}", label, items.len()));
    for item in items {
        lines.push(formatter(item));
    }
    lines.push(String::new());
}

fn body_or_truncated(s: &str, max: usize, compact: bool) -> String {
    if compact {
        truncate_body(s, max)
    } else {
        s.lines().next().unwrap_or("").to_string()
    }
}

fn render_indented_body(lines: &mut Vec<String>, body: &str, indent: &str) {
    let body_lines: Vec<&str> = body.lines().collect();
    if body_lines.len() > 1 {
        for line in &body_lines[1..] {
            lines.push(format!("{indent}{line}"));
        }
    }
}

fn truncate_body(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn single_line_preview(s: &str, max: usize) -> String {
    let joined = s.lines().collect::<Vec<_>>().join(" / ");
    truncate_body(&joined, max)
}

fn size_summary(size: usize) -> String {
    if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

fn node_status(node: &super::super::node::Node) -> &'static str {
    if node.retracted {
        "retracted"
    } else if node.incorporated {
        "incorporated"
    } else if node.resolved {
        "resolved"
    } else {
        "open"
    }
}

/// Truncate an OID to 16 characters for display.
pub fn short_oid(id: &str) -> &str {
    &id[..id.len().min(16)]
}

/// One row in the `show --tree` advisory output: a thread that links to
/// the named thread with `--rel implements`.
///
/// Held as a value-type so the renderer is decoupled from how the rows are
/// resolved (SQLite reverse index → ref tip read), which keeps the renderer
/// trivially testable.
#[derive(Debug, Clone)]
pub struct TreeChild {
    pub id: String,
    pub title: String,
    pub lifecycle_label: String,
    pub status: String,
}

/// Render the `show --tree` advisory: the named thread plus its direct
/// incoming `--rel implements` children, one hop, no recursion.
///
/// Per SPEC-2.0 §B.4 and CORE-VALUE.md "Advisories", this is informational
/// only — it does not gate any operation. Callers must pass a `children`
/// list that is already filtered to `rel == "implements"` (see
/// `index::find_incoming_links`).
pub fn render_tree(parent: &ThreadState, children: &[TreeChild]) -> String {
    let parent_lifecycle = parent.lifecycle.as_str();
    let mut lines = Vec::new();
    lines.push(format!(
        "{}  {}/{}    {}",
        parent.id, parent_lifecycle, parent.status, parent.title
    ));

    if children.is_empty() {
        lines.push("  (no incoming `implements` links)".into());
    } else {
        let last = children.len() - 1;
        for (i, child) in children.iter().enumerate() {
            let connector = if i == last { "└──" } else { "├──" };
            lines.push(format!(
                "{} {}  {}/{}   {}",
                connector, child.id, child.lifecycle_label, child.status, child.title
            ));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

// ============================================================
//  Show command — `git forum show` orchestration
// ============================================================

/// Args for [`run`] — `git forum show` flags.
pub struct ShowArgs {
    pub thread_id: String,
    pub what_next: bool,
    pub compact: bool,
    pub no_timeline: bool,
    pub tree: bool,
}

/// Uniform entry point for the `show` subcommand.
///
/// `--tree`: prints the parent + direct `implements` children (no recursion).
/// `--what-next`: switches the renderer to the WhatNext mode with policy
/// guards; otherwise renders the canonical Full view.
pub fn run(args: ShowArgs, ctx: &Context) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, &args.thread_id)?;
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    let state = thread::replay_thread(&ctx.git, &thread_id)?;
    if args.tree {
        let children = collect_implements_children(&ctx.git, &thread_id)?;
        print!("{}", render_tree(&state, &children));
    } else {
        let mode = if args.what_next {
            ShowMode::WhatNext
        } else {
            ShowMode::Full
        };
        let timeline_entries = if args.no_timeline {
            None
        } else {
            // SPEC-3.0 §5.4: thread history is the snapshot ref's git log.
            history::read_log(&ctx.git, &thread_ref(&thread_id)).ok()
        };
        print!(
            "{}",
            render_show(
                &state,
                &ShowOptions {
                    compact: args.compact,
                    no_timeline: args.no_timeline,
                    policy: Some(policy),
                    mode,
                    timeline_entries,
                }
            )
        );
    }
    Ok(())
}

/// Collect direct incoming `--rel implements` children of a thread for
/// the `show --tree` advisory.
///
/// Phase 2 slot 7c (RFC `7ymtc4b2`): relocated from `main.rs`. Per ADR-011
/// Decision 6 the SQLite reverse-link index is on the Phase 4 DELETE list,
/// so this always falls back to the O(N-thread-refs) tree scan.
pub fn collect_implements_children(
    git: &GitOps,
    parent_thread_id: &str,
) -> Result<Vec<TreeChild>, ForumError> {
    let child_ids = fallback_scan_implements(git, parent_thread_id)?;
    let mut out = Vec::with_capacity(child_ids.len());
    for id in child_ids {
        match thread::replay_thread(git, &id) {
            Ok(child_state) => out.push(TreeChild {
                id: child_state.id.clone(),
                title: child_state.title.clone(),
                lifecycle_label: child_state.lifecycle.as_str().to_string(),
                status: child_state.status.to_string(),
            }),
            Err(_) => continue,
        }
    }
    Ok(out)
}

/// Fallback for `show --tree`: list all thread refs and replay each to
/// find the ones whose forward links include `(parent_thread_id, implements)`.
/// O(N) on thread count.
fn fallback_scan_implements(
    git: &GitOps,
    parent_thread_id: &str,
) -> Result<Vec<String>, ForumError> {
    let ids = thread::list_thread_ids(git)?;
    let mut out = Vec::new();
    for id in ids {
        if id == parent_thread_id {
            continue;
        }
        let Ok(state) = thread::replay_thread(git, &id) else {
            continue;
        };
        if state
            .links
            .iter()
            .any(|l| l.target_thread_id == parent_thread_id && l.rel == "implements")
        {
            out.push(state.id);
        }
    }
    out.sort();
    Ok(out)
}

// User-visible output is locked in by the golden snapshots in
// `tests/snapshot_test.rs`. The unit tests below cover pure-function
// invariants that snapshots can't reach (state-graph filtering, multibyte
// truncation, what-next op-check projection).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::node::{Node, NodeKind};
    use crate::internal::policy::Lifecycle;
    use crate::internal::thread::{ThreadKind, ThreadState, ThreadStatus};
    use chrono::TimeZone;

    fn fixed_state() -> ThreadState {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: "RFC-0001".into(),
            kind: ThreadKind::Rfc,
            lifecycle: Lifecycle::Proposal,
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            status: ThreadStatus::Draft,
            created_at: t,
            created_by: "human/alice".into(),
            ..ThreadState::default()
        }
    }

    #[test]
    fn single_line_preview_handles_multibyte_text() {
        let preview =
            single_line_preview("実装開始: CMake + ImGui + GLFW スケルトンアプリの構築", 20);
        assert!(preview.starts_with("実装開始"));
        assert!(preview.ends_with("..."));
    }

    // Phase 4 Step 1a: the v2 timeline body-preview / revise-body
    // size-summary tests (`timeline_summarizes_long_bodies_to_one_line`,
    // `revise_body_timeline_shows_size_summary`) used to live here.
    // They exercised `state.events` flowing through the legacy
    // `internal::timeline::render_markdown` path, which no longer
    // exists — the canonical 3.0 timeline reads commit subjects per
    // SPEC-3.0 §5.4 instead. Equivalent coverage for the new rendering
    // lives in `internal::snapshot::history::tests`.

    #[test]
    fn show_with_policy_includes_next_and_diagram() {
        let state = fixed_state();
        let out = render_show(
            &state,
            &ShowOptions {
                policy: Some(crate::internal::policy::Policy::default()),
                ..ShowOptions::default()
            },
        );
        // Proposal lifecycle from `draft`: only draft→open and draft→withdrawn.
        assert!(out.contains("**next:**     open, withdrawn"));
        assert!(out.contains("transitions:"));
        assert!(out.contains("[draft]"));
    }

    #[test]
    fn state_diagram_filters_by_category() {
        // rfc: withdrawn is in the registry (draft→withdrawn,
        // open→withdrawn).
        assert!(render_state_diagram("rfc", "open")
            .join("\n")
            .contains("withdrawn"));
        // task: no withdrawn status in the registry.
        let task = render_state_diagram("task", "open").join("\n");
        assert!(!task.contains("withdrawn"));
        assert!(task.contains("working") && task.contains("review"));
        assert!(task.contains("[open]") && task.contains("done"));
    }

    #[test]
    fn what_next_mode_includes_operation_checks() {
        use crate::internal::node::NodeKind;
        use crate::internal::policy::{CategoryPolicy, EvidenceRules, Policy, ReviseRules};
        let state = fixed_state();
        let mut rfc = CategoryPolicy::default();
        rfc.allowed_node_types
            .insert("draft".into(), vec![NodeKind::Comment, NodeKind::Objection]);
        rfc.revise = Some(ReviseRules {
            allow_body_revise: Some(vec!["draft".into()]),
            allow_node_revise: None,
        });
        rfc.evidence = Some(EvidenceRules {
            allow_evidence: Some(vec!["draft".into(), "open".into()]),
        });
        let mut policy = Policy::default();
        policy.categories.insert("rfc".into(), rfc);
        let out = render_show(
            &state,
            &ShowOptions {
                mode: ShowMode::WhatNext,
                policy: Some(policy),
                ..ShowOptions::default()
            },
        );
        assert!(out.contains("operation checks (state: draft):"));
        assert!(out.contains("node types: comment, objection"));
        assert!(out.contains("body revise: allowed"));
        assert!(out.contains("evidence:    allowed"));
    }

    #[test]
    fn status_mode_lists_open_items() {
        let mut state = fixed_state();
        state.nodes.push(Node {
            node_id: "node-0001".into(),
            node_type: NodeKind::Objection,
            body: "Bench results missing".into(),
            actor: "ai/reviewer".into(),
            created_at: state.created_at,
            ..Node::default()
        });
        let out = render_show(
            &state,
            &ShowOptions {
                mode: ShowMode::Status,
                ..ShowOptions::default()
            },
        );
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("objections (1)"));
        assert!(out.contains("Bench results missing"));
    }

    #[test]
    fn render_tree_lists_implements_children() {
        let parent = ThreadState {
            id: "RFC-PARENT".into(),
            kind: ThreadKind::Rfc,
            title: "Parent RFC".into(),
            status: ThreadStatus::Done,
            ..Default::default()
        };
        let children = vec![
            TreeChild {
                id: "TASK-A".into(),
                title: "Add FTS5 schema".into(),
                lifecycle_label: "execution".into(),
                status: "open".into(),
            },
            TreeChild {
                id: "TASK-B".into(),
                title: "Switch search to FTS5".into(),
                lifecycle_label: "execution".into(),
                status: "open".into(),
            },
        ];
        let out = render_tree(&parent, &children);
        assert!(out.contains("RFC-PARENT"));
        assert!(out.contains("Parent RFC"));
        assert!(out.contains("├── TASK-A"));
        assert!(out.contains("└── TASK-B"));
        assert!(out.contains("execution/open"));
    }

    #[test]
    fn render_tree_no_children_shows_marker() {
        let parent = ThreadState {
            id: "RFC-LONELY".into(),
            kind: ThreadKind::Rfc,
            title: "Lonely RFC".into(),
            status: ThreadStatus::Draft,
            ..Default::default()
        };
        let out = render_tree(&parent, &[]);
        assert!(out.contains("RFC-LONELY"));
        assert!(out.contains("(no incoming `implements` links)"));
    }
}
