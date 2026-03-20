use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::internal::node::Node;

use super::markdown::markdown_to_text;
use super::state::{
    auto_link_candidates, link_relation_labels, link_target_kind_labels, link_target_kind_values,
    node_type_labels, selected_link_target_label, thread_kind_labels,
};
use super::{
    App, FilterField, LinkFormField, LinkTargetKind, NodeFormField, ThreadFormField, UiRects, View,
    FILTER_KIND_LABELS, FILTER_STATUS_LABELS, SORT_COLUMNS,
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
}

pub(super) fn short_id(id: &str) -> String {
    id[..id.len().min(16)].to_string()
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

fn kind_color(kind: &str) -> Color {
    match kind {
        "rfc" => Color::Cyan,
        "issue" => Color::Yellow,
        _ => Color::Reset,
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "open" | "draft" => Color::Green,
        "pending" | "proposed" | "under-review" => Color::Yellow,
        "accepted" | "closed" => Color::Magenta,
        "rejected" => Color::Red,
        "deprecated" => Color::DarkGray,
        _ => Color::Reset,
    }
}

fn node_type_color(node_type: &str) -> Color {
    match node_type {
        "objection" => Color::Red,
        "risk" => Color::Red,
        "question" => Color::Yellow,
        "summary" => Color::Green,
        "action" => Color::Cyan,
        "claim" => Color::Reset,
        "review" => Color::Blue,
        "alternative" => Color::Magenta,
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
        let md_text = markdown_to_text(text);
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

    let kind_label = if app.filter.kinds.is_empty() {
        "all".to_string()
    } else {
        let mut v: Vec<&str> = app.filter.kinds.iter().map(|s| s.as_str()).collect();
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
        " [q]quit  [enter]detail  [c]create thread  [r]refresh  [f]filter:{kind_label}/{status_label}  [j/k]navigate"
    );
    // Track filter label position for mouse click
    let filter_prefix = " [q]quit  [enter]detail  [c]create thread  [r]refresh  ";
    let filter_start = filter_prefix.len() as u16;
    let filter_len = format!("[f]filter:{kind_label}/{status_label}").len() as u16;
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
                Row::new(vec![
                    Cell::from(t.id.clone()),
                    Cell::from(t.kind.clone()).style(Style::default().fg(kind_color(&t.kind))),
                    Cell::from(t.status.clone())
                        .style(Style::default().fg(status_color(&t.status))),
                    Cell::from(short_datetime(&t.created_at)),
                    Cell::from(short_datetime(&t.updated_at)),
                    Cell::from(t.title.clone()),
                ])
            })
            .collect();
        (rows, count)
    };

    let widths = [
        Constraint::Length(13),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Min(20),
    ];
    let labels = ["ID", "KIND", "STATUS", "CREATED", "UPDATED", "TITLE"];
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
    let Some(ref bar) = app.filter_bar else {
        return;
    };

    let popup = centered_rect(40, 18, area);
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
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[0]);

    // KIND checkboxes
    let kind_active = bar.field == FilterField::Kind;
    let kind_cursor = if kind_active { bar.cursor } else { usize::MAX };
    let kind_items =
        render_filter_checkbox_list(&FILTER_KIND_LABELS, &bar.kinds, kind_cursor, kind_active);
    let kind_title = if kind_active { " KIND* " } else { " KIND " };
    let kind_list =
        List::new(kind_items).block(Block::default().borders(Borders::ALL).title(kind_title));
    app.ui_rects.filter_kind_area = Some(cols[0]);
    f.render_widget(kind_list, cols[0]);

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
    app.ui_rects.filter_status_area = Some(cols[1]);
    f.render_widget(status_list, cols[1]);

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
    let tree_indicator = if app.tree_fullscreen {
        "t:full"
    } else {
        "t:split"
    };
    f.render_widget(
        Paragraph::new(format!(
            " [esc/q]back [enter]node [c]create [l]link [m]{md_indicator} [S]select [r]refresh [j/k]nodes [z]fold [{tree_indicator}]",
        )),
        chunks[0],
    );

    let thread_id = if let View::ThreadDetail(ref id) = app.view {
        id.as_str()
    } else {
        ""
    };

    // Feature 2: full-width tree toggle
    if app.tree_fullscreen {
        app.ui_rects.thread_body = None;
        app.ui_rects.thread_nodes = Some(chunks[1]);
    } else {
        let left_pct = app.detail_split;
        let right_pct = 100u16.saturating_sub(left_pct);
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(chunks[1]);

        app.ui_rects.thread_body = Some(main[0]);
        app.ui_rects.thread_nodes = Some(main[1]);

        // Feature 3: show selected node body in left pane
        let selected_node: Option<&Node> = app
            .node_table_state
            .selected()
            .and_then(|i| app.visible_tree_indices.get(i))
            .and_then(|&ti| app.tree_entries.get(ti))
            .map(|entry| &app.thread_nodes[entry.node_index]);

        let (body_title, body_content) = if let Some(node) = selected_node {
            let title = format!(" {} {} ", short_id(&node.node_id), node.node_type);
            let mut content = String::new();
            content.push_str(&format!("type:     {}\n", node.node_type));
            content.push_str(&format!("status:   {}\n", node_status(node)));
            content.push_str(&format!("actor:    {}\n", node.actor));
            content.push_str(&format!(
                "created:  {}\n",
                node.created_at.format("%Y-%m-%dT%H:%M:%SZ")
            ));
            if let Some(ref reply_to) = node.reply_to {
                content.push_str(&format!("reply-to: {}\n", short_id(reply_to)));
            }
            content.push_str("body:\n");
            for line in node.body.lines() {
                content.push_str(&format!("  {line}\n"));
            }
            if node.body.is_empty() {
                content.push_str("  \n");
            }
            (title, content)
        } else {
            (format!(" {thread_id} "), app.thread_text.clone())
        };

        let body_block = Block::default().borders(Borders::ALL).title(body_title);
        if app.markdown_mode {
            let md_text = markdown_to_text(&body_content);
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

    // Feature 1: build rows from visible entries only, with collapse indicators
    let rows: Vec<Row> = app
        .visible_tree_indices
        .iter()
        .map(|&ti| {
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
        })
        .collect();
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
        let md_text = markdown_to_text(app.node_detail_text.as_str());
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
        ThreadFormField::Kind => " [tab]next field  [up/down]cycle kind  [esc]cancel",
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
    let lines = [
        form_line(
            app.thread_form.field == ThreadFormField::Kind,
            "kind",
            thread_kind_labels()[app.thread_form.kind_index],
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
        y: main[0].y + 5,
        width: main[0].width.saturating_sub(2),
        height: 1,
    });
    // Track form field rects for click-to-focus (inside block: +1 for border)
    for (i, field_y) in [1u16, 2, 3, 5].iter().enumerate() {
        app.ui_rects.form_fields[i] = Some(Rect {
            x: main[0].x + 1,
            y: main[0].y + field_y,
            width: main[0].width.saturating_sub(2),
            height: 1,
        });
    }

    let items: Vec<ListItem> = thread_kind_labels()
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let prefix = if app.thread_form.field == ThreadFormField::Kind
                && i == app.thread_form.kind_index
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
            .title(" thread kinds "),
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
        Paragraph::new(lines.join("\n")).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" create link from {thread_id} ")),
        ),
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
                    row.id,
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
