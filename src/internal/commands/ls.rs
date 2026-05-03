//! Top-level list/search renderers (`git forum ls`, `shortlog`, search).
//! Separated from `show.rs` because they don't share the thread-detail
//! view's data model — they format thread/index rows, not replayed state.

use chrono::{DateTime, Utc};

use super::super::event::Lifecycle;
use super::super::index::SearchRow;
use super::super::thread::ThreadState;
use super::show::short_oid;

/// Render search results from the local index.
///
/// Phase 3: lifecycle and tags are real index columns; surface them as
/// the canonical 2.0 axes here too, mirroring `render_ls`.
pub fn render_search_results(rows: &[SearchRow]) -> String {
    if rows.is_empty() {
        return "no threads found\n".into();
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<12}  {:<10}  {:<14}  {:<14}  {}",
        "ID", "LIFECYCLE", "STATUS", "TAGS", "TITLE"
    ));
    lines.push("-".repeat(72));
    for r in rows {
        let tags = if r.thread.tags.is_empty() {
            "-".to_string()
        } else {
            r.thread.tags.join(",")
        };
        lines.push(format!(
            "{:<12}  {:<10}  {:<14}  {:<14}  {}",
            r.thread.id, r.thread.lifecycle, r.thread.status, tags, r.thread.title
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
/// Phase 2b: classification axes are LIFECYCLE + TAGS, not KIND. Output
/// columns: ID, LIFECYCLE, STATUS, TAGS, BRANCH, CREATED, UPDATED, TITLE.
/// Deterministic when thread IDs, statuses, and tag insertion order are
/// deterministic.
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
    let lifecycle_width = states
        .iter()
        .map(|s| s.lifecycle.as_str().len())
        .max()
        .unwrap_or(10)
        .clamp(10, 12);
    let status_width = states
        .iter()
        .map(|s| s.status.as_str().len())
        .max()
        .unwrap_or(14)
        .clamp(10, 16);
    let tags_width = states
        .iter()
        .map(|s| join_tags(&s.tags).len())
        .max()
        .unwrap_or(8)
        .clamp(8, 30);
    let branch_width = states
        .iter()
        .map(|s| s.branch.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(12)
        .clamp(12, 30);
    let date_width = 16;
    let fixed_cols =
        id_width + lifecycle_width + status_width + tags_width + branch_width + date_width * 2 + 16;
    let title_max = title_max_for(fixed_cols);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{:<id_width$}  {:<lifecycle_width$}  {:<status_width$}  {:<tags_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
        "ID", "LIFECYCLE", "STATUS", "TAGS", "BRANCH", "CREATED", "UPDATED", "TITLE"
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
        let tags = join_tags(&s.tags);
        lines.push(format!(
            "{:<id_width$}  {:<lifecycle_width$}  {:<status_width$}  {:<tags_width$}  {:<branch_width$}  {:<date_width$}  {:<date_width$}  {}",
            s.id,
            s.lifecycle.as_str(),
            s.status,
            truncate_with_ellipsis(&tags, tags_width),
            s.branch.as_deref().unwrap_or("-"),
            created,
            updated,
            title,
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Render a thread's tag list for column display: comma-joined or `-`.
fn join_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "-".into()
    } else {
        tags.join(",")
    }
}

pub fn render_shortlog(entries: &[(&ThreadState, DateTime<Utc>)]) -> String {
    if entries.is_empty() {
        return "no threads reached terminal state in the given period\n".into();
    }
    // Phase 2b: group by lifecycle, not kind. The three lifecycles are
    // listed in spec-canonical order (proposal -> execution -> record).
    let lifecycle_order = [Lifecycle::Proposal, Lifecycle::Execution, Lifecycle::Record];
    let mut lines: Vec<String> = Vec::new();
    for lifecycle in &lifecycle_order {
        let mut group: Vec<(&ThreadState, DateTime<Utc>)> = entries
            .iter()
            .filter(|(s, _)| s.lifecycle == *lifecycle)
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
        lines.push(format!("## {} ({count} {thread_word})", lifecycle));

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
    use crate::internal::event::{Event, EventType, Lifecycle, ThreadKind, ThreadStatus};
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
        // Phase 2b: lifecycle replaces kind as the column.
        let mut s = fixed_state();
        s.branch = Some("feat/parser".into());
        s.lifecycle = Lifecycle::Proposal; // kind=Rfc would derive this anyway
        let out = render_ls(&[&s]);
        assert!(out.contains("LIFECYCLE"));
        assert!(out.contains("TAGS"));
        assert!(out.contains("BRANCH"));
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("proposal"));
        assert!(out.contains("draft"));
        assert!(out.contains("feat/parser"));
        assert!(out.contains("Test RFC"));
    }
}
