use std::path::Path;
use std::time::Instant;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::Rect;

use crate::internal::error::ForumResult;
use crate::internal::git_ops::GitOps;
use crate::internal::index;
use crate::internal::reindex;

use super::perf::Perf;
use super::state::{
    apply_node_status_action, auto_link_candidates, link_relation_labels, link_target_kind_labels,
    link_target_kind_values, node_type_labels, node_type_values, open_node_detail,
    open_thread_detail, submit_create_link, submit_create_node, submit_create_thread,
    thread_kind_labels, thread_kind_values,
};
use super::{
    App, FilterField, LinkFormField, LinkOrigin, LinkTargetKind, NodeFormField, NodeStatusAction,
    ThreadFormField, View, FILTER_KIND_LABELS, FILTER_STATUS_LABELS,
};

pub(super) fn handle_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    perf: &mut Perf,
) -> ForumResult<bool> {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }

    // Re-enable mouse capture if it was temporarily disabled for text selection
    if app.mouse_capture_disabled {
        execute!(std::io::stdout(), EnableMouseCapture).ok();
        app.mouse_capture_disabled = false;
        // Don't consume the key — fall through to normal handling
    }

    match app.view.clone() {
        View::List => {
            if app.filter_bar.is_some() {
                handle_filter_bar_key(app, key);
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(true),
                    KeyCode::Char('j') | KeyCode::Down => app.move_down(),
                    KeyCode::Char('k') | KeyCode::Up => app.move_up(),
                    KeyCode::Char('f') => app.open_filter_bar(),
                    KeyCode::Char('c') => app.begin_create_thread(),
                    KeyCode::Char('r') => {
                        reindex::run_reindex(git, db_path)?;
                        let threads = index::list_threads(conn)?;
                        let sel = app.table_state.selected().unwrap_or(0);
                        app.threads = threads;
                        let n = app.visible_threads().len();
                        app.table_state
                            .select(if n > 0 { Some(sel.min(n - 1)) } else { None });
                    }
                    KeyCode::Enter => {
                        if let Some(id) = app.selected_thread_id() {
                            open_thread_detail(app, git, &id, None, perf)?;
                        }
                    }
                    _ => {}
                }
            }
        }
        View::ThreadDetail(thread_id) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.view = View::List;
                app.thread_text.clear();
                app.thread_scroll = 0;
                app.thread_nodes.clear();
                app.tree_entries.clear();
                app.visible_tree_indices.clear();
                app.collapsed.clear();
                app.tree_fullscreen = false;
                app.node_detail_text.clear();
                app.node_detail_scroll = 0;
            }
            KeyCode::Char('j') => app.move_node_down(),
            KeyCode::Char('k') => app.move_node_up(),
            KeyCode::Down => app.scroll_thread_down(),
            KeyCode::Up => app.scroll_thread_up(),
            KeyCode::Char('c') => app.begin_create_node(&thread_id),
            KeyCode::Char('l') => app.begin_create_link_from_thread(&thread_id),
            KeyCode::Char('m') => app.markdown_mode = !app.markdown_mode,
            KeyCode::Char('S') => {
                execute!(std::io::stdout(), DisableMouseCapture).ok();
                app.mouse_capture_disabled = true;
            }
            KeyCode::Char('z') => app.toggle_collapse(),
            KeyCode::Char('t') => {
                if app.tree_fullscreen {
                    app.detail_split = app.saved_detail_split;
                    app.tree_fullscreen = false;
                } else {
                    app.saved_detail_split = app.detail_split;
                    app.detail_split = 0;
                    app.tree_fullscreen = true;
                }
            }
            KeyCode::Char('r') => {
                let selected = app.selected_node_id();
                reindex::run_reindex(git, db_path)?;
                open_thread_detail(app, git, &thread_id, selected.as_deref(), perf)?;
            }
            KeyCode::Enter => {
                if let Some(node_id) = app.selected_node_id() {
                    open_node_detail(app, git, &thread_id, &node_id)?;
                }
            }
            _ => {}
        },
        View::NodeDetail { thread_id, node_id } => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                open_thread_detail(app, git, &thread_id, Some(&node_id), perf)?;
            }
            KeyCode::Char('c') => app.begin_create_node(&thread_id),
            KeyCode::Char('l') => app.begin_create_link_from_node(&thread_id, &node_id),
            KeyCode::Char('m') => app.markdown_mode = !app.markdown_mode,
            KeyCode::Char('S') => {
                execute!(std::io::stdout(), DisableMouseCapture).ok();
                app.mouse_capture_disabled = true;
            }
            KeyCode::Char('x') => {
                apply_node_status_action(
                    app,
                    git,
                    &thread_id,
                    &node_id,
                    NodeStatusAction::Resolve,
                )?;
            }
            KeyCode::Char('o') => {
                apply_node_status_action(app, git, &thread_id, &node_id, NodeStatusAction::Reopen)?;
            }
            KeyCode::Char('R') => {
                apply_node_status_action(
                    app,
                    git,
                    &thread_id,
                    &node_id,
                    NodeStatusAction::Retract,
                )?;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.node_detail_scroll = app.node_detail_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.node_detail_scroll = app.node_detail_scroll.saturating_sub(1);
            }
            KeyCode::Char('r') => {
                reindex::run_reindex(git, db_path)?;
                open_node_detail(app, git, &thread_id, &node_id)?;
            }
            _ => {}
        },
        View::CreateThread => {
            handle_create_thread_key(app, key, git, conn, db_path, perf)?;
        }
        View::EditThreadBody => {
            handle_edit_thread_body_key(app, key)?;
        }
        View::CreateNode { thread_id } => {
            handle_create_node_key(app, key, git, conn, db_path, &thread_id, perf)?;
        }
        View::EditNodeBody { thread_id } => {
            handle_edit_node_body_key(app, key, &thread_id)?;
        }
        View::CreateLink { thread_id, origin } => {
            handle_create_link_key(app, key, git, &thread_id, &origin, perf)?;
        }
    }
    Ok(false)
}

pub(super) fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

pub(super) fn table_row_at(area: Rect, row: u16) -> Option<usize> {
    if area.width < 2 || area.height < 3 || row < area.y + 2 || row >= area.y + area.height - 1 {
        return None;
    }
    Some((row - area.y - 2) as usize)
}

/// Map a click position inside a bordered list/dropdown to an item index.
/// Assumes border (1 row top for title/border) + items start at row 1 inside.
pub(super) fn dropdown_item_at(area: Rect, row: u16) -> Option<usize> {
    if row <= area.y || row >= area.y + area.height.saturating_sub(1) {
        return None;
    }
    Some((row - area.y - 1) as usize)
}

pub(super) fn handle_mouse(
    app: &mut App,
    mouse: MouseEvent,
    git: &GitOps,
    _conn: &rusqlite::Connection,
    _db_path: &Path,
    perf: &mut Perf,
) -> ForumResult<bool> {
    match app.view.clone() {
        View::List => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Filter bar popup takes priority when open
                if app.filter_bar.is_some() {
                    handle_filter_bar_mouse(app, mouse);
                    return Ok(false);
                }
                let now = Instant::now();
                let is_double = app.last_click.is_some_and(|(c, r, t)| {
                    c == mouse.column && r == mouse.row && now.duration_since(t).as_millis() < 400
                });
                // Click filter label to open filter bar
                if app
                    .ui_rects
                    .filter_label
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.open_filter_bar();
                } else if let Some(col) = app.column_header_at(mouse.column, mouse.row) {
                    if col == app.sort_column {
                        app.sort_ascending = !app.sort_ascending;
                    } else {
                        app.sort_column = col;
                        app.sort_ascending = true;
                    }
                    let n = app.visible_threads().len();
                    app.table_state.select(if n > 0 { Some(0) } else { None });
                } else if let Some(area) = app.ui_rects.list_table {
                    if let Some(index) = table_row_at(area, mouse.row) {
                        let visible_len = app.visible_threads().len();
                        if index < visible_len {
                            app.table_state.select(Some(index));
                            if is_double {
                                if let Some(thread_id) = app.selected_thread_id() {
                                    open_thread_detail(app, git, &thread_id, None, perf)?;
                                }
                            }
                        }
                    }
                }
                app.last_click = Some((mouse.column, mouse.row, now));
            }
            MouseEventKind::ScrollDown => app.move_down(),
            MouseEventKind::ScrollUp => app.move_up(),
            _ => {}
        },
        View::ThreadDetail(thread_id) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Check if clicking on the border between panes
                if let (Some(body_area), Some(nodes_area)) =
                    (app.ui_rects.thread_body, app.ui_rects.thread_nodes)
                {
                    let border_col = body_area.x + body_area.width;
                    if mouse.column >= border_col.saturating_sub(1)
                        && mouse.column <= nodes_area.x
                        && mouse.row >= body_area.y
                        && mouse.row < body_area.y + body_area.height
                    {
                        app.dragging_border = true;
                        return Ok(false);
                    }
                }
                let now = Instant::now();
                let is_double = app.last_click.is_some_and(|(c, r, t)| {
                    c == mouse.column && r == mouse.row && now.duration_since(t).as_millis() < 400
                });
                // Click [esc/q]back to go back
                if app
                    .ui_rects
                    .help_line
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.view = View::List;
                    app.thread_text.clear();
                    app.thread_nodes.clear();
                    app.tree_entries.clear();
                    app.visible_tree_indices.clear();
                    app.collapsed.clear();
                    app.tree_fullscreen = false;
                    app.thread_scroll = 0;
                } else if let Some(area) = app.ui_rects.thread_nodes {
                    if let Some(index) = table_row_at(area, mouse.row) {
                        // +1 for thread root row at index 0
                        if index < app.visible_tree_indices.len() + 1 {
                            app.node_table_state.select(Some(index));
                            if is_double {
                                if let Some(node_id) = app.selected_node_id() {
                                    open_node_detail(app, git, &thread_id, &node_id)?;
                                }
                            }
                        }
                    }
                }
                app.last_click = Some((mouse.column, mouse.row, now));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if app.dragging_border {
                    if let Some(body_area) = app.ui_rects.thread_body {
                        let total_width = body_area.width
                            + app.ui_rects.thread_nodes.map(|a| a.width).unwrap_or(0);
                        if total_width > 0 {
                            let relative = mouse.column.saturating_sub(body_area.x);
                            let pct = ((relative as u32 * 100) / total_width as u32) as u16;
                            app.detail_split = pct.clamp(20, 80);
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.dragging_border = false;
            }
            MouseEventKind::ScrollDown => {
                if let Some(area) = app.ui_rects.thread_body {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.scroll_thread_down();
                    }
                }
                if let Some(area) = app.ui_rects.thread_nodes {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.move_node_down();
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(area) = app.ui_rects.thread_body {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.scroll_thread_up();
                    }
                }
                if let Some(area) = app.ui_rects.thread_nodes {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.move_node_up();
                    }
                }
            }
            _ => {}
        },
        View::NodeDetail { ref thread_id, .. } => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Click [esc/q]back to go back to thread detail
                if app
                    .ui_rects
                    .help_line
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    let tid = thread_id.clone();
                    let selected = app.selected_node_id();
                    open_thread_detail(app, git, &tid, selected.as_deref(), perf)?;
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(area) = app.ui_rects.node_detail {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.node_detail_scroll = app.node_detail_scroll.saturating_add(1);
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(area) = app.ui_rects.node_detail {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.node_detail_scroll = app.node_detail_scroll.saturating_sub(1);
                    }
                }
            }
            _ => {}
        },
        View::CreateThread => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if app
                    .ui_rects
                    .thread_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.thread_form.field = ThreadFormField::Submit;
                    submit_create_thread(app, git, _conn, _db_path, perf)?;
                } else if let Some(area) = app.ui_rects.dropdown {
                    if let Some(index) = dropdown_item_at(area, mouse.row) {
                        let max = thread_kind_labels().len();
                        if index < max {
                            app.thread_form.kind_index = index;
                            app.thread_form.field = ThreadFormField::Kind;
                        }
                    }
                } else {
                    // Click form field labels to focus
                    let fields = [
                        ThreadFormField::Kind,
                        ThreadFormField::Title,
                        ThreadFormField::Body,
                        ThreadFormField::Submit,
                    ];
                    for (i, field) in fields.iter().enumerate() {
                        if app.ui_rects.form_fields[i]
                            .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                        {
                            app.thread_form.field = *field;
                            break;
                        }
                    }
                }
            }
        }
        View::CreateNode { thread_id } => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if app
                    .ui_rects
                    .node_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.node_form.field = NodeFormField::Submit;
                    submit_create_node(app, git, _conn, _db_path, &thread_id, perf)?;
                } else if let Some(area) = app.ui_rects.dropdown {
                    if let Some(index) = dropdown_item_at(area, mouse.row) {
                        let max = node_type_labels().len();
                        if index < max {
                            app.node_form.node_type_index = index;
                            app.node_form.field = NodeFormField::Type;
                        }
                    }
                } else {
                    let fields = [
                        NodeFormField::Type,
                        NodeFormField::Body,
                        NodeFormField::Submit,
                    ];
                    for (i, field) in fields.iter().enumerate() {
                        if app.ui_rects.form_fields[i]
                            .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                        {
                            app.node_form.field = *field;
                            break;
                        }
                    }
                }
            }
        }
        View::CreateLink { thread_id, origin } => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if app
                    .ui_rects
                    .link_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.link_form.field = LinkFormField::Submit;
                    submit_create_link(app, git, &thread_id, &origin, perf)?;
                } else if let Some(area) = app.ui_rects.dropdown {
                    if let Some(index) = dropdown_item_at(area, mouse.row) {
                        match app.link_form.field {
                            LinkFormField::Relation => {
                                if index < link_relation_labels().len() {
                                    app.link_form.relation_index = index;
                                }
                            }
                            LinkFormField::TargetKind => {
                                if index < link_target_kind_labels().len() {
                                    app.link_form.target_kind_index = index;
                                    app.link_form.target_index = 0;
                                }
                            }
                            LinkFormField::Target => {
                                let candidates = auto_link_candidates(app, &thread_id);
                                if index < candidates.len() {
                                    app.link_form.target_index = index;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    let fields = [
                        LinkFormField::Relation,
                        LinkFormField::TargetKind,
                        LinkFormField::Target,
                        LinkFormField::Submit,
                    ];
                    for (i, field) in fields.iter().enumerate() {
                        if app.ui_rects.form_fields[i]
                            .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                        {
                            app.link_form.field = *field;
                            break;
                        }
                    }
                }
            }
        }
        View::EditThreadBody | View::EditNodeBody { .. } => {}
    }
    Ok(false)
}

fn handle_filter_bar_mouse(app: &mut App, mouse: MouseEvent) {
    // Click inside kind list — toggle checkbox
    if let Some(area) = app.ui_rects.filter_kind_area {
        if rect_contains(area, mouse.column, mouse.row) {
            if let Some(index) = dropdown_item_at(area, mouse.row) {
                if index < FILTER_KIND_LABELS.len() {
                    if let Some(ref mut bar) = app.filter_bar {
                        bar.field = FilterField::Kind;
                        bar.cursor = index;
                    }
                    app.toggle_filter_checkbox();
                }
            }
            return;
        }
    }
    // Click inside status list — toggle checkbox
    if let Some(area) = app.ui_rects.filter_status_area {
        if rect_contains(area, mouse.column, mouse.row) {
            if let Some(index) = dropdown_item_at(area, mouse.row) {
                if index < FILTER_STATUS_LABELS.len() {
                    if let Some(ref mut bar) = app.filter_bar {
                        bar.field = FilterField::Status;
                        bar.cursor = index;
                    }
                    app.toggle_filter_checkbox();
                }
            }
            return;
        }
    }
    // Click inside popup but not on a list — ignore
    if let Some(popup) = app.ui_rects.filter_popup {
        if rect_contains(popup, mouse.column, mouse.row) {
            return;
        }
    }
    // Click outside popup — cancel
    app.cancel_filter_bar();
}

pub(super) fn handle_filter_bar_key(app: &mut App, key: crossterm::event::KeyEvent) {
    let Some(ref mut bar) = app.filter_bar else {
        return;
    };
    let max = match bar.field {
        FilterField::Kind => FILTER_KIND_LABELS.len(),
        FilterField::Status => FILTER_STATUS_LABELS.len(),
    };
    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            bar.field = match bar.field {
                FilterField::Kind => FilterField::Status,
                FilterField::Status => FilterField::Kind,
            };
            bar.cursor = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            bar.cursor = (bar.cursor + 1).min(max - 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            bar.cursor = bar.cursor.saturating_sub(1);
        }
        KeyCode::Char(' ') => app.toggle_filter_checkbox(),
        KeyCode::Enter => app.apply_filter_bar(),
        KeyCode::Esc => app.cancel_filter_bar(),
        KeyCode::Char('x') => app.clear_filter_bar(),
        _ => {}
    }
}

pub(super) fn handle_create_thread_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    perf: &mut Perf,
) -> ForumResult<()> {
    match key.code {
        KeyCode::Esc => app.view = View::List,
        KeyCode::Tab => {
            app.thread_form.field = match app.thread_form.field {
                ThreadFormField::Kind => ThreadFormField::Title,
                ThreadFormField::Title => ThreadFormField::Body,
                ThreadFormField::Body => ThreadFormField::Submit,
                ThreadFormField::Submit => ThreadFormField::Kind,
            };
        }
        KeyCode::Up => {
            if app.thread_form.field == ThreadFormField::Kind {
                app.thread_form.kind_index = app.thread_form.kind_index.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if app.thread_form.field == ThreadFormField::Kind {
                app.thread_form.kind_index =
                    (app.thread_form.kind_index + 1).min(thread_kind_values().len() - 1);
            }
        }
        KeyCode::Backspace => match app.thread_form.field {
            ThreadFormField::Title => {
                app.thread_form.title.pop();
            }
            ThreadFormField::Body | ThreadFormField::Kind | ThreadFormField::Submit => {}
        },
        KeyCode::Char(ch) => match app.thread_form.field {
            ThreadFormField::Title => app.thread_form.title.push(ch),
            ThreadFormField::Body | ThreadFormField::Kind | ThreadFormField::Submit => {}
        },
        KeyCode::Enter => match app.thread_form.field {
            ThreadFormField::Body => app.view = View::EditThreadBody,
            ThreadFormField::Submit => submit_create_thread(app, git, conn, db_path, perf)?,
            ThreadFormField::Kind | ThreadFormField::Title => {}
        },
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_edit_thread_body_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
) -> ForumResult<()> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        app.view = View::CreateThread;
        app.thread_form.field = ThreadFormField::Body;
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            app.view = View::CreateThread;
            app.thread_form.field = ThreadFormField::Body;
        }
        KeyCode::Enter => app.thread_form.body.push('\n'),
        KeyCode::Backspace => {
            app.thread_form.body.pop();
        }
        KeyCode::Tab => app.thread_form.body.push_str("    "),
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.thread_form.body.push(ch);
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_create_node_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    thread_id: &str,
    perf: &mut Perf,
) -> ForumResult<()> {
    match key.code {
        KeyCode::Esc => open_thread_detail(app, git, thread_id, None, perf)?,
        KeyCode::Tab => {
            app.node_form.field = match app.node_form.field {
                NodeFormField::Type => NodeFormField::Body,
                NodeFormField::Body => NodeFormField::Submit,
                NodeFormField::Submit => NodeFormField::Type,
            };
        }
        KeyCode::Up => {
            if app.node_form.field == NodeFormField::Type {
                app.node_form.node_type_index = app.node_form.node_type_index.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if app.node_form.field == NodeFormField::Type {
                app.node_form.node_type_index =
                    (app.node_form.node_type_index + 1).min(node_type_values().len() - 1);
            }
        }
        KeyCode::Enter => {
            if app.node_form.field == NodeFormField::Body {
                app.view = View::EditNodeBody {
                    thread_id: thread_id.to_string(),
                };
            } else if app.node_form.field == NodeFormField::Submit {
                submit_create_node(app, git, conn, db_path, thread_id, perf)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_edit_node_body_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    thread_id: &str,
) -> ForumResult<()> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        app.view = View::CreateNode {
            thread_id: thread_id.to_string(),
        };
        app.node_form.field = NodeFormField::Body;
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            app.view = View::CreateNode {
                thread_id: thread_id.to_string(),
            };
            app.node_form.field = NodeFormField::Body;
        }
        KeyCode::Enter => app.node_form.body.push('\n'),
        KeyCode::Backspace => {
            app.node_form.body.pop();
        }
        KeyCode::Tab => app.node_form.body.push_str("    "),
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.node_form.body.push(ch);
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_create_link_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
    perf: &mut Perf,
) -> ForumResult<()> {
    match key.code {
        KeyCode::Esc => super::state::return_from_link_form(app, git, thread_id, origin, perf)?,
        KeyCode::Tab => {
            app.link_form.field = match app.link_form.field {
                LinkFormField::Relation => LinkFormField::TargetKind,
                LinkFormField::TargetKind => LinkFormField::Target,
                LinkFormField::Target => LinkFormField::Submit,
                LinkFormField::Submit => LinkFormField::Relation,
            };
        }
        KeyCode::Up => match app.link_form.field {
            LinkFormField::Relation => {
                app.link_form.relation_index = app.link_form.relation_index.saturating_sub(1);
            }
            LinkFormField::TargetKind => {
                app.link_form.target_kind_index = app.link_form.target_kind_index.saturating_sub(1);
                app.link_form.target_index = 0;
            }
            LinkFormField::Target
                if link_target_kind_values()[app.link_form.target_kind_index]
                    != LinkTargetKind::Manual =>
            {
                app.link_form.target_index = app.link_form.target_index.saturating_sub(1);
            }
            LinkFormField::Target | LinkFormField::Submit => {}
        },
        KeyCode::Down => match app.link_form.field {
            LinkFormField::Relation => {
                app.link_form.relation_index =
                    (app.link_form.relation_index + 1).min(link_relation_labels().len() - 1);
            }
            LinkFormField::TargetKind => {
                app.link_form.target_kind_index =
                    (app.link_form.target_kind_index + 1).min(link_target_kind_values().len() - 1);
                app.link_form.target_index = 0;
            }
            LinkFormField::Target
                if link_target_kind_values()[app.link_form.target_kind_index]
                    != LinkTargetKind::Manual =>
            {
                let candidates = auto_link_candidates(app, thread_id);
                if !candidates.is_empty() {
                    app.link_form.target_index =
                        (app.link_form.target_index + 1).min(candidates.len() - 1);
                }
            }
            LinkFormField::Target | LinkFormField::Submit => {}
        },
        KeyCode::Backspace => {
            if app.link_form.field == LinkFormField::Target
                && link_target_kind_values()[app.link_form.target_kind_index]
                    == LinkTargetKind::Manual
            {
                app.link_form.manual_target.pop();
            }
        }
        KeyCode::Char(ch) => {
            if app.link_form.field == LinkFormField::Target
                && link_target_kind_values()[app.link_form.target_kind_index]
                    == LinkTargetKind::Manual
                && !key.modifiers.contains(KeyModifiers::CONTROL)
            {
                app.link_form.manual_target.push(ch);
            }
        }
        KeyCode::Enter => {
            if app.link_form.field == LinkFormField::Submit {
                submit_create_link(app, git, thread_id, origin, perf)?;
            }
        }
        _ => {}
    }
    Ok(())
}
