use super::event::{Event, EventType};
use super::run::Run;
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

    if !state.run_labels.is_empty() {
        lines.push(format!("runs: {}", state.run_labels.len()));
        for label in &state.run_labels {
            lines.push(format!("  - {label}"));
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
    lines.push("body:".into());
    for line in node.body.lines() {
        lines.push(format!("  {line}"));
    }
    if node.body.is_empty() {
        lines.push("  ".into());
    }
    lines.push(String::new());

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
        EventType::Say | EventType::Edit => event.body.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn node_status(node: &super::node::Node) -> &'static str {
    if node.retracted {
        "retracted"
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

fn truncate_body(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

fn single_line_preview(s: &str, max: usize) -> String {
    let joined = s.lines().collect::<Vec<_>>().join(" / ");
    truncate_body(&joined, max)
}

/// Render `git forum run show` output for a single run.
pub fn render_run_show(run: &Run) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{:<12} {}", run.run_label, run.status));
    lines.push(format!("thread:   {}", run.thread_id));
    lines.push(format!("by:       {}", run.actor_id));
    lines.push(format!(
        "started:  {}",
        run.started_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    if let Some(ended) = run.ended_at {
        lines.push(format!("ended:    {}", ended.format("%Y-%m-%dT%H:%M:%SZ")));
    }
    if let Some(model) = &run.model {
        lines.push(format!("model:    {} / {}", model.provider, model.name));
    }
    if let Some(result) = &run.result {
        lines.push(format!("result:   {}", result.status));
        if let Some(conf) = result.confidence {
            lines.push(format!("confidence: {conf:.2}"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Render `git forum run ls` output for a list of runs.
pub fn render_run_ls(runs: &[Run]) -> String {
    if runs.is_empty() {
        return "no runs found\n".into();
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<12}  {:<12}  {:<10}  {}",
        "LABEL", "THREAD", "STATUS", "STARTED"
    ));
    lines.push("-".repeat(60));
    for run in runs {
        lines.push(format!(
            "{:<12}  {:<12}  {:<10}  {}",
            run.run_label,
            run.thread_id,
            run.status.to_string(),
            run.started_at.format("%Y-%m-%dT%H:%M:%SZ"),
        ));
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
/// Output columns: ID, KIND, STATUS, TITLE.
/// Deterministic when thread IDs and statuses are deterministic.
pub fn render_ls(states: &[&ThreadState]) -> String {
    if states.is_empty() {
        return "no threads found\n".into();
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<12}  {:<10}  {:<14}  {}",
        "ID", "KIND", "STATUS", "TITLE"
    ));
    lines.push("-".repeat(60));
    for s in states {
        lines.push(format!(
            "{:<12}  {:<10}  {:<14}  {}",
            s.id,
            s.kind.to_string(),
            s.status,
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
                run_label: None,
            }],
            nodes: vec![],
            evidence_items: vec![],
            links: vec![],
            run_labels: vec![],
        }
    }

    #[test]
    fn show_contains_key_fields() {
        let state = fixed_state();
        let out = render_show(&state);
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("Test RFC"));
        assert!(out.contains("rfc"));
        assert!(out.contains("draft"));
        assert!(out.contains("human/alice"));
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
            },
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
                run_label: None,
            }],
        };

        let out = render_node_show(&lookup);
        assert!(out.contains("node-0001"));
        assert!(out.contains("RFC-0001 Test RFC"));
        assert!(out.contains("status:   open"));
        assert!(out.contains("body:"));
        assert!(out.contains("What is this?"));
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
    fn ls_empty() {
        assert_eq!(render_ls(&[]), "no threads found\n");
    }

    #[test]
    fn ls_contains_all_threads() {
        let s = fixed_state();
        let out = render_ls(&[&s]);
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("rfc"));
        assert!(out.contains("draft"));
        assert!(out.contains("Test RFC"));
    }
}
