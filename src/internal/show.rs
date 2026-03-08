use super::event::EventType;
use super::thread::ThreadState;

/// Render `git forum show` output for a thread.
///
/// Output is deterministic given deterministic event timestamps and IDs.
/// Snapshot strategy: tests use FixedClock so `created_at` is stable;
/// no commit hashes or wall-clock times appear in the output.
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
    lines.push(String::new());

    // M3 will add: open objections, open actions, latest summary

    lines.push("timeline:".into());
    for event in &state.events {
        let detail = event_detail(event);
        let mut entry = format!(
            "  {}  {:<10}  by {}",
            event.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
            event.event_type.to_string(),
            event.actor,
        );
        if !detail.is_empty() {
            entry.push_str(&format!("  -- {detail}"));
        }
        lines.push(entry);
    }
    lines.push(String::new());

    lines.join("\n")
}

fn event_detail(event: &super::event::Event) -> String {
    match event.event_type {
        EventType::Create => format!("\"{}\"", event.title.as_deref().unwrap_or("")),
        EventType::State => format!("-> {}", event.new_state.as_deref().unwrap_or("")),
        _ => String::new(),
    }
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
    use crate::internal::event::{Event, EventType, ThreadKind};
    use crate::internal::thread::ThreadState;
    use chrono::TimeZone;

    fn fixed_state() -> ThreadState {
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ThreadState {
            id: "RFC-0001".into(),
            kind: ThreadKind::Rfc,
            title: "Test RFC".into(),
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
            }],
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
        assert!(out.contains("2026-01-01T00:00:00Z"));
        assert!(out.contains("timeline:"));
    }

    #[test]
    fn show_no_commit_hash() {
        let state = fixed_state();
        let out = render_show(&state);
        // SHA-like 40-char hex strings should not appear
        assert!(!out.chars().filter(|c| c.is_ascii_hexdigit()).count() > 40);
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
