//! Top-level list/search renderers (`git forum ls`, `shortlog`, search).
//! Separated from `show.rs` because they don't share the thread-detail
//! view's data model — they format thread/index rows, not replayed state.
//!
//! task `1hg98odf`: the `Ls` arm body relocates from
//! `main.rs` to [`run`] in this module. `render_ls` and the lower-level
//! `list_thread_states` (in `commands::bulk`) are unchanged — the slot
//! is a pure handler relocation since `replay_thread` already reads
//! snapshot tips.
//!
//! Ticket `030xm9s2`: `render_ls` now drops the `TAGS` and `BRANCH`
//! columns when every rendered row has them empty, and accepts an
//! explicit `--columns id,status,title` pin to override the auto-hide
//! rule. The `--branch <B>` filter forces `BRANCH` to render so the
//! caller can verify the filter took effect.

use std::str::FromStr;

use chrono::{DateTime, Utc};

use super::super::error::ForumError;
use super::super::policy;
use super::super::thread::{self, ThreadState};
use super::context::Context;

/// Args for [`run`] — `git forum ls` filters.
pub struct LsArgs {
    pub kind_positional: Option<String>,
    pub branch: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub columns: Option<String>,
}

/// Uniform entry point for the `ls` subcommand.
///
/// Resolves `kind_positional` ↔ `--kind` (rejecting conflicts), filters
/// the replayed thread list, and prints `render_ls` to stdout.
pub fn run(args: LsArgs, ctx: &Context) -> Result<(), ForumError> {
    let effective_kind = match (args.kind_positional.as_deref(), args.kind.as_deref()) {
        (Some(pos), Some(flag)) if pos != flag => {
            return Err(ForumError::Config(format!(
                "conflicting kind: positional '{pos}' vs --kind '{flag}'"
            )));
        }
        (Some(pos), _) => Some(pos),
        (_, Some(flag)) => Some(flag),
        (None, None) => None,
    };
    let kind_filter: Option<&'static str> = effective_kind
        .map(super::shared::parse_thread_kind)
        .transpose()?;
    let states = super::bulk::list_thread_states(&ctx.git, kind_filter, args.branch.as_deref())?;
    let filtered: Vec<&thread::ThreadState> = states
        .iter()
        .filter(|s| {
            args.status
                .as_deref()
                .is_none_or(|st| s.status.as_str() == st)
        })
        .collect();
    let columns = match args.columns.as_deref() {
        Some(spec) => Some(parse_columns(spec)?),
        None => None,
    };
    let opts = LsRenderOptions {
        force_branch_column: args.branch.is_some(),
        columns,
    };
    print!("{}", render_ls(&filtered, &opts));
    Ok(())
}

// task `913c4s9v`: the
// `render_search_results` renderer was deleted alongside the
// `internal::index::SearchRow` type. Search support relied on the
// SQLite index (task `1hg98odf` dropped the `Search` arm; task
// `913c4s9v` removes the index module itself). Re-introducing search is a
// v3.1 concern — see the RFC Exceptions section.

/// Columns rendered by `git forum ls` (ticket `030xm9s2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Id,
    Lifecycle,
    Status,
    Tags,
    Branch,
    Created,
    Updated,
    Title,
}

impl Column {
    fn header(self) -> &'static str {
        match self {
            Column::Id => "ID",
            Column::Lifecycle => "LIFECYCLE",
            Column::Status => "STATUS",
            Column::Tags => "TAGS",
            Column::Branch => "BRANCH",
            Column::Created => "CREATED",
            Column::Updated => "UPDATED",
            Column::Title => "TITLE",
        }
    }
}

impl FromStr for Column {
    type Err = ForumError;

    fn from_str(s: &str) -> Result<Self, ForumError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "id" => Ok(Column::Id),
            "lifecycle" => Ok(Column::Lifecycle),
            "status" => Ok(Column::Status),
            "tags" => Ok(Column::Tags),
            "branch" => Ok(Column::Branch),
            "created" => Ok(Column::Created),
            "updated" => Ok(Column::Updated),
            "title" => Ok(Column::Title),
            other => Err(ForumError::Config(format!(
                "unknown ls column '{other}'; valid: id, lifecycle, status, tags, branch, created, updated, title"
            ))),
        }
    }
}

fn parse_columns(spec: &str) -> Result<Vec<Column>, ForumError> {
    let cols: Vec<Column> = spec
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Column::from_str)
        .collect::<Result<_, _>>()?;
    if cols.is_empty() {
        return Err(ForumError::Config(
            "--columns must list at least one column".into(),
        ));
    }
    Ok(cols)
}

/// Caller-controlled rendering options for `render_ls` (ticket `030xm9s2`).
#[derive(Debug, Default, Clone)]
pub struct LsRenderOptions {
    /// True when `--branch <B>` filter was used: forces the BRANCH column
    /// to stay even if every rendered row shares the same branch
    /// (the filter is what's keeping the column meaningful).
    pub force_branch_column: bool,
    /// Caller-pinned column set; `None` means "auto-decide". When
    /// `Some(...)`, the listed columns are rendered in order and the
    /// auto-hide rule is bypassed entirely.
    pub columns: Option<Vec<Column>>,
}

const DEFAULT_COLUMNS: &[Column] = &[
    Column::Id,
    Column::Lifecycle,
    Column::Status,
    Column::Tags,
    Column::Branch,
    Column::Created,
    Column::Updated,
    Column::Title,
];

/// Render `git forum ls` output for a list of threads.
///
/// SPEC-2.0 classification: classification axes are LIFECYCLE + TAGS, not
/// KIND. Default columns: ID, LIFECYCLE, STATUS, TAGS, BRANCH, CREATED,
/// UPDATED, TITLE. Per ticket `030xm9s2`, TAGS and BRANCH are dropped
/// when uniformly empty across the rendered rows; `opts.columns`
/// overrides the auto-hide rule entirely; `opts.force_branch_column`
/// keeps BRANCH even when uniformly empty (used when `--branch` filter
/// narrowed the rows). Width is recomputed on each call.
pub fn render_ls(states: &[&ThreadState], opts: &LsRenderOptions) -> String {
    if states.is_empty() {
        return "no threads found\n".into();
    }
    let columns = effective_columns(states, opts);
    let widths: Vec<usize> = columns.iter().map(|&c| column_width(c, states)).collect();
    // Match the legacy ruler width: column widths + 2 chars between every
    // adjacent pair + 2 chars of trailing slack (preserves the
    // ls_two_threads snapshot byte-for-byte when both TAGS and BRANCH
    // are present).
    let fixed_cols: usize = widths.iter().sum::<usize>() + spacing_for_columns(&columns) + 2;
    let title_max = title_max_for(fixed_cols);

    let mut lines: Vec<String> = Vec::new();
    lines.push(format_row(&columns, &widths, |c, _| c.header().to_string()));
    lines.push("-".repeat(fixed_cols));
    for s in states {
        lines.push(format_row(&columns, &widths, |c, w| {
            let max = if matches!(c, Column::Title) {
                title_max
            } else {
                w
            };
            cell_value(c, s, max)
        }));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Decide which columns to render. `opts.columns` short-circuits to the
/// caller's explicit pin; otherwise we drop columns whose underlying
/// state value is uniformly empty across all rows (currently TAGS and
/// BRANCH; the rest carry information for every row).
fn effective_columns(states: &[&ThreadState], opts: &LsRenderOptions) -> Vec<Column> {
    if let Some(pinned) = opts.columns.as_ref() {
        return pinned.clone();
    }
    DEFAULT_COLUMNS
        .iter()
        .copied()
        .filter(|&c| !auto_hide(c, states, opts))
        .collect()
}

/// Per ticket `030xm9s2`: TAGS and BRANCH auto-hide when every row's
/// underlying value is empty. Other columns never auto-hide. The
/// `--branch <B>` filter pins BRANCH even when uniformly empty.
fn auto_hide(column: Column, states: &[&ThreadState], opts: &LsRenderOptions) -> bool {
    match column {
        Column::Tags => states.iter().all(|s| s.tags.is_empty()),
        Column::Branch => {
            if opts.force_branch_column {
                return false;
            }
            states.iter().all(|s| s.branch.is_none())
        }
        _ => false,
    }
}

fn cell_value(column: Column, s: &ThreadState, title_max: usize) -> String {
    match column {
        Column::Id => s.id.clone(),
        Column::Lifecycle => policy::lifecycle_label_for(&s.category, &s.tags).to_string(),
        Column::Status => s.status.clone(),
        Column::Tags => join_tags(&s.tags),
        Column::Branch => s.branch.as_deref().unwrap_or("-").to_string(),
        Column::Created => s.created_at.format("%Y-%m-%d %H:%M").to_string(),
        Column::Updated => s.updated_at.format("%Y-%m-%d %H:%M").to_string(),
        Column::Title => truncate_with_ellipsis(&s.title, title_max),
    }
}

fn column_width(column: Column, states: &[&ThreadState]) -> usize {
    match column {
        Column::Id => states
            .iter()
            .map(|s| s.id.len())
            .max()
            .unwrap_or(12)
            .clamp(12, 20),
        Column::Lifecycle => states
            .iter()
            .map(|s| policy::lifecycle_label_for(&s.category, &s.tags).len())
            .max()
            .unwrap_or(10)
            .clamp(10, 12),
        Column::Status => states
            .iter()
            .map(|s| s.status.len())
            .max()
            .unwrap_or(14)
            .clamp(10, 16),
        Column::Tags => states
            .iter()
            .map(|s| join_tags(&s.tags).len())
            .max()
            .unwrap_or(8)
            .clamp(8, 30),
        Column::Branch => states
            .iter()
            .map(|s| s.branch.as_deref().unwrap_or("-").len())
            .max()
            .unwrap_or(12)
            .clamp(12, 30),
        Column::Created | Column::Updated => 16,
        // TITLE is rendered last and consumes the remainder; column_width
        // is only consulted for layout of the leading columns, so 0 is
        // fine here.
        Column::Title => 0,
    }
}

/// Two spaces between every adjacent pair of rendered columns (matches
/// the legacy fixed format).
fn spacing_for_columns(columns: &[Column]) -> usize {
    columns.len().saturating_sub(1) * 2
}

/// Format one row by walking the column list with a per-cell value
/// resolver. The final column is rendered without a trailing pad so
/// long titles aren't padded to a fixed width.
fn format_row(
    columns: &[Column],
    widths: &[usize],
    mut resolve: impl FnMut(Column, usize) -> String,
) -> String {
    let mut out = String::new();
    let last = columns.len().saturating_sub(1);
    for (i, (&col, &w)) in columns.iter().zip(widths.iter()).enumerate() {
        let value = resolve(col, w);
        if i == last {
            out.push_str(&value);
        } else if matches!(col, Column::Title) {
            // Title in non-final position — pad to fit (rare; only when
            // the operator pins TITLE somewhere other than the end).
            out.push_str(&format!("{:<w$}  ", value, w = w.max(value.len())));
        } else {
            out.push_str(&format!("{:<w$}  ", value, w = w));
        }
    }
    out
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
    // Group by lifecycle label (proposal/execution/record), not by category.
    // task `1v400j3l`: lifecycle is now a derived display label rather than
    // a typed enum, computed from category+tags via `policy::lifecycle_label_for`.
    let lifecycle_order = ["proposal", "execution", "record"];
    let mut lines: Vec<String> = Vec::new();
    for lifecycle in &lifecycle_order {
        let mut group: Vec<(&ThreadState, DateTime<Utc>)> = entries
            .iter()
            .filter(|(s, _)| policy::lifecycle_label_for(&s.category, &s.tags) == *lifecycle)
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
            .map(|(s, _)| s.status.len())
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

// task `913c4s9v`: `preview_one_line`
// was the body-preview helper consumed solely by the now-deleted
// `render_search_results` renderer. Removed alongside.

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn fixed_state() -> ThreadState {
        ThreadState {
            id: "RFC-0001".into(),
            title: "Test RFC".into(),
            body: Some("Thread body".into()),
            status: "draft".into(),
            created_at: t(),
            created_by: "human/alice".into(),
            category: "rfc".into(),
            ..ThreadState::default()
        }
    }

    #[test]
    fn ls_empty() {
        assert_eq!(
            render_ls(&[], &LsRenderOptions::default()),
            "no threads found\n"
        );
    }

    #[test]
    fn ls_contains_all_threads() {
        let mut s = fixed_state();
        s.branch = Some("feat/parser".into());
        s.category = "rfc".into();
        let out = render_ls(&[&s], &LsRenderOptions::default());
        assert!(out.contains("LIFECYCLE"));
        // TAGS is uniformly empty for this single row → auto-hidden.
        assert!(!out.contains("TAGS"));
        assert!(out.contains("BRANCH"));
        assert!(out.contains("RFC-0001"));
        assert!(out.contains("proposal"));
        assert!(out.contains("draft"));
        assert!(out.contains("feat/parser"));
        assert!(out.contains("Test RFC"));
    }

    #[test]
    fn ls_drops_uniformly_empty_branch_column() {
        // No row has a bound branch → BRANCH auto-hidden.
        let s = fixed_state();
        let out = render_ls(&[&s], &LsRenderOptions::default());
        assert!(
            !out.contains("BRANCH"),
            "BRANCH column should auto-hide when uniformly empty:\n{out}"
        );
    }

    #[test]
    fn ls_keeps_branch_column_when_filter_active() {
        // Even though no row has a branch, --branch <B> filter forces the
        // column to render so the operator can confirm the filter.
        let s = fixed_state();
        let opts = LsRenderOptions {
            force_branch_column: true,
            ..LsRenderOptions::default()
        };
        let out = render_ls(&[&s], &opts);
        assert!(
            out.contains("BRANCH"),
            "BRANCH column must stay when --branch filter is active:\n{out}"
        );
    }

    #[test]
    fn ls_columns_pin_overrides_auto_hide() {
        // Operator explicitly pins BRANCH → render it even though every
        // row has it empty (and --branch filter is NOT active).
        let s = fixed_state();
        let opts = LsRenderOptions {
            columns: Some(vec![Column::Id, Column::Branch, Column::Title]),
            ..LsRenderOptions::default()
        };
        let out = render_ls(&[&s], &opts);
        assert!(out.contains("ID"));
        assert!(out.contains("BRANCH"));
        assert!(out.contains("TITLE"));
        // Pinned set excludes LIFECYCLE → must NOT appear.
        assert!(!out.contains("LIFECYCLE"));
        assert!(!out.contains("STATUS"));
    }

    #[test]
    fn parse_columns_round_trip() {
        let cols = parse_columns("id,branch,title").unwrap();
        assert_eq!(cols, vec![Column::Id, Column::Branch, Column::Title]);
        // Whitespace and case tolerance.
        let cols = parse_columns(" ID , Branch ").unwrap();
        assert_eq!(cols, vec![Column::Id, Column::Branch]);
    }

    #[test]
    fn parse_columns_rejects_unknown() {
        let err = parse_columns("id,bogus,title").unwrap_err();
        assert!(format!("{err}").contains("unknown ls column 'bogus'"));
    }

    #[test]
    fn parse_columns_rejects_empty_spec() {
        let err = parse_columns("").unwrap_err();
        assert!(format!("{err}").contains("at least one column"));
    }

    #[test]
    fn ls_keeps_branch_when_one_row_has_it() {
        // BRANCH not uniformly empty (one row has it set) → keep.
        let s1 = fixed_state();
        let mut s2 = fixed_state();
        s2.id = "RFC-0002".into();
        s2.branch = Some("feat/auth".into());
        let out = render_ls(&[&s1, &s2], &LsRenderOptions::default());
        assert!(out.contains("BRANCH"));
        assert!(out.contains("feat/auth"));
    }
}
