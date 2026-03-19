use super::event::{Event, EventType};
use super::policy::{self, Policy};
use super::state_machine;
use super::thread::{NodeLookup, ThreadState};

/// Render `git forum show` output for a thread.
///
/// Output is deterministic given deterministic event timestamps and IDs.
/// Snapshot strategy: tests use fixed synthetic events where needed;
/// integration tests should avoid asserting exact Git OIDs.
pub fn render_show(state: &ThreadState) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("{:<12} {}", state.id, state.title));
    lines.push(format!("kind:     {}", state.kind));
    lines.push(format!("status:   {}", state.status));
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
    if !incorporated.is_empty() {
        lines.push(format!("incorporated nodes: {}", incorporated.len()));
        for node in &incorporated {
            let preview = truncate_body(&node.body, 60);
            lines.push(format!(
                "  - {} {} {}",
                short_oid(&node.node_id),
                node.node_type,
                preview
            ));
        }
        lines.push(String::new());
    }

    let open_obj = state.open_objections();
    if !open_obj.is_empty() {
        lines.push(format!("open objections: {}", open_obj.len()));
        for node in &open_obj {
            let preview = truncate_body(&node.body, 60);
            lines.push(format!("  - {} {}", short_oid(&node.node_id), preview));
        }
        lines.push(String::new());
    }

    let open_act = state.open_actions();
    if !open_act.is_empty() {
        lines.push(format!("open actions: {}", open_act.len()));
        for node in &open_act {
            let preview = truncate_body(&node.body, 60);
            lines.push(format!("  - {} {}", short_oid(&node.node_id), preview));
        }
        lines.push(String::new());
    }

    if let Some(summary) = state.latest_summary() {
        lines.push("latest summary:".into());
        lines.push(format!("  {}", summary.body));
        lines.push(String::new());
    }

    if !state.evidence_items.is_empty() {
        lines.push(format!("evidence: {}", state.evidence_items.len()));
        for ev in &state.evidence_items {
            let id_short = &ev.evidence_id[..ev.evidence_id.len().min(8)];
            lines.push(format!("  - {}  {}  {}", id_short, ev.kind, ev.ref_target));
        }
        lines.push(String::new());
    }

    if !state.links.is_empty() {
        lines.push(format!("links: {}", state.links.len()));
        for link in &state.links {
            lines.push(format!("  - {}  {}", link.target_thread_id, link.rel));
        }
        lines.push(String::new());
    }

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
                truncate_body(&root.body, 50)
            ));
            for reply in &convo[1..] {
                lines.push(format!(
                    "    -> {} {} {} {}",
                    short_oid(&reply.node_id),
                    reply.node_type,
                    reply.actor,
                    truncate_body(&reply.body, 50)
                ));
            }
        }
        lines.push(String::new());
    }

    lines.push("timeline:".into());
    let widths = timeline_widths(&state.events);
    lines.push(format_timeline_header(&widths));
    for event in &state.events {
        lines.push(format_timeline_entry(event, &widths));
    }
    lines.push(String::new());

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
        lines.push(format_timeline_entry(event, &widths));
    }
    lines.push(String::new());

    lines.join("\n")
}

fn event_detail(event: &Event) -> String {
    match event.event_type {
        EventType::Create => event.title.clone().unwrap_or_default(),
        EventType::State => event.new_state.clone().unwrap_or_default(),
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

fn timeline_body(event: &Event) -> String {
    single_line_preview(&event_detail(event), 80)
}

struct TimelineWidths {
    date: usize,
    node_id: usize,
    event_id: usize,
    author: usize,
    r#type: usize,
}

fn timeline_widths(events: &[Event]) -> TimelineWidths {
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

fn format_timeline_header(widths: &TimelineWidths) -> String {
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

fn format_timeline_entry(event: &Event, widths: &TimelineWidths) -> String {
    format!(
        "  {:<date$}  {:<node_id$}  {:<event_id$}  {:<author$}  {:<type$}  {}",
        event.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
        event_node_id(event).map(short_oid).unwrap_or("-"),
        short_oid(&event.event_id),
        event.actor,
        event_display_type(event),
        timeline_body(event),
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

/// Render `git forum status --all` output across multiple threads.
pub fn render_status_all(states: &[&ThreadState]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut any_open = false;
    for state in states {
        let text = render_status(state);
        if !text.contains("no open items") {
            lines.push(text);
            any_open = true;
        }
    }
    if !any_open {
        return "no open items across any thread\n".into();
    }
    lines.join("")
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
        let out = render_show(&state);
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
        let out = render_show(&state);
        assert!(out.contains("evt-0001"));
    }

    #[test]
    fn show_is_deterministic() {
        let state = fixed_state();
        assert_eq!(render_show(&state), render_show(&state));
    }

    #[test]
    fn single_line_preview_handles_multibyte_text() {
        let preview =
            single_line_preview("実装開始: CMake + ImGui + GLFW スケルトンアプリの構築", 20);
        assert!(preview.starts_with("実装開始"));
        assert!(preview.ends_with("..."));
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
