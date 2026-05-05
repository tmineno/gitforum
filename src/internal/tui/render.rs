use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::internal::id;
use crate::internal::node::Node;
use crate::internal::snapshot::list::ThreadRow;
use crate::internal::thread::ThreadKind;

use super::markdown::markdown_to_text;
use super::state::{
    auto_link_candidates, link_relation_labels, link_target_kind_labels, link_target_kind_values,
    node_type_labels, selected_link_target_label, thread_lifecycle_labels,
};
use super::{
    App, ErrorFlash, FilterField, LinkFormField, LinkTargetKind, NodeFormField, ThreadFormField,
    UiRects, View, FILTER_LIFECYCLE_LABELS, FILTER_STATUS_LABELS, SORT_COLUMNS,
};

/// Render the current app state into `frame`.
pub(super) fn render(f: &mut Frame, app: &mut App) {
    app.ui_rects = UiRects::default();

    // In select mode, show only the relevant text full-screen for clean selection
    if app.mouse_capture_disabled {
        render_select_mode(f, f.area(), app);
        return;
    }

    match app.view {
        View::List => {
            let area = f.area();
            render_list(f, area, app);
            if app.filter_bar.is_some() {
                render_filter_bar(f, area, app);
            }
        }
        View::ThreadDetail(_) => render_thread_detail(f, f.area(), app),
        View::NodeDetail { .. } => render_node_detail(f, f.area(), app),
        View::CreateThread => render_create_thread(f, f.area(), app),
        View::EditThreadBody => render_edit_thread_body(f, f.area(), app),
        View::CreateNode { .. } => render_create_node(f, f.area(), app),
        View::EditNodeBody { .. } => render_edit_node_body(f, f.area(), app),
        View::CreateLink { .. } => render_create_link(f, f.area(), app),
    }

    // Discard confirmation overlay
    if app.confirm_discard {
        render_confirm_discard(f, f.area());
    }

    // Info flash overlay (e.g. "Copied: RFC-0025")
    if let Some(ref msg) = app.info_flash {
        render_info_flash(f, f.area(), msg);
    }

    // Error flash overlay (rendered last, on top of everything)
    if let Some(ref flash) = app.error_flash {
        render_error_flash(f, f.area(), flash);
    }
}

pub(super) fn short_id(id: &str) -> String {
    id[..id.len().min(16)].to_string()
}

/// SPEC-2.0 §6.1: format a thread ID for human-facing display in the TUI.
/// Bare 8-char base36 tokens render as `@XXXXXXXX`; legacy `KIND-…` IDs
/// render unchanged. Single source of truth for every TUI render site.
pub(super) fn display_thread_id(thread_id: &str) -> String {
    id::display_thread_id(thread_id)
}

/// SPEC-2.0 §2.3.3: the conventional tag set a 1.x kind would emit if it
/// were created today via the kind preset. Used by `tui/state.rs` to seed
/// the thread-detail tag panel for unmigrated threads (no `facet_set` and
/// no replayed tags); migrated threads use [`row_tags`] which now reads
/// from the real `thread_tags` column.
pub(super) fn conventional_tags_for_kind(kind: ThreadKind) -> Vec<String> {
    match kind {
        ThreadKind::Rfc => vec!["cross-cutting".to_string()],
        ThreadKind::Issue => vec!["bug".to_string()],
        ThreadKind::Task => vec!["task".to_string()],
        ThreadKind::Dec => Vec::new(),
    }
}

/// Lifecycle string for a `ThreadRow`. Phase 3: reads the real `lifecycle`
/// column (no longer kind-derived). Falls back to a kind-based guess when
/// the column is empty (defensive — schema NOT NULL DEFAULT 'execution'
/// makes this branch unreachable for v2 indexes).
pub(super) fn row_lifecycle(row: &ThreadRow) -> String {
    if !row.lifecycle.is_empty() {
        return row.lifecycle.clone();
    }
    match row.kind.as_str() {
        "rfc" => "proposal".to_string(),
        "dec" => "record".to_string(),
        _ => "execution".to_string(),
    }
}

/// Display tags for a `ThreadRow`. Phase 3: reads the real `thread_tags`
/// rows (joined into `ThreadRow.tags`). For rows with no replayed tags
/// AND no explicit `facet_set`, falls back to the kind-conventional set
/// so unmigrated 1.x threads still display familiar labels.
pub(super) fn row_tags(row: &ThreadRow) -> Vec<String> {
    if !row.tags.is_empty() {
        return row.tags.clone();
    }
    if row.lifecycle_explicit {
        return Vec::new();
    }
    match row.kind.as_str() {
        "rfc" => vec!["cross-cutting".to_string()],
        "issue" => vec!["bug".to_string()],
        "task" => vec!["task".to_string()],
        _ => Vec::new(),
    }
}

/// SPEC-2.0 §11 / JOB-d4cdyi5b AC#7 — thread-detail "linked" advisory panel.
///
/// One-line text summarising direct incoming `implements` children. Track G
/// is responsible for the reverse-link index this should read from; until
/// that index ships, the panel falls back to the documented "rebuild
/// required" message. Pure display per CORE-VALUE.md "Advisories".
fn linked_panel_text() -> &'static str {
    "**linked:** (linked-children index unavailable; run `git forum reindex`)"
}

fn lifecycle_color(lifecycle: &str) -> Color {
    match lifecycle {
        "proposal" => Color::Cyan,
        "execution" => Color::Yellow,
        "record" => Color::Magenta,
        _ => Color::Reset,
    }
}

pub(super) fn form_line(active: bool, label: &str, value: &str) -> String {
    let marker = if active { ">" } else { " " };
    format!("{marker} {label}: {value}")
}

pub(super) fn single_line_preview(s: &str, max: usize) -> String {
    let joined = s.lines().collect::<Vec<_>>().join(" / ");
    if joined.chars().count() <= max {
        joined
    } else {
        format!("{}...", joined.chars().take(max).collect::<String>())
    }
}

/// Shorten an ISO datetime string to date-only (YYYY-MM-DD) for list display.
fn short_datetime(s: &str) -> String {
    if s.len() >= 10 {
        s[..10].to_string()
    } else {
        s.to_string()
    }
}

pub(super) fn node_status(node: &Node) -> &'static str {
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

fn status_color(status: &str) -> Color {
    match status {
        "open" | "draft" => Color::Green,
        "working" | "review" => Color::Yellow,
        "done" => Color::Magenta,
        "rejected" => Color::Red,
        "deprecated" | "withdrawn" => Color::DarkGray,
        _ => Color::Reset,
    }
}

fn node_type_color(node_type: &str) -> Color {
    // SPEC-2.0 §2.5 + ADR-006: palette reduced to the four canonical types;
    // legacy prose-only types collapse to Reset.
    match node_type {
        "objection" => Color::Red,
        "approval" => Color::Green,
        "action" => Color::Cyan,
        _ => Color::Reset,
    }
}

fn node_status_color(status: &str) -> Color {
    match status {
        "open" => Color::Green,
        "resolved" => Color::DarkGray,
        "retracted" => Color::DarkGray,
        "incorporated" => Color::DarkGray,
        _ => Color::Reset,
    }
}

fn node_row_modifier(node: &Node) -> Modifier {
    if node.retracted || node.resolved || node.incorporated {
        Modifier::DIM
    } else {
        Modifier::empty()
    }
}

/// Render full-screen text for terminal-native selection.
///
/// Shows the main pane text without borders or other UI elements so that
/// terminal drag selection captures only the relevant content.
fn render_select_mode(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(
        Paragraph::new(" SELECT MODE — select text with mouse, press any key to return")
            .style(Style::default().bg(Color::Yellow).fg(Color::Black)),
        chunks[0],
    );

    let selected_node_body: String;
    let text: &str = match &app.view {
        View::ThreadDetail(_) => {
            let node = app
                .node_table_state
                .selected()
                .and_then(|i| app.visible_tree_indices.get(i))
                .and_then(|&ti| app.tree_entries.get(ti))
                .map(|entry| &app.thread_nodes[entry.node_index]);
            if let Some(node) = node {
                selected_node_body = format!(
                    "type:     {}\nstatus:   {}\nactor:    {}\ncreated:  {}\nbody:\n{}",
                    node.node_type,
                    node_status(node),
                    node.actor,
                    node.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
                    node.body,
                );
                &selected_node_body
            } else {
                &app.thread_text
            }
        }
        View::NodeDetail { .. } => &app.node_detail_text,
        _ => &app.thread_text,
    };

    let scroll = match &app.view {
        View::ThreadDetail(_) => app.thread_scroll,
        View::NodeDetail { .. } => app.node_detail_scroll,
        _ => 0,
    };

    if app.markdown_mode {
        // No block/borders → inner width equals the chunk width
        let inner_w = chunks[1].width as usize;
        let md_text = markdown_to_text(text, Some(inner_w));
        f.render_widget(
            Paragraph::new(md_text)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            chunks[1],
        );
    }
}

pub(crate) fn render_list(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let lifecycle_label = if app.filter.lifecycles.is_empty() {
        "all".to_string()
    } else {
        let mut v: Vec<&str> = app.filter.lifecycles.iter().map(|s| s.as_str()).collect();
        v.sort();
        v.join(",")
    };
    let tag_label = if app.filter.tags.is_empty() {
        "all".to_string()
    } else {
        let mut v: Vec<&str> = app.filter.tags.iter().map(|s| s.as_str()).collect();
        v.sort();
        v.join(",")
    };
    let status_label = if app.filter.statuses.is_empty() {
        "all".to_string()
    } else {
        let mut v: Vec<&str> = app.filter.statuses.iter().map(|s| s.as_str()).collect();
        v.sort();
        v.join(",")
    };
    let help_text = format!(
        " [q]quit  [enter]detail  [c]create thread  [r]refresh  [f]filter:{lifecycle_label}/{tag_label}/{status_label}  [j/k]navigate"
    );
    let filter_prefix = " [q]quit  [enter]detail  [c]create thread  [r]refresh  ";
    let filter_start = filter_prefix.len() as u16;
    let filter_len = format!("[f]filter:{lifecycle_label}/{tag_label}/{status_label}").len() as u16;
    app.ui_rects.filter_label = Some(Rect {
        x: chunks[0].x + filter_start,
        y: chunks[0].y,
        width: filter_len,
        height: 1,
    });
    f.render_widget(Paragraph::new(Line::from(help_text)), chunks[0]);

    // Collect rows eagerly so the immutable borrow of `app.threads` ends
    // before we need `&mut app.table_state` for render_stateful_widget.
    let (rows, count) = {
        let visible = app.visible_threads();
        let count = visible.len();
        let rows: Vec<Row> = visible
            .iter()
            .map(|t| {
                let lifecycle = row_lifecycle(t);
                let tags = row_tags(t);
                let title_cell = if tags.is_empty() {
                    t.title.clone()
                } else {
                    format!("[{}] {}", tags.join(","), t.title)
                };
                Row::new(vec![
                    Cell::from(display_thread_id(&t.id)),
                    Cell::from(lifecycle.clone())
                        .style(Style::default().fg(lifecycle_color(&lifecycle))),
                    Cell::from(t.status.clone())
                        .style(Style::default().fg(status_color(&t.status))),
                    Cell::from(short_datetime(&t.created_at)),
                    Cell::from(short_datetime(&t.updated_at)),
                    Cell::from(title_cell),
                ])
            })
            .collect();
        (rows, count)
    };

    let widths = [
        Constraint::Length(13),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Min(20),
    ];
    let labels = ["ID", "LIFECYCLE", "STATUS", "CREATED", "UPDATED", "TITLE"];
    let indicator = if app.sort_ascending {
        " \u{25b2}"
    } else {
        " \u{25bc}"
    };
    let header_cells: Vec<Cell> = SORT_COLUMNS
        .iter()
        .zip(labels.iter())
        .map(|(col, label)| {
            if *col == app.sort_column {
                Cell::from(format!("{label}{indicator}"))
            } else {
                Cell::from(*label)
            }
        })
        .collect();
    let header = Row::new(header_cells).style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" git-forum "))
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );

    // Track column header rects for click-to-sort.
    // Table area has border (1px each side), header row is at area.y + 1.
    let table_area = chunks[1];
    let header_y = table_area.y + 1;
    let mut col_x = table_area.x + 1; // +1 for left border
    let inner_width = table_area.width.saturating_sub(2);
    // Resolve constraints to actual widths
    let resolved: Vec<u16> = {
        let fixed_total: u16 = widths
            .iter()
            .filter_map(|c| match c {
                Constraint::Length(l) => Some(*l),
                _ => None,
            })
            .sum();
        widths
            .iter()
            .map(|c| match c {
                Constraint::Length(l) => *l,
                Constraint::Min(_) => inner_width.saturating_sub(fixed_total),
                _ => 0,
            })
            .collect()
    };
    for (i, w) in resolved.iter().enumerate() {
        app.ui_rects.column_headers[i] = Some(Rect {
            x: col_x,
            y: header_y,
            width: *w,
            height: 1,
        });
        col_x += w;
    }

    app.ui_rects.list_table = Some(chunks[1]);
    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    f.render_widget(Paragraph::new(format!(" {count} threads")), chunks[2]);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

fn render_filter_checkbox_list<'a>(
    labels: &[&str],
    checked: &std::collections::HashSet<String>,
    cursor: usize,
    is_active: bool,
) -> Vec<ListItem<'a>> {
    labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let check = if checked.contains(*label) {
                "[x]"
            } else {
                "[ ]"
            };
            let cursor_mark = if is_active && i == cursor { ">" } else { " " };
            let style = if is_active && i == cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if checked.contains(*label) {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(format!("{cursor_mark}{check} {label}")).style(style)
        })
        .collect()
}

fn render_filter_bar(f: &mut Frame, area: Rect, app: &mut App) {
    let discovered_tags = app.discovered_tags();
    let Some(ref bar) = app.filter_bar else {
        return;
    };

    let popup = centered_rect(60, 18, area);
    app.ui_rects.filter_popup = Some(popup);

    f.render_widget(Clear, popup);

    let block = Block::default().borders(Borders::ALL).title(" filter ");
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(36),
            Constraint::Percentage(36),
        ])
        .split(chunks[0]);

    // LIFECYCLE checkboxes
    let lifecycle_active = bar.field == FilterField::Lifecycle;
    let lifecycle_cursor = if lifecycle_active {
        bar.cursor
    } else {
        usize::MAX
    };
    let lifecycle_items = render_filter_checkbox_list(
        &FILTER_LIFECYCLE_LABELS,
        &bar.lifecycles,
        lifecycle_cursor,
        lifecycle_active,
    );
    let lifecycle_title = if lifecycle_active {
        " LIFECYCLE* "
    } else {
        " LIFECYCLE "
    };
    let lifecycle_list = List::new(lifecycle_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(lifecycle_title),
    );
    app.ui_rects.filter_kind_area = Some(cols[0]);
    f.render_widget(lifecycle_list, cols[0]);

    // TAG checkboxes — discovered from the loaded thread set (§2.3.5: no
    // preregistered list).
    let tag_active = bar.field == FilterField::Tag;
    let tag_cursor = if tag_active { bar.cursor } else { usize::MAX };
    let tag_label_refs: Vec<&str> = discovered_tags.iter().map(|s| s.as_str()).collect();
    let tag_items = render_filter_checkbox_list(&tag_label_refs, &bar.tags, tag_cursor, tag_active);
    let tag_title = if tag_active { " TAGS* " } else { " TAGS " };
    let tag_list =
        List::new(tag_items).block(Block::default().borders(Borders::ALL).title(tag_title));
    f.render_widget(tag_list, cols[1]);

    // STATUS checkboxes
    let status_active = bar.field == FilterField::Status;
    let status_cursor = if status_active {
        bar.cursor
    } else {
        usize::MAX
    };
    let status_items = render_filter_checkbox_list(
        &FILTER_STATUS_LABELS,
        &bar.statuses,
        status_cursor,
        status_active,
    );
    let status_title = if status_active {
        " STATUS* "
    } else {
        " STATUS "
    };
    let status_list =
        List::new(status_items).block(Block::default().borders(Borders::ALL).title(status_title));
    app.ui_rects.filter_status_area = Some(cols[2]);
    f.render_widget(status_list, cols[2]);

    // Help line
    f.render_widget(
        Paragraph::new(" [tab]col [j/k]move [space]toggle [enter]ok [esc]cancel [x]clear"),
        chunks[1],
    );
}

pub(crate) fn render_thread_detail(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Track [esc/q]back label for mouse click
    app.ui_rects.help_line = Some(Rect {
        x: chunks[0].x + 1,
        y: chunks[0].y,
        width: 11, // "[esc/q]back"
        height: 1,
    });
    let md_indicator = if app.markdown_mode { "md:on" } else { "md:off" };
    let tree_indicator = if app.split_horizontal {
        "t:horiz"
    } else {
        "t:vert"
    };
    f.render_widget(
        Paragraph::new(format!(
            " [esc/q]back [enter]node [e]edit [c]create [l]link [m]{md_indicator} [S]select [r]refresh [j/k]nodes [z]fold [{tree_indicator}]",
        )),
        chunks[0],
    );

    let thread_id = if let View::ThreadDetail(ref id) = app.view {
        id.as_str()
    } else {
        ""
    };
    let thread_id_display = display_thread_id(thread_id);

    // Feature 2: split direction toggle (horizontal = top/bottom, vertical = left/right)
    if app.tree_fullscreen {
        app.ui_rects.thread_body = None;
        app.ui_rects.thread_nodes = Some(chunks[1]);
    } else {
        let left_pct = app.detail_split;
        let right_pct = 100u16.saturating_sub(left_pct);
        let direction = if app.split_horizontal {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        let main = Layout::default()
            .direction(direction)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(chunks[1]);

        app.ui_rects.thread_body = Some(main[0]);
        app.ui_rects.thread_nodes = Some(main[1]);

        // Feature 3: show selected node body in left pane
        // Row 0 is the thread root; node rows start at index 1
        let selected_node: Option<&Node> = app
            .node_table_state
            .selected()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| app.visible_tree_indices.get(i))
            .and_then(|&ti| app.tree_entries.get(ti))
            .map(|entry| &app.thread_nodes[entry.node_index]);

        let (body_title, body_content) = if let Some(node) = selected_node {
            let title = format!(" {} {} ", short_id(&node.node_id), node.node_type);
            let mut content = String::new();
            content.push_str(&format!("**type:**     {}\n", node.node_type));
            content.push_str(&format!("**status:**   {}\n", node_status(node)));
            content.push_str(&format!("**actor:**    {}\n", node.actor));
            content.push_str(&format!(
                "**created:**  {}\n",
                node.created_at.format("%Y-%m-%dT%H:%M:%SZ")
            ));
            if let Some(ref reply_to) = node.reply_to {
                content.push_str(&format!("**reply-to:** {}\n", short_id(reply_to)));
            }
            content.push_str(&format!(
                "**in thread:** {}\n",
                display_thread_id(thread_id)
            ));
            content.push_str("\n---\n\n");
            for line in node.body.lines() {
                content.push_str(&format!("{line}\n"));
            }
            if node.body.is_empty() {
                content.push('\n');
            }
            (title, content)
        } else {
            // SPEC-2.0 §11: thread-detail header surfaces lifecycle + tags
            // (replacing 1.x kind), plus a one-line "linked" advisory panel
            // (incoming `implements` children, advisory only).
            let lifecycle = app.thread_lifecycle.as_deref().unwrap_or("execution");
            let tags_line = if app.thread_tags.is_empty() {
                "(none)".to_string()
            } else {
                app.thread_tags.join(", ")
            };
            let mut content = String::new();
            content.push_str(&format!("**lifecycle:** {lifecycle}\n"));
            content.push_str(&format!("**tags:**      {tags_line}\n"));
            content.push_str(&format!("**status:**    {}\n", app.thread_status));
            content.push_str("\n---\n\n");
            content.push_str(&app.thread_text);
            // Linked advisory panel — pure display, no enforcement.
            // CORE-VALUE.md "Advisories": informational only.
            content.push_str("\n\n---\n");
            content.push_str(linked_panel_text());
            content.push('\n');
            (format!(" {thread_id_display} "), content)
        };

        let body_block = Block::default().borders(Borders::ALL).title(body_title);
        if app.markdown_mode {
            // Inner width = area - 2 (left/right border)
            let inner_w = main[0].width.saturating_sub(2) as usize;
            let md_text = markdown_to_text(&body_content, Some(inner_w));
            f.render_widget(
                Paragraph::new(md_text)
                    .block(body_block)
                    .wrap(Wrap { trim: false })
                    .scroll((app.thread_scroll, 0)),
                main[0],
            );
        } else {
            f.render_widget(
                Paragraph::new(body_content)
                    .block(body_block)
                    .wrap(Wrap { trim: false })
                    .scroll((app.thread_scroll, 0)),
                main[0],
            );
        }
    }

    // Thread root row: shows the thread itself as the first entry. SPEC-2.0
    // §11: lifecycle column replaces kind; tag chip prepends the title.
    let root_lifecycle = app.thread_lifecycle.clone().unwrap_or_default();
    let root_title_cell = if app.thread_tags.is_empty() {
        single_line_preview(&app.thread_title, 36)
    } else {
        single_line_preview(
            &format!("[{}] {}", app.thread_tags.join(","), app.thread_title),
            36,
        )
    };
    let root_row = Row::new(vec![
        Cell::from(display_thread_id(thread_id)),
        Cell::from(root_lifecycle.clone())
            .style(Style::default().fg(lifecycle_color(&root_lifecycle))),
        Cell::from(app.thread_status.clone())
            .style(Style::default().fg(status_color(&app.thread_status))),
        Cell::from(root_title_cell),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    // Feature 1: build rows from visible entries only, with collapse indicators
    let node_rows = app.visible_tree_indices.iter().map(|&ti| {
        let entry = &app.tree_entries[ti];
        let node = &app.thread_nodes[entry.node_index];
        let type_str = node.node_type.to_string();
        let status_str = node_status(node);
        let dim = node_row_modifier(node);
        let node_id = &node.node_id;
        // Collapse indicator for nodes with children
        let fold_indicator = if entry.has_children {
            if app.collapsed.contains(node_id) {
                "▸"
            } else {
                "▾"
            }
        } else {
            " "
        };
        let type_display = if entry.prefix.is_empty() {
            format!("{fold_indicator}{type_str}")
        } else {
            format!("{}{fold_indicator}{type_str}", entry.prefix)
        };
        let body_max = 36usize.saturating_sub(entry.depth as usize * 2);
        Row::new(vec![
            Cell::from(short_id(&node.node_id)),
            Cell::from(type_display).style(Style::default().fg(node_type_color(&type_str))),
            Cell::from(status_str).style(Style::default().fg(node_status_color(status_str))),
            Cell::from(single_line_preview(&node.body, body_max)),
        ])
        .style(Style::default().add_modifier(dim))
    });

    let rows: Vec<Row> = std::iter::once(root_row).chain(node_rows).collect();
    let visible_count = app.visible_tree_indices.len();
    let total_count = app.thread_nodes.len();
    let node_title = if visible_count < total_count {
        format!(" nodes ({}/{}) ", visible_count, total_count)
    } else {
        format!(" nodes ({}) ", total_count)
    };
    let header = Row::new(["ID", "TYPE", "STATUS", "BODY"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Min(16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(node_title))
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );
    let tree_area = app.ui_rects.thread_nodes.unwrap_or(chunks[1]);
    f.render_stateful_widget(table, tree_area, &mut app.node_table_state);
}

pub(crate) fn render_node_detail(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Track [esc/q]back label for mouse click
    app.ui_rects.help_line = Some(Rect {
        x: chunks[0].x + 1,
        y: chunks[0].y,
        width: 11, // "[esc/q]back"
        height: 1,
    });
    let md_indicator = if app.markdown_mode { "md:on" } else { "md:off" };
    f.render_widget(
        Paragraph::new(format!(
            " [esc/q]back  [c]create  [l]link  [x]resolve  [o]reopen  [R]retract  [m]{md_indicator}  [r]refresh  [j/k]scroll",
        )),
        chunks[0],
    );

    let title = if let View::NodeDetail { ref node_id, .. } = app.view {
        short_id(node_id)
    } else {
        String::new()
    };
    app.ui_rects.node_detail = Some(chunks[1]);
    let node_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" node {title} "));
    if app.markdown_mode {
        let inner_w = chunks[1].width.saturating_sub(2) as usize;
        let md_text = markdown_to_text(app.node_detail_text.as_str(), Some(inner_w));
        f.render_widget(
            Paragraph::new(md_text)
                .block(node_block)
                .wrap(Wrap { trim: false })
                .scroll((app.node_detail_scroll, 0)),
            chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(app.node_detail_text.as_str())
                .block(node_block)
                .wrap(Wrap { trim: false })
                .scroll((app.node_detail_scroll, 0)),
            chunks[1],
        );
    }
}

pub(crate) fn render_create_thread(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let help = match app.thread_form.field {
        ThreadFormField::Lifecycle => " [tab]next field  [up/down]cycle lifecycle  [esc]cancel",
        ThreadFormField::Tags => " [tab]next field  [type]edit tags (comma-sep)  [esc]cancel",
        ThreadFormField::Title => " [tab]next field  [type]edit title  [esc]cancel",
        ThreadFormField::Body => " [tab]next field  [enter]edit body  [esc]cancel",
        ThreadFormField::Submit => " [tab]next field  [enter]submit  [esc]cancel",
    };
    f.render_widget(Paragraph::new(help), chunks[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let body_preview = thread_body_preview(&app.thread_form.body);
    let tags_display = if app.thread_form.tags.is_empty() {
        "(none)".to_string()
    } else {
        app.thread_form.tags.clone()
    };
    let mut lines = vec![
        form_line(
            app.thread_form.field == ThreadFormField::Lifecycle,
            "lifecycle",
            thread_lifecycle_labels()[app.thread_form.lifecycle_index],
        ),
        form_line(
            app.thread_form.field == ThreadFormField::Tags,
            "tags",
            &tags_display,
        ),
        form_line(
            app.thread_form.field == ThreadFormField::Title,
            "title",
            app.thread_form.title.as_str(),
        ),
        form_line(
            app.thread_form.field == ThreadFormField::Body,
            "body",
            &body_preview,
        ),
        String::new(),
        form_line(
            app.thread_form.field == ThreadFormField::Submit,
            "submit",
            "[Create thread]",
        ),
    ];
    if let Some(ref err) = app.thread_form.tag_error {
        lines.push(format!("  ! {err}"));
    }
    f.render_widget(
        Paragraph::new(lines.join("\n")).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" create thread "),
        ),
        main[0],
    );
    app.ui_rects.thread_submit = Some(Rect {
        x: main[0].x + 1,
        y: main[0].y + 6,
        width: main[0].width.saturating_sub(2),
        height: 1,
    });
    // Track form field rects: lifecycle(1), tags(2), title(3), body(4), submit(6)
    for (i, field_y) in [1u16, 2, 3, 4].iter().enumerate() {
        app.ui_rects.form_fields[i] = Some(Rect {
            x: main[0].x + 1,
            y: main[0].y + field_y,
            width: main[0].width.saturating_sub(2),
            height: 1,
        });
    }

    let items: Vec<ListItem> = thread_lifecycle_labels()
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let prefix = if app.thread_form.field == ThreadFormField::Lifecycle
                && i == app.thread_form.lifecycle_index
            {
                "> "
            } else {
                "  "
            };
            ListItem::new(format!("{prefix}{label}"))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" thread lifecycles "),
    );
    // Track dropdown rect for click-to-select
    app.ui_rects.dropdown = Some(main[1]);
    f.render_widget(list, main[1]);
}

pub(crate) fn render_create_node(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let help = match app.node_form.field {
        NodeFormField::Type => " [tab]next field  [up/down]cycle type  [esc]cancel",
        NodeFormField::Body => " [tab]next field  [enter]edit body  [esc]cancel",
        NodeFormField::Submit => " [tab]next field  [enter]submit  [esc]cancel",
    };
    f.render_widget(Paragraph::new(help), chunks[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let body_preview = node_body_preview(&app.node_form.body);
    let lines = [
        form_line(
            app.node_form.field == NodeFormField::Type,
            "type",
            node_type_labels()[app.node_form.node_type_index],
        ),
        String::new(),
        form_line(
            app.node_form.field == NodeFormField::Body,
            "body",
            &body_preview,
        ),
        String::new(),
        form_line(
            app.node_form.field == NodeFormField::Submit,
            "submit",
            "[Create node]",
        ),
    ];
    f.render_widget(
        Paragraph::new(lines.join("\n")).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" create node "),
        ),
        main[0],
    );
    app.ui_rects.node_submit = Some(Rect {
        x: main[0].x + 1,
        y: main[0].y + 5,
        width: main[0].width.saturating_sub(2),
        height: 1,
    });
    // Track form field rects: type(1), body(3), submit(5) — rows 2,4 are blank
    for (i, field_y) in [1u16, 3, 5].iter().enumerate() {
        app.ui_rects.form_fields[i] = Some(Rect {
            x: main[0].x + 1,
            y: main[0].y + field_y,
            width: main[0].width.saturating_sub(2),
            height: 1,
        });
    }

    let items: Vec<ListItem> = node_type_labels()
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let prefix = if index == app.node_form.node_type_index {
                ">"
            } else {
                " "
            };
            ListItem::new(format!("{prefix} {label}"))
        })
        .collect();

    let title = if app.node_form.field == NodeFormField::Type {
        " node types (dropdown) "
    } else {
        " node types "
    };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    app.ui_rects.dropdown = Some(main[1]);
    f.render_widget(list, main[1]);
}

pub(crate) fn render_edit_node_body(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(
        Paragraph::new(" [ctrl+s]done  [enter]newline  [tab]indent  [esc]back"),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(app.node_form.body.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" edit node body "),
        ),
        chunks[1],
    );
}

pub(crate) fn render_edit_thread_body(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(
        Paragraph::new(" [ctrl+s]done  [enter]newline  [tab]indent  [esc]back"),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(app.thread_form.body.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" edit thread body "),
        ),
        chunks[1],
    );
}

pub(crate) fn render_create_link(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let help = match app.link_form.field {
        LinkFormField::Relation => " [tab]next field  [up/down]cycle relation  [esc]cancel",
        LinkFormField::TargetKind => " [tab]next field  [up/down]cycle target kind  [esc]cancel",
        LinkFormField::Target => {
            if link_target_kind_values()[app.link_form.target_kind_index] == LinkTargetKind::Manual
            {
                " [tab]next field  [type]target id  [esc]cancel"
            } else {
                " [tab]next field  [up/down]select target  [esc]cancel"
            }
        }
        LinkFormField::Submit => " [tab]next field  [enter]submit  [esc]cancel",
    };
    f.render_widget(Paragraph::new(help), chunks[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let thread_id = if let View::CreateLink { ref thread_id, .. } = app.view {
        thread_id.as_str()
    } else {
        ""
    };
    let lines = [
        form_line(
            app.link_form.field == LinkFormField::Relation,
            "relation",
            link_relation_labels()[app.link_form.relation_index],
        ),
        form_line(
            app.link_form.field == LinkFormField::TargetKind,
            "target kind",
            link_target_kind_labels()[app.link_form.target_kind_index],
        ),
        form_line(
            app.link_form.field == LinkFormField::Target,
            "target",
            &selected_link_target_label(app, thread_id),
        ),
        String::new(),
        form_line(
            app.link_form.field == LinkFormField::Submit,
            "submit",
            "[Create link]",
        ),
    ];
    f.render_widget(
        Paragraph::new(lines.join("\n")).block(Block::default().borders(Borders::ALL).title(
            format!(" create link from {} ", display_thread_id(thread_id)),
        )),
        main[0],
    );
    app.ui_rects.link_submit = Some(Rect {
        x: main[0].x + 1,
        y: main[0].y + 5,
        width: main[0].width.saturating_sub(2),
        height: 1,
    });
    // Track form field rects: relation(1), target_kind(2), target(3), submit(5)
    for (i, field_y) in [1u16, 2, 3, 5].iter().enumerate() {
        app.ui_rects.form_fields[i] = Some(Rect {
            x: main[0].x + 1,
            y: main[0].y + field_y,
            width: main[0].width.saturating_sub(2),
            height: 1,
        });
    }
    // Track dropdown rect (right pane)
    app.ui_rects.dropdown = Some(main[1]);

    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    if app.link_form.field == LinkFormField::Relation {
        let items: Vec<ListItem> = link_relation_labels()
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let prefix = if index == app.link_form.relation_index {
                    ">"
                } else {
                    " "
                };
                ListItem::new(format!("{prefix} {label}"))
            })
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" link relations "),
        );
        f.render_widget(list, main[1]);
    } else if app.link_form.field == LinkFormField::TargetKind {
        let items: Vec<ListItem> = link_target_kind_labels()
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let prefix = if index == app.link_form.target_kind_index {
                    ">"
                } else {
                    " "
                };
                ListItem::new(format!("{prefix} {label}"))
            })
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" target kinds "),
        );
        f.render_widget(list, main[1]);
    } else if target_kind == LinkTargetKind::Manual {
        let message = Paragraph::new(
            "Enter a thread ID manually.\n\nExamples:\n  ISSUE-0001\n  RFC-0002\n  DEC-0003",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" manual target "),
        );
        f.render_widget(message, main[1]);
    } else {
        let candidates = auto_link_candidates(app, thread_id);
        let items: Vec<ListItem> = candidates
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let prefix = if index == app.link_form.target_index {
                    ">"
                } else {
                    " "
                };
                ListItem::new(format!(
                    "{prefix} {}  {}",
                    display_thread_id(&row.id),
                    single_line_preview(&row.title, 28)
                ))
            })
            .collect();
        let title = format!(
            " {} targets ({}) ",
            link_target_kind_labels()[app.link_form.target_kind_index],
            candidates.len()
        );
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(list, main[1]);
    }
}

fn node_body_preview(body: &str) -> String {
    if body.trim().is_empty() {
        "(empty)".to_string()
    } else {
        single_line_preview(body, 40)
    }
}

fn thread_body_preview(body: &str) -> String {
    if body.trim().is_empty() {
        "(empty)".to_string()
    } else {
        single_line_preview(body, 40)
    }
}

/// Render an error flash as a centered popup overlay.
fn render_error_flash(f: &mut Frame, area: Rect, flash: &ErrorFlash) {
    let mut lines = vec![
        Line::from(""),
        Line::styled(
            format!("  {}", flash.message),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(ref hint) = flash.hint {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("  {hint}"),
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "  Press any key to dismiss",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(""));

    let height = lines.len() as u16 + 2; // +2 for border
    let width = lines
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(30)
        .max(30)
        + 4; // padding

    let popup_width = width.min(area.width.saturating_sub(4));
    let popup_height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Error ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_area);
}

/// Render a brief info flash as a small centered overlay (e.g. "Copied: RFC-0025").
fn render_info_flash(f: &mut Frame, area: Rect, message: &str) {
    let text = format!("  {message}  ");
    let width = (text.len() as u16 + 4).min(area.width.saturating_sub(4));
    let height: u16 = 3;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));
    let paragraph = Paragraph::new(Line::styled(
        text,
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))
    .block(block);
    f.render_widget(paragraph, popup_area);
}

/// Render a confirmation prompt for discarding unsaved form input.
fn render_confirm_discard(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::styled(
            "  Unsaved changes will be lost. Quit?",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled(
            "  Press y to discard, any other key to cancel",
            Style::default().fg(Color::DarkGray),
        ),
        Line::from(""),
    ];

    let height = lines.len() as u16 + 2;
    let width: u16 = 50;
    let popup_width = width.min(area.width.saturating_sub(4));
    let popup_height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Confirm ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_area);
}
