use super::event::{Event, EventType, ThreadKind};
use super::policy::{self, Policy};
use super::state_machine;
use super::thread::{NodeLookup, ThreadState};

#[derive(Debug, Clone, Default)]
pub struct ShowOptions {
    /// Truncate node bodies and conversation details to single-line previews.
    pub compact: bool,
    /// Omit the timeline section entirely.
    pub no_timeline: bool,
    /// When set, show compact next-states and state diagram.
    pub policy: Option<Policy>,
}

/// Render a compact Unicode state diagram for a thread kind.
///
/// The current state is highlighted with brackets: `[under-review]`.
/// Forward edges use `→`, branch edges use `├→` / `└→`.
pub fn render_state_diagram(kind: ThreadKind, current_status: &str) -> Vec<String> {
    // Define main spine (longest forward path) and branch edges per kind
    let (spine, branches): (&[&str], Vec<(&str, &str)>) = match kind {
        ThreadKind::Issue => (
            &["open", "pending", "closed"],
            vec![
                ("open", "rejected"),
                ("open", "closed"),
                ("rejected", "open"),
                ("closed", "open"),
            ],
        ),
        ThreadKind::Rfc => (
            &[
                "draft",
                "proposed",
                "under-review",
                "accepted",
                "deprecated",
            ],
            vec![
                ("draft", "rejected"),
                ("proposed", "draft"),
                ("under-review", "rejected"),
                ("under-review", "draft"),
                ("rejected", "deprecated"),
            ],
        ),
        ThreadKind::Dec => (
            &["proposed", "accepted", "deprecated"],
            vec![
                ("proposed", "rejected"),
                ("proposed", "deprecated"),
                ("rejected", "deprecated"),
            ],
        ),
        ThreadKind::Task => (
            &["open", "designing", "implementing", "reviewing", "closed"],
            vec![
                ("open", "rejected"),
                ("open", "closed"),
                ("designing", "rejected"),
                ("designing", "open"),
                ("implementing", "rejected"),
                ("implementing", "designing"),
                ("reviewing", "rejected"),
                ("reviewing", "implementing"),
                ("closed", "open"),
                ("rejected", "open"),
            ],
        ),
    };

    let fmt_state = |s: &str| -> String {
        if s == current_status {
            format!("[{s}]")
        } else {
            s.to_string()
        }
    };

    let mut lines = Vec::new();

    // Render main spine
    let spine_parts: Vec<String> = spine.iter().map(|s| fmt_state(s)).collect();
    lines.push(format!("  {}", spine_parts.join(" → ")));

    // Group branches by source state on the spine
    for &src in spine {
        let src_branches: Vec<&&str> = branches
            .iter()
            .filter(|(from, _)| *from == src)
            .map(|(_, to)| to)
            .collect();
        if src_branches.is_empty() {
            continue;
        }
        // Calculate indent to align under the source state on the spine
        let indent = spine_indent(spine, src, current_status);
        for (i, target) in src_branches.iter().enumerate() {
            let connector = if i + 1 < src_branches.len() {
                "├→"
            } else {
                "└→"
            };
            lines.push(format!("{indent}{connector} {}", fmt_state(target)));
        }
    }

    // Also show branches from non-spine states
    for (src, targets) in &branches {
        if spine.contains(src) {
            continue;
        }
        let indent = "  ";
        let target_list: Vec<String> = std::iter::once(targets).map(|t| fmt_state(t)).collect();
        lines.push(format!(
            "{indent}{} → {}",
            fmt_state(src),
            target_list.join(", ")
        ));
    }

    lines
}

/// Calculate whitespace indent to align a branch connector under a spine state.
fn spine_indent(spine: &[&str], src: &str, current: &str) -> String {
    let mut offset = 2; // leading "  "
    for &s in spine {
        if s == src {
            break;
        }
        // Width of state name (with brackets if current)
        let w = if s == current { s.len() + 2 } else { s.len() };
        offset += w + 3; // " → " separator
    }
    " ".repeat(offset)
}

/// Render `git forum show` output for a thread.
///
/// When `compact` is true, node bodies and timeline details are truncated
/// to single-line previews. When false (default), full bodies are shown.
/// Timeline event bodies are always summarized (single-line preview);
/// `revise-body` events show a size summary instead of full body text.
/// When `no_timeline` is true, the timeline section is omitted entirely.
///
/// Output is deterministic given deterministic event timestamps and IDs.
/// Snapshot strategy: tests use fixed synthetic events where needed;
/// integration tests should avoid asserting exact Git OIDs.
pub fn render_show(state: &ThreadState, compact: bool) -> String {
    render_show_with_options(
        state,
        &ShowOptions {
            compact,
            no_timeline: false,
            policy: None,
        },
    )
}

pub fn render_show_with_options(state: &ThreadState, options: &ShowOptions) -> String {
    let compact = options.compact;
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("{:<12} {}", state.id, state.title));
    lines.push(format!("kind:     {}", state.kind));
    lines.push(format!("status:   {}", state.status));
    if let Some(policy) = &options.policy {
        let targets = state_machine::valid_targets(state.kind, &state.status);
        if !targets.is_empty() {
            let mut target_parts: Vec<String> = Vec::new();
            for target in &targets {
                let violations = policy::check_guards(policy, state, &state.status, target, &[]);
                if violations.is_empty() {
                    target_parts.push(target.to_string());
                } else {
                    let blockers: Vec<String> = violations.iter().map(|v| v.rule.clone()).collect();
                    target_parts.push(format!("{target} (blocked: {})", blockers.join(", ")));
                }
            }
            lines.push(format!("next:     {}", target_parts.join(", ")));
        }
        lines.push("transitions:".into());
        for diagram_line in render_state_diagram(state.kind, &state.status) {
            lines.push(diagram_line);
        }
    }
    lines.push(format!(
        "created:  {}",
        state.created_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(format!("by:       {}", state.created_by));
    if let Some(branch) = &state.branch {
        lines.push(format!("branch:   {}", branch));
    }
    if let Some(body) = &state.body {
        lines.push("body:".into());
        for line in body.lines() {
            lines.push(format!("  {line}"));
        }
        if body.is_empty() {
            lines.push("  ".into());
        }
    }
    lines.push(String::new());

    if state.body_revision_count > 0 {
        lines.push(format!("body revisions: {}", state.body_revision_count));
    }

    let incorporated: Vec<&super::node::Node> =
        state.nodes.iter().filter(|n| n.incorporated).collect();
    render_item_list(&mut lines, "incorporated nodes", &incorporated, |node| {
        let preview = body_or_truncated(&node.body, 60, compact);
        format!(
            "  - {} {} {}",
            short_oid(&node.node_id),
            node.node_type,
            preview
        )
    });

    let open_obj = state.open_objections();
    render_item_list(&mut lines, "open objections", &open_obj, |node| {
        let preview = body_or_truncated(&node.body, 60, compact);
        format!("  - {} {}", short_oid(&node.node_id), preview)
    });

    let open_act = state.open_actions();
    render_item_list(&mut lines, "open actions", &open_act, |node| {
        let preview = body_or_truncated(&node.body, 60, compact);
        format!("  - {} {}", short_oid(&node.node_id), preview)
    });

    if let Some(summary) = state.latest_summary() {
        lines.push("latest summary:".into());
        lines.push(format!("  {}", summary.body));
        lines.push(String::new());
    }

    render_item_list(&mut lines, "evidence", &state.evidence_items, |ev| {
        let id_short = &ev.evidence_id[..ev.evidence_id.len().min(8)];
        format!("  - {}  {}  {}", id_short, ev.kind, ev.ref_target)
    });

    render_item_list(&mut lines, "links", &state.links, |link| {
        format!("  - {}  {}", link.target_thread_id, link.rel)
    });

    // Conversation grouping: show reply chains grouped by root node
    let conversations = build_conversations(&state.nodes);
    if !conversations.is_empty() {
        lines.push(format!("conversations: {}", conversations.len()));
        for convo in &conversations {
            let root = convo[0];
            let status = node_status(root);
            lines.push(format!(
                "  {} {} [{}] {}",
                short_oid(&root.node_id),
                root.node_type,
                status,
                body_or_truncated(&root.body, 50, compact)
            ));
            if !compact {
                render_indented_body(&mut lines, &root.body, "      ");
            }
            for reply in &convo[1..] {
                lines.push(format!(
                    "    -> {} {} {} {}",
                    short_oid(&reply.node_id),
                    reply.node_type,
                    reply.actor,
                    body_or_truncated(&reply.body, 50, compact)
                ));
                if !compact {
                    render_indented_body(&mut lines, &reply.body, "        ");
                }
            }
        }
        lines.push(String::new());
    }

    if !options.no_timeline {
        lines.push("timeline:".into());
        let widths = timeline_widths(&state.events);
        lines.push(format_timeline_header(&widths));
        for event in &state.events {
            lines.push(format_timeline_entry(event, &widths, compact));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render a hint showing valid next actions for a thread.
///
/// Includes valid state transitions with guard check results, open item counts,
/// and actionable IDs for open items so agents can construct resolve commands.
pub fn render_next_actions(state: &ThreadState, policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    let targets = state_machine::valid_targets(state.kind, &state.status);
    if targets.is_empty() {
        lines.push("  next: (no transitions available)".to_string());
    } else {
        let mut target_parts: Vec<String> = Vec::new();
        for target in &targets {
            let violations = policy::check_guards(policy, state, &state.status, target, &[]);
            if violations.is_empty() {
                target_parts.push(target.to_string());
            } else {
                let blockers: Vec<String> = violations
                    .iter()
                    .map(|v| format!("{}: {}", v.rule, v.reason))
                    .collect();
                target_parts.push(format!("{target} (blocked: {})", blockers.join("; ")));
            }
        }
        lines.push(format!("  next: {}", target_parts.join(", ")));
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
        // List IDs so agents can construct resolve commands
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

/// Render `git forum show --what-next` output.
///
/// Shows valid transitions with guard check results, and open item counts.
pub fn render_what_next(state: &ThreadState, policy: &Policy) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("{} ({})", state.id, state.status));
    lines.push(String::new());

    let targets = state_machine::valid_targets(state.kind, &state.status);
    if targets.is_empty() {
        lines.push("valid transitions: (none)".to_string());
    } else {
        lines.push(format!("valid transitions: {}", targets.join(", ")));
    }
    lines.push(String::new());

    // Guard check for each valid transition
    for target in &targets {
        let violations = policy::check_guards(policy, state, &state.status, target, &[]);
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

    // Open items
    let obj = state.open_objections().len();
    let act = state.open_actions().len();
    lines.push(format!("open objections: {obj}"));
    lines.push(format!("open actions:    {act}"));
    lines.push(format!("nodes:           {}", state.nodes.len()));
    lines.push(format!("evidence:        {}", state.evidence_items.len()));
    lines.push(format!("links:           {}", state.links.len()));

    let has_summary = state.latest_summary().is_some();
    lines.push(format!(
        "has summary:     {}",
        if has_summary { "yes" } else { "no" }
    ));
    lines.push(String::new());

    // Operation check rules for current state
    let mut op_lines: Vec<String> = Vec::new();

    // Node rules
    if !policy.node_rules.is_empty() {
        if let Some(allowed) = policy.node_rules.get(&state.status) {
            if allowed.is_empty() {
                op_lines.push("  node types: (none allowed)".into());
            } else {
                let types: Vec<String> = allowed.iter().map(|n| n.to_string()).collect();
                op_lines.push(format!("  node types: {}", types.join(", ")));
            }
        } else {
            op_lines.push("  node types: (all allowed)".into());
        }
    }

    // Revise rules
    if let Some(revise) = &policy.revise_rules {
        if !revise.allow_body_revise.is_empty() {
            let allowed = revise.allow_body_revise.iter().any(|s| s == &state.status);
            op_lines.push(format!(
                "  body revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
        if !revise.allow_node_revise.is_empty() {
            let allowed = revise.allow_node_revise.iter().any(|s| s == &state.status);
            op_lines.push(format!(
                "  node revise: {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
    }

    // Evidence rules
    if let Some(evidence) = &policy.evidence_rules {
        if !evidence.allow_evidence.is_empty() {
            let allowed = evidence.allow_evidence.iter().any(|s| s == &state.status);
            op_lines.push(format!(
                "  evidence:    {}",
                if allowed { "allowed" } else { "blocked" }
            ));
        }
    }

    if !op_lines.is_empty() {
        lines.push(format!("operation checks (state: {}):", state.status));
        lines.extend(op_lines);
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render `git forum node show` output for a single node.
pub fn render_node_show(lookup: &NodeLookup) -> String {
    let mut lines: Vec<String> = Vec::new();
    let node = &lookup.node;

    lines.push(format!(
        "{:<18} {}",
        short_oid(&node.node_id),
        node.node_type
    ));
    lines.push(format!(
        "thread:   {} {}",
        lookup.thread_id, lookup.thread_title
    ));
    lines.push(format!("kind:     {}", lookup.thread_kind));
    lines.push(format!("status:   {}", node_status(node)));
    lines.push(format!(
        "created:  {}",
        node.created_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    lines.push(format!("by:       {}", node.actor));
    if let Some(ref parent_id) = node.reply_to {
        lines.push(format!("reply-to: {}", short_oid(parent_id)));
    }
    lines.push("body:".into());
    for line in node.body.lines() {
        lines.push(format!("  {line}"));
    }
    if node.body.is_empty() {
        lines.push("  ".into());
    }
    lines.push(String::new());

    if !lookup.links.is_empty() {
        lines.push(format!("thread links: {}", lookup.links.len()));
        for link in &lookup.links {
            lines.push(format!("  - {}  {}", link.target_thread_id, link.rel));
        }
        lines.push(String::new());
    }

    lines.push("history:".into());
    let widths = timeline_widths(&lookup.events);
    lines.push(format_timeline_header(&widths));
    for event in &lookup.events {
        lines.push(format_timeline_entry(event, &widths, false));
    }
    lines.push(String::new());

    lines.join("\n")
}

fn event_detail(event: &Event) -> String {
    match event.event_type {
        EventType::Create => event.title.clone().unwrap_or_default(),
        EventType::State => {
            let state = event.new_state.clone().unwrap_or_default();
            match &event.body {
                Some(body) if !body.is_empty() => format!("{state} — {body}"),
                _ => state,
            }
        }
        EventType::Scope => event
            .branch
            .clone()
            .unwrap_or_else(|| "(clear branch)".into()),
        EventType::Link => {
            if let Some(evidence) = &event.evidence {
                format!("{} {}", evidence.kind, evidence.ref_target)
            } else if let (Some(target), Some(rel)) = (&event.target_node_id, &event.link_rel) {
                format!("{target} ({rel})")
            } else {
                String::new()
            }
        }
        EventType::Say | EventType::Edit | EventType::ReviseBody => {
            event.body.clone().unwrap_or_default()
        }
        _ => String::new(),
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

fn event_node_id(event: &Event) -> Option<&str> {
    match event.event_type {
        EventType::Say => Some(
            event
                .target_node_id
                .as_deref()
                .unwrap_or(event.event_id.as_str()),
        ),
        _ => event.target_node_id.as_deref(),
    }
}

fn event_display_type(event: &Event) -> String {
    match event.event_type {
        EventType::Say => event
            .node_type
            .map(|node_type| node_type.to_string())
            .unwrap_or_else(|| event.event_type.to_string()),
        EventType::ReviseBody => "revise-body".to_string(),
        _ => event.event_type.to_string(),
    }
}

fn timeline_body(event: &Event, _compact: bool) -> String {
    // ReviseBody events get a size summary instead of full body text
    if event.event_type == EventType::ReviseBody {
        return revise_body_summary(event);
    }
    // All other events: single-line preview to keep timeline scannable
    let detail = event_detail(event);
    single_line_preview(&detail, 80)
}

fn revise_body_summary(event: &Event) -> String {
    let body = event.body.as_deref().unwrap_or("");
    let size = body.len();
    let size_str = if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    };
    let inc_count = event.incorporated_node_ids.len();
    if inc_count > 0 {
        format!("({size_str}, incorporated {inc_count} node(s))")
    } else {
        format!("({size_str})")
    }
}

pub struct TimelineWidths {
    date: usize,
    node_id: usize,
    event_id: usize,
    author: usize,
    r#type: usize,
}

pub fn timeline_widths(events: &[Event]) -> TimelineWidths {
    let mut widths = TimelineWidths {
        date: 20,
        node_id: 16,
        event_id: 16,
        author: 18,
        r#type: 10,
    };

    for event in events {
        widths.date = widths.date.max(
            event
                .created_at
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
                .len(),
        );
        widths.node_id = widths.node_id.max(
            event_node_id(event)
                .map(short_oid)
                .map(str::len)
                .unwrap_or(1),
        );
        widths.event_id = widths.event_id.max(short_oid(&event.event_id).len());
        widths.author = widths.author.max(event.actor.len());
        widths.r#type = widths.r#type.max(event_display_type(event).len());
    }

    widths
}

pub fn format_timeline_header(widths: &TimelineWidths) -> String {
    format!(
        "  {:<date$}  {:<node_id$}  {:<event_id$}  {:<author$}  {:<type$}  {}",
        "date",
        "node_id",
        "event_id",
        "author",
        "type",
        "body",
        date = widths.date,
        node_id = widths.node_id,
        event_id = widths.event_id,
        author = widths.author,
        type = widths.r#type,
    )
}

pub fn format_timeline_entry(event: &Event, widths: &TimelineWidths, compact: bool) -> String {
    format!(
        "  {:<date$}  {:<node_id$}  {:<event_id$}  {:<author$}  {:<type$}  {}",
        event.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
        event_node_id(event).map(short_oid).unwrap_or("-"),
        short_oid(&event.event_id),
        event.actor,
        event_display_type(event),
        timeline_body(event, compact),
        date = widths.date,
        node_id = widths.node_id,
        event_id = widths.event_id,
        author = widths.author,
        type = widths.r#type,
    )
}

/// Build conversation groups from nodes with reply chains.
///
/// A conversation is a root node (no reply_to, or reply_to not found in nodes)
/// plus all transitive replies to it, in chronological order.
/// Only nodes that are part of a reply chain (have reply_to or are replied to) are included.
fn build_conversations(nodes: &[super::node::Node]) -> Vec<Vec<&super::node::Node>> {
    use std::collections::{HashMap, HashSet};

    // Build parent->children map
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

    // Roots: nodes that have children OR are replied to, but have no parent in the thread
    let mut roots: Vec<&super::node::Node> = Vec::new();
    for node in nodes {
        if !has_parent.contains(node.node_id.as_str())
            && children.contains_key(node.node_id.as_str())
        {
            roots.push(node);
        }
    }

    // Build conversation chains via BFS
    let mut conversations = Vec::new();
    for root in roots {
        let mut chain = vec![root];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root.node_id.as_str());
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

/// Render a non-empty list with a header, formatted items, and trailing blank line.
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

/// Return truncated body in compact mode, or the first line in full mode.
fn body_or_truncated(s: &str, max: usize, compact: bool) -> String {
    if compact {
        truncate_body(s, max)
    } else {
        s.lines().next().unwrap_or("").to_string()
    }
}

/// Render multi-line body text indented under a node header (full mode only).
/// Only emits lines if the body has more than one line.
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

/// Render `git forum status` output for a single thread.
pub fn render_status(state: &ThreadState) -> String {
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
        if !open_obj.is_empty() {
            lines.push(format!("  objections ({})", open_obj.len()));
            for node in &open_obj {
                lines.push(format!(
                    "    - {} {}",
                    short_oid(&node.node_id),
                    truncate_body(&node.body, 60)
                ));
            }
        }
        if !open_act.is_empty() {
            lines.push(format!("  actions ({})", open_act.len()));
            for node in &open_act {
                lines.push(format!(
                    "    - {} {}",
                    short_oid(&node.node_id),
                    truncate_body(&node.body, 60)
                ));
            }
        }
        if !open_questions.is_empty() {
            lines.push(format!("  questions ({})", open_questions.len()));
            for node in &open_questions {
                lines.push(format!(
                    "    - {} {}",
                    short_oid(&node.node_id),
                    truncate_body(&node.body, 60)
                ));
            }
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Render search results from the local index.
pub fn render_search_results(rows: &[super::index::SearchRow]) -> String {
    if rows.is_empty() {
        return "no threads found\n".into();
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<12}  {:<10}  {:<14}  {}",
        "ID", "KIND", "STATUS", "TITLE"
    ));
    lines.push("-".repeat(60));
    for r in rows {
        lines.push(format!(
            "{:<12}  {:<10}  {:<14}  {}",
            r.thread.id, r.thread.kind, r.thread.status, r.thread.title
        ));
        for hit in &r.node_hits {
            lines.push(format!(
                "  -> node {}  {:<10}  {:<10}  {}",
                short_oid(&hit.node_id),
                hit.node_type,
                hit.status,
                single_line_preview(&hit.body, 60),
            ));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn short_oid(id: &str) -> &str {
    &id[..id.len().min(16)]
}

/// Render `git forum ls` output for a list of threads.
///
/// Output columns: ID, KIND, STATUS, BRANCH, CREATED, UPDATED, TITLE.
/// Deterministic when thread IDs and statuses are deterministic.
pub fn render_ls(states: &[&ThreadState]) -> String {
    if states.is_empty() {
        return "no threads found\n".into();
    }
    let id_width = states
        .iter()
        .map(|s| s.id.len())
        .max()
        .unwrap_or(12)
        .max(12);
    let kind_width = states
        .iter()
        .map(|s| s.kind.to_string().len())
        .max()
        .unwrap_or(10)
        .max(10);
    let status_width = states
        .iter()
        .map(|s| s.status.len())
        .max()
        .unwrap_or(14)
        .max(14);
    let branch_width = states
        .iter()
        .map(|s| s.branch.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(12)
        .max(12);
    let date_width = 16; // YYYY-MM-DD HH:MM
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<id_width$}  {:<kind_width$}  {:<status_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
        "ID", "KIND", "STATUS", "BRANCH", "CREATED", "UPDATED", "TITLE"
    ));
    lines.push(
        "-".repeat(id_width + kind_width + status_width + branch_width + date_width * 2 + 14),
    );
    for s in states {
        let created = &s.created_at.format("%Y-%m-%d %H:%M").to_string();
        let updated = s
            .events
            .last()
            .map(|e| e.created_at.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".into());
        lines.push(format!(
            "{:<id_width$}  {:<kind_width$}  {:<status_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
            s.id,
            s.kind.to_string(),
            s.status,
            s.branch.as_deref().unwrap_or("-"),
            created,
            updated,
            s.title,
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::event::{EventType, ThreadKind};
    use crate::internal::node::Node;
    use crate::internal::thread::{NodeLookup, ThreadState};
    use chrono::TimeZone;

    fn fixed_state() -> ThreadState {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: "RFC-0001".into(),
            kind: ThreadKind::Rfc,
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            branch: None,
            status: "draft".into(),
            created_at: t,
            created_by: "human/alice".into(),
            events: vec![Event {
                event_id: "evt-0001".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::Create,
                created_at: t,
                actor: "human/alice".into(),
                base_rev: None,
                parents: vec![],
                title: Some("Test RFC".into()),
                kind: Some(ThreadKind::Rfc),
                body: None,
                node_type: None,
                target_node_id: None,
                new_state: None,
                approvals: vec![],
                evidence: None,
                link_rel: None,
                branch: None,
                incorporated_node_ids: vec![],
                reply_to: None,
            }],
            nodes: vec![],
            evidence_items: vec![],
            links: vec![],
            body_revision_count: 0,
            incorporated_node_ids: vec![],
        }
    }

    #[test]
    fn show_contains_key_fields() {
        let mut state = fixed_state();
        state.branch = Some("feat/solver".into());
        let out = render_show(&state, false);
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("Test RFC"));
        assert!(out.contains("rfc"));
        assert!(out.contains("draft"));
        assert!(out.contains("human/alice"));
        assert!(out.contains("branch:   feat/solver"));
        assert!(out.contains("body:"));
        assert!(out.contains("Thread body"));
        assert!(out.contains("2026-01-01T00:00:00Z"));
        assert!(out.contains("timeline:"));
    }

    #[test]
    fn node_show_contains_body_and_history() {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let lookup = NodeLookup {
            thread_id: "RFC-0001".into(),
            thread_title: "Test RFC".into(),
            thread_kind: ThreadKind::Rfc,
            node: Node {
                node_id: "node-0001".into(),
                node_type: crate::internal::event::NodeType::Question,
                body: "What is this?".into(),
                actor: "human/alice".into(),
                created_at: t,
                resolved: false,
                retracted: false,
                incorporated: false,
                reply_to: None,
            },
            links: vec![crate::internal::thread::ThreadLink {
                target_thread_id: "ISSUE-0001".into(),
                rel: "implements".into(),
            }],
            events: vec![Event {
                event_id: "evt-0002".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::Say,
                created_at: t,
                actor: "human/alice".into(),
                base_rev: None,
                parents: vec![],
                title: None,
                kind: None,
                body: Some("What is this?".into()),
                node_type: Some(crate::internal::event::NodeType::Question),
                target_node_id: Some("node-0001".into()),
                new_state: None,
                approvals: vec![],
                evidence: None,
                link_rel: None,
                branch: None,
                incorporated_node_ids: vec![],
                reply_to: None,
            }],
        };

        let out = render_node_show(&lookup);
        assert!(out.contains("node-0001"));
        assert!(out.contains("RFC-0001 Test RFC"));
        assert!(out.contains("status:   open"));
        assert!(out.contains("body:"));
        assert!(out.contains("What is this?"));
        assert!(out.contains("thread links: 1"));
        assert!(out.contains("ISSUE-0001  implements"));
        assert!(out.contains("history:"));
        assert!(out.contains("question"));
        assert!(out.contains("date"));
        assert!(out.contains("node_id"));
        assert!(out.contains("event_id"));
        assert!(out.contains("evt-0002"));
    }

    #[test]
    fn show_includes_timeline_event_id() {
        let state = fixed_state();
        let out = render_show(&state, false);
        assert!(out.contains("evt-0001"));
    }

    #[test]
    fn show_is_deterministic() {
        let state = fixed_state();
        assert_eq!(render_show(&state, false), render_show(&state, false));
    }

    #[test]
    fn single_line_preview_handles_multibyte_text() {
        let preview =
            single_line_preview("実装開始: CMake + ImGui + GLFW スケルトンアプリの構築", 20);
        assert!(preview.starts_with("実装開始"));
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn show_timeline_always_summarizes_bodies() {
        let mut state = fixed_state();
        let long_body = "First line of a multi-paragraph body\n\nSecond paragraph with more detail that is long enough to exceed the eighty character limit for preview";
        state.events[0].body = Some(long_body.into());
        state.events[0].event_type = EventType::Say;
        state.events[0].node_type = Some(crate::internal::event::NodeType::Claim);
        state.events[0].target_node_id = Some("node-0001".into());
        let out = render_show(&state, false);
        // Timeline always uses single-line preview — long body is truncated
        assert!(out.contains("First line of a multi-paragraph body"));
        assert!(out.contains("..."));
        // Full body text beyond preview is NOT in the timeline
        assert!(!out.contains("eighty character limit for preview"));
    }

    #[test]
    fn show_revise_body_timeline_shows_size_summary() {
        let mut state = fixed_state();
        state.events[0].body = Some("x".repeat(2048));
        state.events[0].event_type = EventType::ReviseBody;
        state.events[0].incorporated_node_ids = vec!["node-001".into(), "node-002".into()];
        let out = render_show(&state, false);
        assert!(out.contains("(2.0 KB, incorporated 2 node(s))"));
        // Full body text is NOT in the timeline
        assert!(!out.contains(&"x".repeat(100)));
    }

    #[test]
    fn show_no_timeline_omits_timeline_section() {
        let state = fixed_state();
        let with_timeline = render_show(&state, false);
        let without_timeline = render_show_with_options(
            &state,
            &ShowOptions {
                compact: false,
                no_timeline: true,
                policy: None,
            },
        );
        assert!(with_timeline.contains("timeline:"));
        assert!(!without_timeline.contains("timeline:"));
        // Non-timeline content is preserved
        assert!(without_timeline.contains("status:"));
        assert!(without_timeline.contains("body:"));
    }

    #[test]
    fn show_with_policy_includes_next_and_diagram() {
        let state = fixed_state();
        let policy = crate::internal::policy::Policy::default();
        let out = render_show_with_options(
            &state,
            &ShowOptions {
                compact: false,
                no_timeline: false,
                policy: Some(policy),
            },
        );
        // RFC in draft has transitions: proposed, rejected
        assert!(out.contains("next:     proposed, rejected"));
        assert!(out.contains("transitions:"));
        // State diagram should show [draft] highlighted
        assert!(out.contains("[draft]"));
        assert!(out.contains("→"));
    }

    #[test]
    fn state_diagram_highlights_current() {
        let lines = render_state_diagram(ThreadKind::Issue, "open");
        let joined = lines.join("\n");
        assert!(joined.contains("[open]"));
        assert!(!joined.contains("[closed]"));
        assert!(joined.contains("closed"));
    }

    #[test]
    fn state_diagram_rfc_all_states_present() {
        let lines = render_state_diagram(ThreadKind::Rfc, "under-review");
        let joined = lines.join("\n");
        assert!(joined.contains("[under-review]"));
        assert!(joined.contains("draft"));
        assert!(joined.contains("proposed"));
        assert!(joined.contains("accepted"));
        assert!(joined.contains("deprecated"));
        assert!(joined.contains("rejected"));
    }

    #[test]
    fn state_diagram_dec() {
        let lines = render_state_diagram(ThreadKind::Dec, "proposed");
        let joined = lines.join("\n");
        assert!(joined.contains("[proposed]"));
        assert!(joined.contains("accepted"));
        assert!(joined.contains("deprecated"));
        assert!(joined.contains("rejected"));
    }

    #[test]
    fn state_diagram_task() {
        let lines = render_state_diagram(ThreadKind::Task, "implementing");
        let joined = lines.join("\n");
        assert!(joined.contains("[implementing]"));
        assert!(joined.contains("open"));
        assert!(joined.contains("designing"));
        assert!(joined.contains("reviewing"));
        assert!(joined.contains("closed"));
        assert!(joined.contains("rejected"));
    }

    #[test]
    fn what_next_includes_operation_checks() {
        let state = fixed_state();
        let policy = crate::internal::policy::Policy {
            node_rules: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "draft".into(),
                    vec![
                        crate::internal::event::NodeType::Claim,
                        crate::internal::event::NodeType::Question,
                    ],
                );
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
        let out = render_what_next(&state, &policy);
        assert!(out.contains("operation checks (state: draft):"));
        assert!(out.contains("node types: claim, question"));
        assert!(out.contains("body revise: allowed"));
        assert!(out.contains("evidence:    allowed"));
    }

    #[test]
    fn ls_empty() {
        assert_eq!(render_ls(&[]), "no threads found\n");
    }

    #[test]
    fn ls_contains_all_threads() {
        let mut s = fixed_state();
        s.branch = Some("feat/parser".into());
        let out = render_ls(&[&s]);
        assert!(out.contains("BRANCH"));
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("rfc"));
        assert!(out.contains("draft"));
        assert!(out.contains("feat/parser"));
        assert!(out.contains("Test RFC"));
    }
}
