//! Renderers for `git forum show`, `node show`, `log`, `status`, `ls`,
//! `shortlog`, and search results. Per RFC-lmr3wfcm Track E, the thread-detail
//! renderers all collapse to a single [`render_show`] driven by [`ShowOptions`].
//!
//! task `1hg98odf`: the `Show` arm body relocates from
//! `main.rs` to [`run`] in this module. The `--tree` advisory's
//! [`collect_implements_children`] / [`fallback_scan_implements`]
//! helpers also move here — they are now exclusively a `show --tree`
//! concern, and per task `913c4s9v` the SQLite reverse-link index
//! is on the task `913c4s9v` DELETE list, so the tree-scan fallback is the
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
    /// (task `913c4s9v`, RFC `7ymtc4b2`: replaces `state.events`-driven
    /// timeline rendering. The TUI per-thread timeline panel uses the
    /// same surface.)
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
        short_oid(&node.record.id),
        node_display_label(node)
    ));
    lines.push(String::new());
    lines.push(format!(
        "**thread:**    {} {}",
        lookup.thread_id, lookup.thread_title
    ));
    // SPEC-2.0 classification: lifecycle + tags, not kind.
    lines.push(format!(
        "**lifecycle:** {}",
        super::super::policy::lifecycle_label_for(&lookup.thread_category, &lookup.thread_tags)
    ));
    if !lookup.thread_tags.is_empty() {
        lines.push(format!("**tags:**      {}", lookup.thread_tags.join(", ")));
    }
    lines.push(format!("**status:**    {}", node_status(node)));
    lines.push(format!(
        "**created:**   {}",
        node.record.created_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(format!("**by:**        {}", node.record.created_by));
    if let Some(ref parent_id) = node.record.reply_to {
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
        lines.extend(node_history_lines(&lookup.node.record.id, options));
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
//  Mode: Full — section model
// ============================================================
//
// task `px3ss55s`: `render_full` builds a `Vec<Section>` of typed
// blocks, then joins them into the CLI string. The TUI calls
// `render_full_sections` directly so it can render the three
// structured-section variants (`OpenObjections`, `OpenActions`,
// `Conversations`) with the node-kind palette while keeping every
// other block (header, body, evidence, links, timeline, …) as
// pre-formatted text. The CLI and TUI surfaces share the section
// data, so counts and grouping cannot drift between them.

/// One block of `git forum show --mode full` output. Each variant
/// is independently elidable: a section that produces no rows
/// emits no text on the CLI and no rendered region in the TUI.
#[derive(Debug, Clone)]
pub enum Section {
    /// Pre-formatted CLI lines for any block other than the three
    /// structured variants. Trailing blanks are part of the
    /// section so the joined output preserves spacing.
    Text(Vec<String>),
    /// `Open objections (N)` — open `objection` nodes (gates
    /// forward transitions via `no_open_objections`).
    OpenObjections(OpenItemsSection),
    /// `Open actions (N)` — open `action` nodes (gates forward
    /// transitions via `no_open_actions`).
    OpenActions(OpenItemsSection),
    /// `Conversations (N)` — root nodes plus their transitive
    /// replies, grouped via `build_conversations`.
    Conversations(ConversationsSection),
}

/// Source data for `Open objections` / `Open actions` sections.
#[derive(Debug, Clone)]
pub struct OpenItemsSection {
    pub thread_id: String,
    pub items: Vec<OpenItem>,
    /// `true` for objections (CLI emits the per-row `reply:`
    /// cheat-sheet), `false` for actions (resolve only).
    pub with_reply: bool,
    /// Compact mode controls how the body preview is computed
    /// (truncated to 60 chars vs first-line only).
    pub compact: bool,
}

#[derive(Debug, Clone)]
pub struct OpenItem {
    pub id_short: String,
    pub author: String,
    pub body: String,
    pub kind: super::super::node::NodeKind,
}

/// Source data for the `Conversations` section.
#[derive(Debug, Clone)]
pub struct ConversationsSection {
    pub thread_id: String,
    pub chains: Vec<ConversationChain>,
    pub compact: bool,
    /// Number of `Incorporated` nodes in the thread; surfaced as
    /// the compact-mode `(N incorporated)` hint after the count.
    pub incorporated_count: usize,
}

#[derive(Debug, Clone)]
pub struct ConversationChain {
    /// `nodes[0]` = root; subsequent entries are replies in BFS
    /// order produced by `build_conversations`.
    pub nodes: Vec<ConversationNode>,
    /// Whether `nodes.last()` is `Open`. Drives the chain-trailing
    /// `reply:` cheat-sheet line — matches today's
    /// `push_conversations` `convo.last()` behaviour.
    pub last_is_open: bool,
}

#[derive(Debug, Clone)]
pub struct ConversationNode {
    pub id_short: String,
    pub kind: super::super::node::NodeKind,
    /// Display label (legacy_subtype if set, else kind name).
    pub label: String,
    /// `node_status` string (`open`, `resolved`, …).
    pub status_label: String,
    pub author: String,
    pub body: String,
}

impl Section {
    /// Render this section as the CLI plain-text lines that today
    /// live in `render_full`. Empty sections produce no lines so
    /// callers can join the sections directly.
    pub fn to_text_lines(&self) -> Vec<String> {
        match self {
            Section::Text(lines) => lines.clone(),
            Section::OpenObjections(s) => render_open_items_text(s, "Open objections"),
            Section::OpenActions(s) => render_open_items_text(s, "Open actions"),
            Section::Conversations(s) => render_conversations_text(s),
        }
    }
}

/// Build the typed section list for `ShowMode::Full`. Both the CLI
/// (via `render_full`) and the TUI body pane consume the same list.
pub fn render_full_sections(state: &ThreadState, options: &ShowOptions) -> Vec<Section> {
    let compact = options.compact;
    let mut sections: Vec<Section> = Vec::new();

    sections.push(Section::Text(build_header_lines(state, options)));

    if !compact && state.body_revision_count > 0 {
        sections.push(Section::Text(vec![format!(
            "**body revisions:** {}",
            state.body_revision_count
        )]));
    }

    if !compact {
        let mut buf: Vec<String> = Vec::new();
        let incorporated: Vec<&super::super::snapshot::store::NodeWithBody> = state
            .nodes
            .iter()
            .filter(|n| n.record.status == super::super::node::NodeStatus::Incorporated)
            .collect();
        render_item_list(&mut buf, "incorporated nodes", &incorporated, |node| {
            format!(
                "  - {} {} {}",
                short_oid(&node.record.id),
                node_display_label(node),
                body_or_truncated(&node.body, 60, false)
            )
        });
        if !buf.is_empty() {
            sections.push(Section::Text(buf));
        }
    }

    sections.push(Section::OpenObjections(collect_open_items_section(
        &state.id,
        &state.open_objections(),
        compact,
        true,
    )));
    sections.push(Section::OpenActions(collect_open_items_section(
        &state.id,
        &state.open_actions(),
        compact,
        false,
    )));

    if let Some(summary) = state.latest_summary() {
        sections.push(Section::Text(vec![
            "**latest summary:**".to_string(),
            format!("  {}", summary.body),
            String::new(),
        ]));
    }

    if !compact {
        let mut buf: Vec<String> = Vec::new();
        render_item_list(&mut buf, "evidence", &state.evidence_items, |ev| {
            let id_short = &ev.id[..ev.id.len().min(8)];
            format!("  - {}  {}  {}", id_short, ev.kind, ev.ref_target)
        });
        if !buf.is_empty() {
            sections.push(Section::Text(buf));
        }
    }

    {
        let mut buf: Vec<String> = Vec::new();
        render_item_list(&mut buf, "links", &state.links, |link| {
            format!("  - {}  {}", link.target_thread_id, link.rel)
        });
        if !buf.is_empty() {
            sections.push(Section::Text(buf));
        }
    }

    sections.push(Section::Conversations(collect_conversations_section(
        &state.id, state, compact,
    )));

    if !options.no_timeline {
        let mut buf: Vec<String> = Vec::new();
        buf.push("---".into());
        buf.push(String::new());
        if compact {
            let count = options
                .timeline_entries
                .as_ref()
                .map(|e| e.len())
                .unwrap_or(0);
            buf.push(format!(
                "**timeline:** {} commits (use 'log {}' for full view)",
                count, state.id
            ));
        } else {
            buf.push("### timeline".into());
            buf.push(String::new());
            buf.extend(thread_history_lines(options));
        }
        buf.push(String::new());
        sections.push(Section::Text(buf));
    }

    if !compact && (!state.open_objections().is_empty() || !state.open_actions().is_empty()) {
        sections.push(Section::Text(vec![
            format!(
                "tip: run `git forum show {} --what-next` for action guidance",
                state.id
            ),
            String::new(),
        ]));
    }

    sections
}

fn render_full(state: &ThreadState, options: &ShowOptions) -> String {
    let mut lines: Vec<String> = Vec::new();
    for section in render_full_sections(state, options) {
        lines.extend(section.to_text_lines());
    }
    lines.join("\n")
}

fn build_header_lines(state: &ThreadState, options: &ShowOptions) -> Vec<String> {
    let compact = options.compact;
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## {} {}", state.id, state.title));
    lines.push(String::new());
    // SPEC-2.0 classification (Finding 1): lifecycle + tags are the canonical 2.0
    // classification axis. The legacy `kind` field stays in storage for
    // backward compatibility (per SPEC-3.0 §8.3) but is no longer surfaced as
    // a primary display label.
    lines.push(format!(
        "**lifecycle:** {}",
        policy::lifecycle_label_for(&state.category, &state.tags)
    ));
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
            for diagram_line in render_state_diagram(&state.category, state.status.as_str()) {
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
    lines
}

fn collect_open_items_section(
    thread_id: &str,
    items: &[&super::super::snapshot::store::NodeWithBody],
    compact: bool,
    with_reply: bool,
) -> OpenItemsSection {
    OpenItemsSection {
        thread_id: thread_id.to_string(),
        with_reply,
        compact,
        items: items
            .iter()
            .map(|n| OpenItem {
                id_short: short_oid(&n.record.id).to_string(),
                author: n.record.created_by.clone(),
                body: n.body.clone(),
                kind: n.record.kind,
            })
            .collect(),
    }
}

fn collect_conversations_section(
    thread_id: &str,
    state: &ThreadState,
    compact: bool,
) -> ConversationsSection {
    let incorporated_count = if compact {
        state
            .nodes
            .iter()
            .filter(|n| n.record.status == super::super::node::NodeStatus::Incorporated)
            .count()
    } else {
        0
    };
    let chains: Vec<ConversationChain> = build_conversations(&state.nodes)
        .into_iter()
        .map(|chain| {
            let last_is_open = chain
                .last()
                .map(|n| n.record.status == super::super::node::NodeStatus::Open)
                .unwrap_or(false);
            let nodes = chain
                .iter()
                .map(|n| ConversationNode {
                    id_short: short_oid(&n.record.id).to_string(),
                    kind: n.record.kind,
                    label: node_display_label(n),
                    status_label: node_status(n).to_string(),
                    author: n.record.created_by.clone(),
                    body: n.body.clone(),
                })
                .collect();
            ConversationChain {
                nodes,
                last_is_open,
            }
        })
        .collect();
    ConversationsSection {
        thread_id: thread_id.to_string(),
        chains,
        compact,
        incorporated_count,
    }
}

fn render_open_items_text(section: &OpenItemsSection, header: &str) -> Vec<String> {
    if section.items.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    out.push(format!("## {} ({})", header, section.items.len()));
    out.push(String::new());
    for item in &section.items {
        let preview = body_or_truncated(&item.body, 60, section.compact);
        out.push(format!("  {}  {}  {}", item.id_short, item.author, preview));
        out.push(format!(
            "    resolve: git forum resolve {} {}",
            section.thread_id, item.id_short
        ));
        if section.with_reply {
            out.push(format!(
                "    reply:   git forum claim {} --reply-to {} --body \"...\"",
                section.thread_id, item.id_short
            ));
        }
    }
    out.push(String::new());
    out
}

fn render_conversations_text(section: &ConversationsSection) -> Vec<String> {
    if section.chains.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if section.compact {
        let inc_hint = if section.incorporated_count > 0 {
            format!(" ({} incorporated)", section.incorporated_count)
        } else {
            String::new()
        };
        out.push(format!(
            "## Conversations ({}){}",
            section.chains.len(),
            inc_hint
        ));
        out.push(String::new());
        for chain in &section.chains {
            let root = &chain.nodes[0];
            out.push(format!(
                "  {} {} [{}] {} — {}",
                root.id_short,
                root.label,
                root.status_label,
                root.author,
                single_line_preview(&root.body, 60)
            ));
        }
    } else {
        // task `px3ss55s`: non-compact mode renders each chain as a
        // markdown h3 root, blockquote root preview, bullet list of
        // replies, and the `reply:` cheat-sheet on the last open node.
        // Full bodies live in `git forum node show <id>`.
        out.push(format!("## Conversations ({})", section.chains.len()));
        out.push(String::new());
        for (i, chain) in section.chains.iter().enumerate() {
            let root = &chain.nodes[0];
            out.push(format!(
                "### {} {} [{}] {}",
                root.id_short, root.label, root.status_label, root.author
            ));
            out.push(format!("> {}", first_line_preview(&root.body, 100)));
            for reply in &chain.nodes[1..] {
                out.push(format!(
                    "- {} {} {} — {}",
                    reply.id_short,
                    reply.label,
                    reply.author,
                    first_line_preview(&reply.body, 80)
                ));
            }
            if chain.last_is_open {
                out.push(String::new());
                out.push(format!(
                    "    reply: git forum claim {} --reply-to {} --body \"...\"",
                    section.thread_id,
                    chain.nodes.last().unwrap().id_short
                ));
            }
            if i + 1 < section.chains.len() {
                out.push(String::new());
            }
        }
    }
    out.push(String::new());
    out
}

fn first_line_preview(body: &str, max: usize) -> String {
    let first = body.lines().next().unwrap_or("");
    if first.chars().count() <= max {
        first.to_string()
    } else {
        let truncated: String = first.chars().take(max).collect();
        format!("{truncated}…")
    }
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

    use super::super::node::NodeStatus;
    let open_obj = state.open_objections();
    let open_act = state.open_actions();
    let open_questions: Vec<&super::super::snapshot::store::NodeWithBody> = state
        .nodes
        .iter()
        .filter(|n| {
            n.record.legacy_label.as_deref() == Some("question")
                && n.record.status == NodeStatus::Open
        })
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

fn push_status_group(
    lines: &mut Vec<String>,
    label: &str,
    items: &[&super::super::snapshot::store::NodeWithBody],
) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("  {label} ({})", items.len()));
    for node in items {
        lines.push(format!(
            "    - {} {}",
            short_oid(&node.record.id),
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
                short_oid(&node.record.id),
                state.id,
                short_oid(&node.record.id)
            ));
        }
        for node in &open_act {
            lines.push(format!(
                "    action {} — resolve with: resolve {} {}",
                short_oid(&node.record.id),
                state.id,
                short_oid(&node.record.id)
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
/// `Lifecycle::allows_state`) per RFC `7ymtc4b2`, task `1v400j3l`.
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
fn node_display_label(node: &super::super::snapshot::store::NodeWithBody) -> String {
    node.record
        .legacy_label
        .clone()
        .unwrap_or_else(|| node.record.kind.to_string())
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
/// replies, in chronological order. Standalone leaf nodes appear as a
/// one-element chain (matches CLI / TUI shared semantics, task `px3ss55s`).
///
/// `pub(crate)` so the TUI body-pane composer (`internal::tui::render`)
/// can reuse the grouping without duplicating logic.
pub(crate) fn build_conversations(
    nodes: &[super::super::snapshot::store::NodeWithBody],
) -> Vec<Vec<&super::super::snapshot::store::NodeWithBody>> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let node_ids: HashSet<&str> = nodes.iter().map(|n| n.record.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&super::super::snapshot::store::NodeWithBody>> =
        HashMap::new();
    let mut has_parent: HashSet<&str> = HashSet::new();
    for node in nodes {
        if let Some(ref parent_id) = node.record.reply_to {
            if node_ids.contains(parent_id.as_str()) {
                children.entry(parent_id.as_str()).or_default().push(node);
                has_parent.insert(node.record.id.as_str());
            }
        }
    }

    let mut conversations = Vec::new();
    for node in nodes {
        if has_parent.contains(node.record.id.as_str()) {
            continue;
        }
        // SPEC-3.0 transitional (RFC `7ymtc4b2`): in the v2 event-chain
        // model the timeline carried Say bodies inline, so a leaf
        // comment with no replies could be elided from the
        // conversations section without losing visibility. Snapshot
        // threads have no event timeline, so leaf nodes must surface
        // here too. The legacy event-chain leaf-only filter is removed.
        let mut chain = vec![node];
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(node.record.id.as_str());
        while let Some(parent_id) = queue.pop_front() {
            if let Some(replies) = children.get(parent_id) {
                for reply in replies {
                    chain.push(reply);
                    queue.push_back(reply.record.id.as_str());
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

fn node_status(node: &super::super::snapshot::store::NodeWithBody) -> &'static str {
    use super::super::node::NodeStatus;
    match node.record.status {
        NodeStatus::Retracted => "retracted",
        NodeStatus::Incorporated => "incorporated",
        NodeStatus::Resolved => "resolved",
        NodeStatus::Open => "open",
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
    let parent_lifecycle = policy::lifecycle_label_for(&parent.category, &parent.tags);
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
/// task `1hg98odf`: relocated from `main.rs`. Per task `913c4s9v`
/// Decision 6 the SQLite reverse-link index is on the task `913c4s9v` DELETE list,
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
                lifecycle_label: policy::lifecycle_label_for(
                    &child_state.category,
                    &child_state.tags,
                )
                .to_string(),
                status: child_state.status.clone(),
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
    use crate::internal::node::NodeKind;
    use crate::internal::thread::ThreadState;
    use chrono::TimeZone;

    fn fixed_state() -> ThreadState {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: "RFC-0001".into(),
            category: "rfc".into(),
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            status: "draft".into(),
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

    // task `913c4s9v`: the v2 timeline body-preview / revise-body
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
        state
            .nodes
            .push(crate::internal::snapshot::store::NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "node-0001".into(),
                    kind: NodeKind::Objection,
                    created_at: state.created_at,
                    created_by: "ai/reviewer".into(),
                    ..Default::default()
                },
                body: "Bench results missing".into(),
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
            category: "rfc".into(),
            title: "Parent RFC".into(),
            status: "done".into(),
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
            category: "rfc".into(),
            title: "Lonely RFC".into(),
            status: "draft".into(),
            ..Default::default()
        };
        let out = render_tree(&parent, &[]);
        assert!(out.contains("RFC-LONELY"));
        assert!(out.contains("(no incoming `implements` links)"));
    }
}
