//! Renderers for `git forum show`, `node show`, `log`, `status`, `ls`,
//! `shortlog`, and search results. Per RFC-lmr3wfcm Track E, the thread-detail
//! renderers all collapse to a single [`render_show`] driven by [`ShowOptions`].

use super::event::{self, Lifecycle, UNIFIED_TRANSITIONS};
use super::policy::{self, Policy};
use super::thread::{NodeLookup, ThreadState};
use super::timeline;

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
        node.node_type
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
        lines.extend(timeline::render_markdown(&lookup.events));
        lines.push(String::new());
    }

    lines.join("\n")
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
            for diagram_line in render_state_diagram(state.lifecycle, state.status.as_str()) {
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
        let incorporated: Vec<&super::node::Node> =
            state.nodes.iter().filter(|n| n.incorporated).collect();
        render_item_list(&mut lines, "incorporated nodes", &incorporated, |node| {
            format!(
                "  - {} {} {}",
                short_oid(&node.node_id),
                node.node_type,
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
            lines.push(format!(
                "**timeline:** {} events (use 'log {}' for full view)",
                state.events.len(),
                state.id
            ));
        } else {
            lines.push("### timeline".into());
            lines.push(String::new());
            lines.extend(timeline::render_markdown(&state.events));
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
    items: &[&super::node::Node],
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
                root.node_type,
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
                root.node_type,
                node_status(root),
                body_or_truncated(&root.body, 50, false)
            ));
            render_indented_body(lines, &root.body, "      ");
            for reply in &convo[1..] {
                lines.push(format!(
                    "    -> {} {} {} {}",
                    short_oid(&reply.node_id),
                    reply.node_type,
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

/// Build the `**next:**` line shown under the status field. Returns `None` if
/// the current state has no outgoing transitions.
fn next_states_line(state: &ThreadState, policy: &Policy) -> Option<String> {
    let targets = event::valid_targets(state.lifecycle, state.status.as_str());
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
    let open_questions: Vec<&super::node::Node> = state
        .nodes
        .iter()
        .filter(|n| n.node_type == super::event::NodeType::Question && n.is_open())
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

fn push_status_group(lines: &mut Vec<String>, label: &str, items: &[&super::node::Node]) {
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

    let targets = event::valid_targets(state.lifecycle, state.status.as_str());
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

    let lookahead =
        super::verify::build_lookahead(state.kind, state.status.as_str(), state, policy);
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
    if !policy.node_rules.is_empty() {
        if let Some(allowed) = policy.node_rules.get(state.status.as_str()) {
            if allowed.is_empty() {
                out.push("  node types: (none allowed)".into());
            } else {
                let types: Vec<String> = allowed.iter().map(|n| n.to_string()).collect();
                out.push(format!("  node types: {}", types.join(", ")));
            }
        } else {
            out.push("  node types: (all allowed)".into());
        }
    }
    if let Some(revise) = &policy.revise_rules {
        if !revise.allow_body_revise.is_empty() {
            let allowed = revise
                .allow_body_revise
                .iter()
                .any(|s| s.as_str() == state.status.as_str());
            out.push(format!(
                "  body revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
        if !revise.allow_node_revise.is_empty() {
            let allowed = revise
                .allow_node_revise
                .iter()
                .any(|s| s.as_str() == state.status.as_str());
            out.push(format!(
                "  node revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
    }
    if let Some(evidence) = &policy.evidence_rules {
        if !evidence.allow_evidence.is_empty() {
            let allowed = evidence
                .allow_evidence
                .iter()
                .any(|s| s.as_str() == state.status.as_str());
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

    let targets = event::valid_targets(state.lifecycle, state.status.as_str());
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
//  State diagram — table-driven from Lifecycle
// ============================================================

/// SPEC-2.0 §3.1.1: per-lifecycle "happy-path" spine. Every other allowed
/// state is rendered as a branch off whichever spine state it leaves from.
fn lifecycle_spine(lifecycle: Lifecycle) -> &'static [&'static str] {
    match lifecycle {
        Lifecycle::Proposal => &["draft", "open", "review", "done", "deprecated"],
        Lifecycle::Execution => &["open", "working", "review", "done", "deprecated"],
        Lifecycle::Record => &["open", "done", "deprecated"],
    }
}

/// Render a compact Unicode state diagram for `lifecycle` with `current`
/// highlighted in `[brackets]`. Forward edges use `→`; branch edges use
/// `├→` / `└→`. Driven entirely by [`UNIFIED_TRANSITIONS`] filtered by the
/// lifecycle's allowed states (§3.1).
pub fn render_state_diagram(lifecycle: Lifecycle, current: &str) -> Vec<String> {
    let spine = lifecycle_spine(lifecycle);
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
        let branches: Vec<&str> = UNIFIED_TRANSITIONS
            .iter()
            .filter_map(|&(s, d)| {
                (s == src && lifecycle.allows_state(d) && !is_spine_edge(spine, s, d)).then_some(d)
            })
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
    for &(src, dst) in UNIFIED_TRANSITIONS {
        if !lifecycle.allows_state(src) || !lifecycle.allows_state(dst) {
            continue;
        }
        if spine.contains(&src) {
            continue;
        }
        lines.push(format!("  {} → {}", fmt_state(src), fmt_state(dst)));
    }

    lines
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
fn build_conversations(nodes: &[super::node::Node]) -> Vec<Vec<&super::node::Node>> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let node_ids: HashSet<&str> = nodes.iter().map(|n| n.node_id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&super::node::Node>> = HashMap::new();
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
        if !children.contains_key(node.node_id.as_str()) {
            continue;
        }
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

fn node_status(node: &super::node::Node) -> &'static str {
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

// User-visible output is locked in by the golden snapshots in
// `tests/snapshot_test.rs`. The unit tests below cover pure-function
// invariants that snapshots can't reach (state-graph filtering, multibyte
// truncation, what-next op-check projection).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::event::{Event, EventType, Lifecycle, NodeType, ThreadKind, ThreadStatus};
    use crate::internal::node::Node;
    use crate::internal::thread::ThreadState;
    use chrono::TimeZone;

    fn fixed_state() -> ThreadState {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: "RFC-0001".into(),
            kind: ThreadKind::Rfc,
            // Phase 2c: lifecycle is independent — keep it kind-aligned.
            lifecycle: Lifecycle::Proposal,
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            status: ThreadStatus::Draft,
            created_at: t,
            created_by: "human/alice".into(),
            events: vec![Event {
                event_id: "evt-0001".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::Create,
                created_at: t,
                actor: "human/alice".into(),
                title: Some("Test RFC".into()),
                kind: Some(ThreadKind::Rfc),
                ..Event::default()
            }],
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

    #[test]
    fn timeline_summarizes_long_bodies_to_one_line() {
        let mut state = fixed_state();
        let long_body = "First line of a multi-paragraph body\n\nSecond paragraph with more detail that is long enough to exceed the eighty character limit for preview";
        state.events[0].body = Some(long_body.into());
        state.events[0].event_type = EventType::Say;
        state.events[0].node_type = Some(NodeType::Claim);
        state.events[0].target_node_id = Some("node-0001".into());
        let out = render_show(&state, &ShowOptions::default());
        assert!(out.contains("First line of a multi-paragraph body"));
        assert!(out.contains("..."));
        assert!(!out.contains("eighty character limit for preview"));
    }

    #[test]
    fn revise_body_timeline_shows_size_summary() {
        let mut state = fixed_state();
        state.events[0].body = Some("x".repeat(2048));
        state.events[0].event_type = EventType::ReviseBody;
        state.events[0].incorporated_node_ids = vec!["node-001".into(), "node-002".into()];
        let out = render_show(&state, &ShowOptions::default());
        assert!(out.contains("(2.0 KB, incorporated 2 node(s))"));
        assert!(!out.contains(&"x".repeat(100)));
    }

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
    fn state_diagram_filters_by_lifecycle() {
        // Proposal: withdrawn is reachable (from draft).
        assert!(render_state_diagram(Lifecycle::Proposal, "open")
            .join("\n")
            .contains("withdrawn"));
        // Execution: withdrawn excluded by §3.1.1 allowed_states.
        let exec = render_state_diagram(Lifecycle::Execution, "open").join("\n");
        assert!(!exec.contains("withdrawn"));
        assert!(exec.contains("working") && exec.contains("review"));
        // Record: minimal — no review/working.
        let record = render_state_diagram(Lifecycle::Record, "open").join("\n");
        assert!(!record.contains("review") && !record.contains("working"));
        assert!(record.contains("[open]") && record.contains("done"));
    }

    #[test]
    fn what_next_mode_includes_operation_checks() {
        let state = fixed_state();
        let policy = crate::internal::policy::Policy {
            node_rules: {
                let mut m = std::collections::HashMap::new();
                m.insert("draft".into(), vec![NodeType::Claim, NodeType::Question]);
                m
            },
            revise_rules: Some(crate::internal::policy::ReviseRules {
                allow_body_revise: vec!["draft".into()],
                allow_node_revise: vec![],
            }),
            evidence_rules: Some(crate::internal::policy::EvidenceRules {
                allow_evidence: vec!["draft".into(), "proposed".into()],
            }),
            ..Default::default()
        };
        let out = render_show(
            &state,
            &ShowOptions {
                mode: ShowMode::WhatNext,
                policy: Some(policy),
                ..ShowOptions::default()
            },
        );
        assert!(out.contains("operation checks (state: draft):"));
        assert!(out.contains("node types: claim, question"));
        assert!(out.contains("body revise: allowed"));
        assert!(out.contains("evidence:    allowed"));
    }

    #[test]
    fn status_mode_lists_open_items() {
        let mut state = fixed_state();
        state.nodes.push(Node {
            node_id: "node-0001".into(),
            node_type: NodeType::Objection,
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
