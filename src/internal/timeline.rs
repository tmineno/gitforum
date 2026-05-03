//! Single timeline renderer (markdown table). Used by `git forum show`,
//! `git forum log`, and `git forum node show` so all three share one
//! rendering pass — see RFC-lmr3wfcm Track E AC for the consolidation.

use super::commands::show::short_oid;
use super::event::{Event, EventType};

/// Render a markdown table for the given events.
pub fn render_markdown(events: &[Event]) -> Vec<String> {
    render_markdown_refs(&events.iter().collect::<Vec<_>>())
}

/// Slice-of-references variant for the `log` command, where filtered events
/// are collected as `&[&Event]`.
pub fn render_markdown_refs(events: &[&Event]) -> Vec<String> {
    let mut lines = Vec::with_capacity(events.len() + 2);
    lines.push("| date | node_id | event_id | author | type | body |".into());
    lines.push("|------|---------|----------|--------|------|------|".into());
    for event in events {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            event.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
            event_node_id(event).map(short_oid).unwrap_or("-"),
            short_oid(&event.event_id),
            event.actor,
            event_display_type(event),
            timeline_body(event),
        ));
    }
    lines
}

/// Display label for an event in a timeline row. `Say` shows the node-type
/// label; `ReviseBody` shows `revise-body`; everything else uses the event
/// type's display name. Public because `git forum log --type` filters on this.
pub fn event_display_type(event: &Event) -> String {
    match event.event_type {
        EventType::Say => event
            .node_type
            .map(|t| t.to_string())
            .unwrap_or_else(|| event.event_type.to_string()),
        EventType::ReviseBody => "revise-body".to_string(),
        _ => event.event_type.to_string(),
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

fn timeline_body(event: &Event) -> String {
    if event.event_type == EventType::ReviseBody {
        return revise_body_summary(event);
    }
    single_line_preview(&event_detail(event), 80)
}

fn revise_body_summary(event: &Event) -> String {
    let body = event.body.as_deref().unwrap_or("");
    let inc = event.incorporated_node_ids.len();
    if inc > 0 {
        format!("({}, incorporated {inc} node(s))", size_summary(body.len()))
    } else {
        format!("({})", size_summary(body.len()))
    }
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
        EventType::Retype => match (event.old_node_type, event.node_type) {
            (Some(old), Some(new)) => format!("{old} -> {new}"),
            (None, Some(new)) => format!("-> {new}"),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn single_line_preview(s: &str, max: usize) -> String {
    let joined = s.lines().collect::<Vec<_>>().join(" / ");
    if joined.chars().count() <= max {
        joined
    } else {
        format!("{}...", joined.chars().take(max).collect::<String>())
    }
}

fn size_summary(size: usize) -> String {
    if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}
