//! Top-level list/search renderers (`git forum ls`, `shortlog`, search).
//! Separated from `show.rs` because they don't share the thread-detail
//! view's data model — they format thread/index rows, not replayed state.

use chrono::{DateTime, Utc};

use super::event::ThreadKind;
use super::index::SearchRow;
use super::show::short_oid;
use super::thread::ThreadState;

/// Render search results from the local index.
pub fn render_search_results(rows: &[SearchRow]) -> String {
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
                preview_one_line(&hit.body, 60),
            ));
        }
    }
    lines.push(String::new());
    lines.join("\n")
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
        .clamp(12, 20);
    let kind_width = states
        .iter()
        .map(|s| s.kind.to_string().len())
        .max()
        .unwrap_or(10)
        .clamp(10, 16);
    let status_width = states
        .iter()
        .map(|s| s.status.as_str().len())
        .max()
        .unwrap_or(14)
        .clamp(14, 20);
    let branch_width = states
        .iter()
        .map(|s| s.branch.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(12)
        .clamp(12, 30);
    let date_width = 16;
    let fixed_cols = id_width + kind_width + status_width + branch_width + date_width * 2 + 14;
    let title_max = title_max_for(fixed_cols);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<id_width$}  {:<kind_width$}  {:<status_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
        "ID", "KIND", "STATUS", "BRANCH", "CREATED", "UPDATED", "TITLE"
    ));
    lines.push("-".repeat(fixed_cols));
    for s in states {
        let created = s.created_at.format("%Y-%m-%d %H:%M").to_string();
        let updated = s
            .events
            .last()
            .map(|e| e.created_at.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".into());
        let title = truncate_with_ellipsis(&s.title, title_max);
        lines.push(format!(
            "{:<id_width$}  {:<kind_width$}  {:<status_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
            s.id,
            s.kind.to_string(),
            s.status,
            s.branch.as_deref().unwrap_or("-"),
            created,
            updated,
            title,
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

pub fn render_shortlog(entries: &[(&ThreadState, DateTime<Utc>)]) -> String {
    if entries.is_empty() {
        return "no threads reached terminal state in the given period\n".into();
    }
    let kind_order = [
        ThreadKind::Issue,
        ThreadKind::Rfc,
        ThreadKind::Dec,
        ThreadKind::Task,
    ];
    let mut lines: Vec<String> = Vec::new();
    for kind in &kind_order {
        let mut group: Vec<(&ThreadState, DateTime<Utc>)> = entries
            .iter()
            .filter(|(s, _)| s.kind == *kind)
            .copied()
            .collect();
        if group.is_empty() {
            continue;
        }
        group.sort_by_key(|(_, dt)| *dt);

        let count = group.len();
        let thread_word = if count == 1 { "thread" } else { "threads" };
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("## {} ({count} {thread_word})", kind));

        let id_width = group
            .iter()
            .map(|(s, _)| s.id.len())
            .max()
            .unwrap_or(12)
            .clamp(12, 20);
        let status_width = group
            .iter()
            .map(|(s, _)| s.status.as_str().len())
            .max()
            .unwrap_or(10)
            .clamp(10, 16);
        let date_width = 16;
        let fixed_cols = id_width + status_width + date_width + 8;
        let title_max = title_max_for(fixed_cols);

        lines.push(format!(
            "{:<id_width$}  {:<status_width$}  {:<date_width$}  {}",
            "ID", "STATUS", "RESOLVED", "TITLE"
        ));
        for (state, term_date) in &group {
            let resolved = term_date.format("%Y-%m-%d %H:%M").to_string();
            let title = truncate_with_ellipsis(&state.title, title_max);
            lines.push(format!(
                "{:<id_width$}  {:<status_width$}  {:<date_width$}  {}",
                state.id, state.status, resolved, title,
            ));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Available width for the title column, given the fixed columns. Returns 0
/// when output is piped (non-TTY) or terminal width is < 40 — by design
/// piped output is lossless for downstream processing.
fn title_max_for(fixed_cols: usize) -> usize {
    let term_width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .ok()
        .filter(|&w| w >= 40)
        .unwrap_or(0);
    term_width.saturating_sub(fixed_cols)
}

fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    if max == 0 || s.len() <= max {
        return s.to_string();
    }
    let end = s.floor_char_boundary(max.saturating_sub(3));
    format!("{}...", &s[..end])
}

fn preview_one_line(s: &str, max: usize) -> String {
    let joined = s.lines().collect::<Vec<_>>().join(" / ");
    if joined.chars().count() <= max {
        joined
    } else {
        format!("{}...", joined.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::event::{Event, EventType, ThreadKind, ThreadStatus};
    use chrono::TimeZone;

    fn t() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn fixed_state() -> ThreadState {
        ThreadState {
            id: "RFC-0001".into(),
            kind: ThreadKind::Rfc,
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            status: ThreadStatus::Draft,
            created_at: t(),
            created_by: "human/alice".into(),
            events: vec![Event {
                event_id: "evt-0001".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::Create,
                created_at: t(),
                actor: "human/alice".into(),
                title: Some("Test RFC".into()),
                kind: Some(ThreadKind::Rfc),
                ..Event::default()
            }],
            ..ThreadState::default()
        }
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
