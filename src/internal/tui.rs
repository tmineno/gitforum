use std::path::Path;
use std::time::Instant;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::{Frame, Terminal};

use super::actor;
use super::clock::SystemClock;
use super::create;
use super::error::ForumResult;
use super::event::{NodeType, ThreadKind};
use super::evidence_ops;
use super::git_ops::GitOps;
use super::id::UlidGenerator;
use super::index::{self, ThreadRow};
use super::node::Node;
use super::reindex;
use super::say;
use super::show;
use super::thread;

use ratatui::text::{Line as RLine, Span};

/// Convert a markdown string to styled ratatui Text.
///
/// Supports headings (bold), bold/italic inline, code spans (green),
/// code blocks (green on dark), list items with bullet, and blockquotes.
fn markdown_to_text(input: &str) -> ratatui::text::Text<'static> {
    use pulldown_cmark::{Event as MdEvent, Options, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(input, Options::all());

    let mut lines: Vec<RLine<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut in_heading = false;
    let mut in_code_block = false;
    let mut list_depth: usize = 0;

    let current_style = |stack: &[Style]| -> Style { stack.last().copied().unwrap_or_default() };

    let flush_line = |lines: &mut Vec<RLine<'static>>, spans: &mut Vec<Span<'static>>| {
        if !spans.is_empty() {
            lines.push(RLine::from(std::mem::take(spans)));
        }
    };

    for event in parser {
        match event {
            MdEvent::Start(Tag::Heading { .. }) => {
                flush_line(&mut lines, &mut current_spans);
                in_heading = true;
                style_stack.push(
                    current_style(&style_stack)
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Cyan),
                );
            }
            MdEvent::End(TagEnd::Heading(_)) => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(RLine::default());
                in_heading = false;
                style_stack.pop();
            }
            MdEvent::Start(Tag::Emphasis) => {
                style_stack.push(current_style(&style_stack).add_modifier(Modifier::ITALIC));
            }
            MdEvent::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }
            MdEvent::Start(Tag::Strong) => {
                style_stack.push(current_style(&style_stack).add_modifier(Modifier::BOLD));
            }
            MdEvent::End(TagEnd::Strong) => {
                style_stack.pop();
            }
            MdEvent::Start(Tag::CodeBlock(_)) => {
                flush_line(&mut lines, &mut current_spans);
                in_code_block = true;
                style_stack.push(Style::default().fg(Color::Green));
            }
            MdEvent::End(TagEnd::CodeBlock) => {
                flush_line(&mut lines, &mut current_spans);
                in_code_block = false;
                style_stack.pop();
            }
            MdEvent::Start(Tag::List(_)) => {
                flush_line(&mut lines, &mut current_spans);
                list_depth += 1;
            }
            MdEvent::End(TagEnd::List(_)) => {
                flush_line(&mut lines, &mut current_spans);
                list_depth = list_depth.saturating_sub(1);
            }
            MdEvent::Start(Tag::Item) => {
                flush_line(&mut lines, &mut current_spans);
                let indent = "  ".repeat(list_depth.saturating_sub(1));
                current_spans.push(Span::styled(
                    format!("{indent}• "),
                    current_style(&style_stack),
                ));
            }
            MdEvent::End(TagEnd::Item) => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Start(Tag::BlockQuote(_)) => {
                flush_line(&mut lines, &mut current_spans);
                style_stack.push(Style::default().fg(Color::DarkGray));
            }
            MdEvent::End(TagEnd::BlockQuote(_)) => {
                flush_line(&mut lines, &mut current_spans);
                style_stack.pop();
            }
            MdEvent::Start(Tag::Paragraph) => {
                if !in_heading {
                    flush_line(&mut lines, &mut current_spans);
                }
            }
            MdEvent::End(TagEnd::Paragraph) => {
                flush_line(&mut lines, &mut current_spans);
                if !in_heading {
                    lines.push(RLine::default());
                }
            }
            MdEvent::Code(code) => {
                current_spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Green),
                ));
            }
            MdEvent::Text(text) => {
                if in_code_block {
                    // Preserve newlines in code blocks
                    for (i, line) in text.split('\n').enumerate() {
                        if i > 0 {
                            flush_line(&mut lines, &mut current_spans);
                        }
                        if !line.is_empty() {
                            current_spans
                                .push(Span::styled(line.to_string(), current_style(&style_stack)));
                        }
                    }
                } else {
                    current_spans.push(Span::styled(text.to_string(), current_style(&style_stack)));
                }
            }
            MdEvent::SoftBreak => {
                current_spans.push(Span::raw(" "));
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Rule => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(RLine::from(Span::styled(
                    "───────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(RLine::default());
            }
            _ => {}
        }
    }
    flush_line(&mut lines, &mut current_spans);

    ratatui::text::Text::from(lines)
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    List,
    ThreadDetail(String),
    NodeDetail {
        thread_id: String,
        node_id: String,
    },
    CreateThread,
    EditThreadBody,
    CreateNode {
        thread_id: String,
    },
    EditNodeBody {
        thread_id: String,
    },
    CreateLink {
        thread_id: String,
        origin: LinkOrigin,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOrigin {
    ThreadDetail { selected_node_id: Option<String> },
    NodeDetail { node_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadFormField {
    Kind,
    Title,
    Body,
    Submit,
}

#[derive(Debug, Clone)]
struct ThreadForm {
    kind_index: usize,
    title: String,
    body: String,
    field: ThreadFormField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeFormField {
    Type,
    Body,
    Submit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeStatusAction {
    Resolve,
    Reopen,
    Retract,
}

#[derive(Debug, Clone)]
struct NodeForm {
    node_type_index: usize,
    body: String,
    field: NodeFormField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortColumn {
    Id,
    Kind,
    Status,
    Created,
    Updated,
    Title,
}

const SORT_COLUMNS: [SortColumn; 6] = [
    SortColumn::Id,
    SortColumn::Kind,
    SortColumn::Status,
    SortColumn::Created,
    SortColumn::Updated,
    SortColumn::Title,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFormField {
    Relation,
    TargetKind,
    Target,
    Submit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkTargetKind {
    Issue,
    Rfc,
    Manual,
}

#[derive(Debug, Clone)]
struct LinkForm {
    relation_index: usize,
    target_kind_index: usize,
    target_index: usize,
    manual_target: String,
    field: LinkFormField,
}

#[derive(Debug, Clone, Copy, Default)]
struct UiRects {
    list_table: Option<Rect>,
    thread_body: Option<Rect>,
    thread_nodes: Option<Rect>,
    node_detail: Option<Rect>,
    thread_submit: Option<Rect>,
    node_submit: Option<Rect>,
    link_submit: Option<Rect>,
    /// Dropdown area in form views (right pane).
    dropdown: Option<Rect>,
    /// Column header rects for click-to-sort in list view.
    column_headers: [Option<Rect>; 6],
    /// Filter label area in list view help line.
    filter_label: Option<Rect>,
    /// Help line area (first row) for back navigation clicks.
    help_line: Option<Rect>,
    /// Form field areas for click-to-focus.
    form_fields: [Option<Rect>; 4],
}

/// A node in the tree view with depth information.
#[derive(Debug, Clone)]
struct TreeEntry {
    /// Index into `thread_nodes`.
    node_index: usize,
    /// Nesting depth (0 = top-level).
    depth: u16,
    /// Tree connector prefix for display (e.g. "├─ ", "│  └─ ").
    prefix: String,
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub threads: Vec<ThreadRow>,
    pub table_state: TableState,
    pub kind_filter: Option<String>,
    pub thread_text: String,
    pub thread_scroll: u16,
    pub thread_nodes: Vec<Node>,
    /// Tree-ordered entries for the nodes panel (may differ from thread_nodes order).
    tree_entries: Vec<TreeEntry>,
    pub node_table_state: TableState,
    pub node_detail_text: String,
    pub node_detail_scroll: u16,
    thread_form: ThreadForm,
    node_form: NodeForm,
    link_form: LinkForm,
    ui_rects: UiRects,
    /// Tracks the last left-click position and time for double-click detection.
    last_click: Option<(u16, u16, Instant)>,
    sort_column: SortColumn,
    sort_ascending: bool,
    /// Percentage of horizontal space for the left (body) pane in thread detail (20..80).
    detail_split: u16,
    /// Whether the user is currently dragging the pane border.
    dragging_border: bool,
    /// Timestamp of last auto-refresh check.
    last_refresh: Instant,
    /// Cached tip SHA for the currently viewed thread (for change detection).
    thread_tip_sha: Option<String>,
    /// Whether to render the main pane body as markdown.
    markdown_mode: bool,
    /// Whether mouse capture is temporarily disabled for text selection.
    mouse_capture_disabled: bool,
}

impl App {
    pub fn new(threads: Vec<ThreadRow>) -> Self {
        let mut table_state = TableState::default();
        if !threads.is_empty() {
            table_state.select(Some(0));
        }
        Self {
            view: View::List,
            threads,
            table_state,
            kind_filter: None,
            thread_text: String::new(),
            thread_scroll: 0,
            thread_nodes: Vec::new(),
            tree_entries: Vec::new(),
            node_table_state: TableState::default(),
            node_detail_text: String::new(),
            node_detail_scroll: 0,
            thread_form: ThreadForm {
                kind_index: 1,
                title: String::new(),
                body: String::new(),
                field: ThreadFormField::Kind,
            },
            node_form: NodeForm {
                node_type_index: 0,
                body: String::new(),
                field: NodeFormField::Type,
            },
            link_form: LinkForm {
                relation_index: 0,
                target_kind_index: 0,
                target_index: 0,
                manual_target: String::new(),
                field: LinkFormField::Relation,
            },
            ui_rects: UiRects::default(),
            last_click: None,
            sort_column: SortColumn::Updated,
            sort_ascending: false,
            detail_split: 60,
            dragging_border: false,
            last_refresh: Instant::now(),
            thread_tip_sha: None,
            markdown_mode: false,
            mouse_capture_disabled: false,
        }
    }

    pub fn visible_threads(&self) -> Vec<&ThreadRow> {
        let mut rows: Vec<&ThreadRow> = self
            .threads
            .iter()
            .filter(|t| {
                self.kind_filter
                    .as_deref()
                    .map(|k| t.kind == k)
                    .unwrap_or(true)
            })
            .collect();
        let asc = self.sort_ascending;
        rows.sort_by(|a, b| {
            let ord = match self.sort_column {
                SortColumn::Id => a.id.cmp(&b.id),
                SortColumn::Kind => a.kind.cmp(&b.kind),
                SortColumn::Status => a.status.cmp(&b.status),
                SortColumn::Created => a.created_at.cmp(&b.created_at),
                SortColumn::Updated => a.updated_at.cmp(&b.updated_at),
                SortColumn::Title => a.title.cmp(&b.title),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
        rows
    }

    fn selected_thread_id(&self) -> Option<String> {
        let visible = self.visible_threads();
        self.table_state
            .selected()
            .and_then(|i| visible.get(i))
            .map(|t| t.id.clone())
    }

    fn move_down(&mut self) {
        let n = self.visible_threads().len();
        if n == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|i| (i + 1).min(n - 1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn move_up(&mut self) {
        let n = self.visible_threads().len();
        if n == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn cycle_filter(&mut self) {
        self.kind_filter = match self.kind_filter.as_deref() {
            None => Some("issue".into()),
            Some("issue") => Some("rfc".into()),
            Some("rfc") => None,
            _ => Some("issue".into()),
        };
        let n = self.visible_threads().len();
        self.table_state.select(if n > 0 { Some(0) } else { None });
    }

    fn column_header_at(&self, column: u16, row: u16) -> Option<SortColumn> {
        for (i, rect) in self.ui_rects.column_headers.iter().enumerate() {
            if let Some(area) = rect {
                if rect_contains(*area, column, row) {
                    return Some(SORT_COLUMNS[i]);
                }
            }
        }
        None
    }

    fn selected_node_id(&self) -> Option<String> {
        self.node_table_state
            .selected()
            .and_then(|i| self.tree_entries.get(i))
            .map(|entry| self.thread_nodes[entry.node_index].node_id.clone())
    }

    fn move_node_down(&mut self) {
        let n = self.tree_entries.len();
        if n == 0 {
            return;
        }
        let next = self
            .node_table_state
            .selected()
            .map(|i| (i + 1).min(n - 1))
            .unwrap_or(0);
        self.node_table_state.select(Some(next));
    }

    fn move_node_up(&mut self) {
        let n = self.tree_entries.len();
        if n == 0 {
            return;
        }
        let next = self
            .node_table_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.node_table_state.select(Some(next));
    }

    fn select_node_by_id(&mut self, node_id: Option<&str>) {
        let selected = node_id.and_then(|id| {
            self.tree_entries
                .iter()
                .position(|e| self.thread_nodes[e.node_index].node_id == id)
        });
        self.node_table_state
            .select(match (selected, self.tree_entries.is_empty()) {
                (Some(index), _) => Some(index),
                (None, false) => Some(0),
                (None, true) => None,
            });
    }

    fn scroll_thread_down(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_add(1);
    }

    fn scroll_thread_up(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_sub(1);
    }

    fn begin_create_thread(&mut self) {
        self.thread_form = ThreadForm {
            kind_index: default_thread_kind_index(self.kind_filter.as_deref()),
            title: String::new(),
            body: String::new(),
            field: ThreadFormField::Kind,
        };
        self.view = View::CreateThread;
    }

    fn begin_create_node(&mut self, thread_id: &str) {
        self.node_form = NodeForm {
            node_type_index: 0,
            body: String::new(),
            field: NodeFormField::Type,
        };
        self.view = View::CreateNode {
            thread_id: thread_id.to_string(),
        };
    }

    fn begin_create_link_from_thread(&mut self, thread_id: &str) {
        self.reset_link_form();
        self.view = View::CreateLink {
            thread_id: thread_id.to_string(),
            origin: LinkOrigin::ThreadDetail {
                selected_node_id: self.selected_node_id(),
            },
        };
    }

    fn begin_create_link_from_node(&mut self, thread_id: &str, node_id: &str) {
        self.reset_link_form();
        self.view = View::CreateLink {
            thread_id: thread_id.to_string(),
            origin: LinkOrigin::NodeDetail {
                node_id: node_id.to_string(),
            },
        };
    }

    fn reset_link_form(&mut self) {
        self.link_form = LinkForm {
            relation_index: 0,
            target_kind_index: 0,
            target_index: 0,
            manual_target: String::new(),
            field: LinkFormField::Relation,
        };
    }
}

/// Run the interactive TUI.
///
/// Preconditions: `db_path` is writable so the local index can be refreshed on startup.
/// Postconditions: terminal is restored on exit.
/// Failure modes: ForumError::Io on terminal I/O failure; ForumError::Repo on index/replay errors.
/// Side effects: modifies terminal state; restores on exit.
pub fn run(git: &GitOps, db_path: &Path, initial_thread_id: Option<&str>) -> ForumResult<()> {
    let threads = load_threads(git, db_path)?;
    let conn = index::open_db(db_path)?;

    let mut app = App::new(threads);
    if let Some(thread_id) = initial_thread_id {
        open_thread_detail(&mut app, git, thread_id, None)?;
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app, git, &conn, db_path);

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    result
}

/// Auto-refresh interval: check for thread changes every 2 seconds.
const AUTO_REFRESH_INTERVAL_MS: u128 = 2000;

pub(crate) fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
) -> ForumResult<()> {
    loop {
        terminal.draw(|f| render(f, app))?;

        // Auto-refresh: check if the viewed thread has changed
        if app.last_refresh.elapsed().as_millis() >= AUTO_REFRESH_INTERVAL_MS {
            auto_refresh(app, git, conn, db_path)?;
            app.last_refresh = Instant::now();
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(app, key, git, conn, db_path)? {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    if handle_mouse(app, mouse, git, conn, db_path)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }
}

/// Check if the currently viewed thread or list has changed, and refresh if so.
fn auto_refresh(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
) -> ForumResult<()> {
    match &app.view {
        View::ThreadDetail(thread_id) => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            if let Ok(Some(current_sha)) = git.resolve_ref(&ref_name) {
                let changed = app
                    .thread_tip_sha
                    .as_ref()
                    .is_none_or(|prev| *prev != current_sha);
                if changed {
                    let selected = app.selected_node_id();
                    let thread_id = thread_id.clone();
                    open_thread_detail(app, git, &thread_id, selected.as_deref())?;
                }
            }
        }
        View::NodeDetail { thread_id, node_id } => {
            let ref_name = format!("refs/forum/threads/{thread_id}");
            if let Ok(Some(current_sha)) = git.resolve_ref(&ref_name) {
                let changed = app
                    .thread_tip_sha
                    .as_ref()
                    .is_none_or(|prev| *prev != current_sha);
                if changed {
                    let thread_id = thread_id.clone();
                    let node_id = node_id.clone();
                    app.thread_tip_sha = Some(current_sha);
                    open_node_detail(app, git, &thread_id, &node_id)?;
                }
            }
        }
        View::List => {
            // Refresh list by reindexing
            reindex::run_reindex(git, db_path)?;
            let threads = index::list_threads(conn)?;
            let sel = app.table_state.selected().unwrap_or(0);
            app.threads = threads;
            let n = app.visible_threads().len();
            app.table_state
                .select(if n > 0 { Some(sel.min(n - 1)) } else { None });
        }
        _ => {}
    }
    Ok(())
}

fn handle_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
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
        View::List => match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => app.move_down(),
            KeyCode::Char('k') | KeyCode::Up => app.move_up(),
            KeyCode::Char('f') => app.cycle_filter(),
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
                    open_thread_detail(app, git, &id, None)?;
                }
            }
            _ => {}
        },
        View::ThreadDetail(thread_id) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.view = View::List;
                app.thread_text.clear();
                app.thread_scroll = 0;
                app.thread_nodes.clear();
                app.tree_entries.clear();
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
            KeyCode::Char('r') => {
                let selected = app.selected_node_id();
                reindex::run_reindex(git, db_path)?;
                open_thread_detail(app, git, &thread_id, selected.as_deref())?;
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
                open_thread_detail(app, git, &thread_id, Some(&node_id))?;
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
            handle_create_thread_key(app, key, git, conn, db_path)?;
        }
        View::EditThreadBody => {
            handle_edit_thread_body_key(app, key)?;
        }
        View::CreateNode { thread_id } => {
            handle_create_node_key(app, key, git, conn, db_path, &thread_id)?;
        }
        View::EditNodeBody { thread_id } => {
            handle_edit_node_body_key(app, key, &thread_id)?;
        }
        View::CreateLink { thread_id, origin } => {
            handle_create_link_key(app, key, git, &thread_id, &origin)?;
        }
    }
    Ok(false)
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn table_row_at(area: Rect, row: u16) -> Option<usize> {
    if area.width < 2 || area.height < 3 || row < area.y + 2 || row >= area.y + area.height - 1 {
        return None;
    }
    Some((row - area.y - 2) as usize)
}

/// Map a click position inside a bordered list/dropdown to an item index.
/// Assumes border (1 row top for title/border) + items start at row 1 inside.
fn dropdown_item_at(area: Rect, row: u16) -> Option<usize> {
    if row <= area.y || row >= area.y + area.height.saturating_sub(1) {
        return None;
    }
    Some((row - area.y - 1) as usize)
}

fn handle_mouse(
    app: &mut App,
    mouse: MouseEvent,
    git: &GitOps,
    _conn: &rusqlite::Connection,
    _db_path: &Path,
) -> ForumResult<bool> {
    match app.view.clone() {
        View::List => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = Instant::now();
                let is_double = app.last_click.is_some_and(|(c, r, t)| {
                    c == mouse.column && r == mouse.row && now.duration_since(t).as_millis() < 400
                });
                // Click filter label to cycle
                if app
                    .ui_rects
                    .filter_label
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
                {
                    app.cycle_filter();
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
                                    open_thread_detail(app, git, &thread_id, None)?;
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
                    app.thread_scroll = 0;
                } else if let Some(area) = app.ui_rects.thread_nodes {
                    if let Some(index) = table_row_at(area, mouse.row) {
                        if index < app.tree_entries.len() {
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
                    open_thread_detail(app, git, &tid, selected.as_deref())?;
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
                    submit_create_thread(app, git, _conn, _db_path)?;
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
                    submit_create_node(app, git, _conn, _db_path, &thread_id)?;
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
                    submit_create_link(app, git, &thread_id, &origin)?;
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

fn open_thread_detail(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    selected_node_id: Option<&str>,
) -> ForumResult<()> {
    let state = thread::replay_thread(git, thread_id)?;
    app.thread_text = show::render_show(&state);
    app.thread_scroll = 0;
    app.thread_nodes = state.nodes;
    app.tree_entries = build_tree_entries(&app.thread_nodes);
    app.node_detail_text.clear();
    app.node_detail_scroll = 0;
    // Cache the tip SHA for auto-refresh change detection
    let ref_name = format!("refs/forum/threads/{thread_id}");
    app.thread_tip_sha = git.resolve_ref(&ref_name)?.or(None);
    app.last_refresh = Instant::now();
    app.select_node_by_id(selected_node_id);
    app.view = View::ThreadDetail(thread_id.to_string());
    Ok(())
}

fn open_node_detail(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
) -> ForumResult<()> {
    let lookup = thread::find_node_in_thread(git, thread_id, node_id)?;
    app.node_detail_text = show::render_node_show(&lookup);
    app.node_detail_scroll = 0;
    app.view = View::NodeDetail {
        thread_id: thread_id.to_string(),
        node_id: lookup.node.node_id,
    };
    Ok(())
}

fn apply_node_status_action(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    action: NodeStatusAction,
) -> ForumResult<()> {
    let lookup = thread::find_node_in_thread(git, thread_id, node_id)?;
    let actor = actor::current_actor(git);
    let clock = SystemClock;
    let ids = UlidGenerator;

    match action {
        NodeStatusAction::Resolve if !lookup.node.resolved && !lookup.node.retracted => {
            say::resolve_node(git, thread_id, &lookup.node.node_id, &actor, &clock, &ids)?;
        }
        NodeStatusAction::Reopen
            if lookup.node.resolved || lookup.node.retracted || lookup.node.incorporated =>
        {
            say::reopen_node(git, thread_id, &lookup.node.node_id, &actor, &clock, &ids)?;
        }
        NodeStatusAction::Retract if !lookup.node.retracted => {
            say::retract_node(git, thread_id, &lookup.node.node_id, &actor, &clock, &ids)?;
        }
        _ => {}
    }

    open_node_detail(app, git, thread_id, &lookup.node.node_id)
}

#[doc(hidden)]
pub fn load_threads(git: &GitOps, db_path: &Path) -> ForumResult<Vec<ThreadRow>> {
    reindex::run_reindex(git, db_path)?;
    let conn = index::open_db(db_path)?;
    index::list_threads(&conn)
}

fn short_id(id: &str) -> String {
    id[..id.len().min(16)].to_string()
}

fn form_line(active: bool, label: &str, value: &str) -> String {
    let marker = if active { ">" } else { " " };
    format!("{marker} {label}: {value}")
}

fn single_line_preview(s: &str, max: usize) -> String {
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

fn node_status(node: &Node) -> &'static str {
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
        "proposed" | "under-review" => Color::Yellow,
        "accepted" | "closed" => Color::Magenta,
        "rejected" => Color::Red,
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

/// Build tree-ordered entries from a flat list of nodes using reply_to relationships.
///
/// Returns entries in depth-first order with tree connector prefixes.
fn build_tree_entries(nodes: &[Node]) -> Vec<TreeEntry> {
    use std::collections::HashMap;

    // Build index: node_id -> position
    let id_to_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.node_id.as_str(), i))
        .collect();

    // Build children map: parent_id -> [child indices]
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut has_parent = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if let Some(ref parent_id) = node.reply_to {
            if let Some(&parent_idx) = id_to_idx.get(parent_id.as_str()) {
                children.entry(parent_idx).or_default().push(i);
                has_parent[i] = true;
            }
        }
    }

    // Roots are nodes without a parent (or whose parent is not in this thread)
    let roots: Vec<usize> = (0..nodes.len()).filter(|&i| !has_parent[i]).collect();

    let mut entries = Vec::with_capacity(nodes.len());

    // DFS with prefix tracking
    fn walk(
        idx: usize,
        depth: u16,
        prefix: &str,
        is_last: bool,
        children: &HashMap<usize, Vec<usize>>,
        entries: &mut Vec<TreeEntry>,
    ) {
        let connector = if depth == 0 {
            String::new()
        } else if is_last {
            format!("{prefix}└─")
        } else {
            format!("{prefix}├─")
        };
        entries.push(TreeEntry {
            node_index: idx,
            depth,
            prefix: connector,
        });

        if let Some(child_indices) = children.get(&idx) {
            let n = child_indices.len();
            for (i, &child_idx) in child_indices.iter().enumerate() {
                let child_prefix = if depth == 0 {
                    String::new()
                } else if is_last {
                    format!("{prefix}  ")
                } else {
                    format!("{prefix}│ ")
                };
                walk(
                    child_idx,
                    depth + 1,
                    &child_prefix,
                    i == n - 1,
                    children,
                    entries,
                );
            }
        }
    }

    let n = roots.len();
    for (i, &root_idx) in roots.iter().enumerate() {
        walk(root_idx, 0, "", i == n - 1, &children, &mut entries);
    }

    entries
}

fn thread_kind_values() -> [ThreadKind; 2] {
    [ThreadKind::Issue, ThreadKind::Rfc]
}

fn thread_kind_labels() -> [&'static str; 2] {
    ["issue", "rfc"]
}

fn default_thread_kind_index(kind_filter: Option<&str>) -> usize {
    match kind_filter {
        Some("issue") => 0,
        _ => 1,
    }
}

fn node_type_values() -> [NodeType; 9] {
    [
        NodeType::Claim,
        NodeType::Question,
        NodeType::Objection,
        NodeType::Alternative,
        NodeType::Evidence,
        NodeType::Summary,
        NodeType::Action,
        NodeType::Risk,
        NodeType::Assumption,
    ]
}

fn node_type_labels() -> [&'static str; 9] {
    [
        "claim",
        "question",
        "objection",
        "alternative",
        "evidence",
        "summary",
        "action",
        "risk",
        "assumption",
    ]
}

fn link_relation_labels() -> [&'static str; 4] {
    ["implements", "relates-to", "depends-on", "blocks"]
}

fn link_target_kind_values() -> [LinkTargetKind; 3] {
    [
        LinkTargetKind::Issue,
        LinkTargetKind::Rfc,
        LinkTargetKind::Manual,
    ]
}

fn link_target_kind_labels() -> [&'static str; 3] {
    ["issue", "rfc", "manual"]
}

fn thread_kind_matches_target(kind: &str, target_kind: LinkTargetKind) -> bool {
    match target_kind {
        LinkTargetKind::Issue => kind == "issue",
        LinkTargetKind::Rfc => kind == "rfc",
        LinkTargetKind::Manual => false,
    }
}

fn auto_link_candidates<'a>(app: &'a App, source_thread_id: &str) -> Vec<&'a ThreadRow> {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    if target_kind == LinkTargetKind::Manual {
        return Vec::new();
    }

    app.threads
        .iter()
        .filter(|row| {
            row.id != source_thread_id && thread_kind_matches_target(&row.kind, target_kind)
        })
        .collect()
}

fn selected_link_target(app: &App, source_thread_id: &str) -> Option<String> {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    match target_kind {
        LinkTargetKind::Manual => {
            let target = app.link_form.manual_target.trim();
            (!target.is_empty()).then(|| target.to_string())
        }
        _ => auto_link_candidates(app, source_thread_id)
            .get(app.link_form.target_index)
            .map(|row| row.id.clone()),
    }
}

fn selected_link_target_label(app: &App, source_thread_id: &str) -> String {
    let target_kind = link_target_kind_values()[app.link_form.target_kind_index];
    match target_kind {
        LinkTargetKind::Manual => {
            let target = app.link_form.manual_target.trim();
            if target.is_empty() {
                "(enter thread id)".to_string()
            } else {
                target.to_string()
            }
        }
        _ => {
            let candidates = auto_link_candidates(app, source_thread_id);
            candidates
                .get(app.link_form.target_index)
                .map(|row| format!("{}  {}", row.id, single_line_preview(&row.title, 28)))
                .unwrap_or_else(|| "(no matching threads)".to_string())
        }
    }
}

fn handle_create_thread_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
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
            ThreadFormField::Submit => submit_create_thread(app, git, conn, db_path)?,
            ThreadFormField::Kind | ThreadFormField::Title => {}
        },
        _ => {}
    }
    Ok(())
}

fn handle_edit_thread_body_key(app: &mut App, key: crossterm::event::KeyEvent) -> ForumResult<()> {
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

fn handle_create_node_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    thread_id: &str,
) -> ForumResult<()> {
    match key.code {
        KeyCode::Esc => open_thread_detail(app, git, thread_id, None)?,
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
                submit_create_node(app, git, conn, db_path, thread_id)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_edit_node_body_key(
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

fn return_from_link_form(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
) -> ForumResult<()> {
    match origin {
        LinkOrigin::ThreadDetail { selected_node_id } => {
            open_thread_detail(app, git, thread_id, selected_node_id.as_deref())
        }
        LinkOrigin::NodeDetail { node_id } => open_node_detail(app, git, thread_id, node_id),
    }
}

fn handle_create_link_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
) -> ForumResult<()> {
    match key.code {
        KeyCode::Esc => return_from_link_form(app, git, thread_id, origin)?,
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
                submit_create_link(app, git, thread_id, origin)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn submit_create_thread(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
) -> ForumResult<()> {
    let title = app.thread_form.title.trim();
    if title.is_empty() {
        return Ok(());
    }

    let actor = actor::current_actor(git);
    let clock = SystemClock;
    let ids = UlidGenerator;
    let kind = thread_kind_values()[app.thread_form.kind_index];
    let body = if app.thread_form.body.trim().is_empty() {
        None
    } else {
        Some(app.thread_form.body.trim())
    };

    let thread_id = create::create_thread(git, kind, title, body, &actor, &clock, &ids)?;
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    if let Some(pos) = app.threads.iter().position(|row| row.id == thread_id) {
        app.table_state.select(Some(pos));
    }
    open_thread_detail(app, git, &thread_id, None)
}

fn submit_create_node(
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
    thread_id: &str,
) -> ForumResult<()> {
    let body = app.node_form.body.trim();
    if body.is_empty() {
        return Ok(());
    }

    let actor = actor::current_actor(git);
    let clock = SystemClock;
    let ids = UlidGenerator;
    let node_type = node_type_values()[app.node_form.node_type_index];
    let node_id = say::say_node(git, thread_id, node_type, body, &actor, &clock, &ids, None)?;
    reindex::run_reindex(git, db_path)?;
    app.threads = index::list_threads(conn)?;
    open_thread_detail(app, git, thread_id, Some(&node_id))
}

fn submit_create_link(
    app: &mut App,
    git: &GitOps,
    thread_id: &str,
    origin: &LinkOrigin,
) -> ForumResult<()> {
    let Some(target_thread_id) = selected_link_target(app, thread_id) else {
        return Ok(());
    };

    let actor = actor::current_actor(git);
    let clock = SystemClock;
    let relation = link_relation_labels()[app.link_form.relation_index];
    evidence_ops::add_thread_link(git, thread_id, &target_thread_id, relation, &actor, &clock)?;
    return_from_link_form(app, git, thread_id, origin)
}

/// Render the current app state into `frame`.
pub fn render(f: &mut Frame, app: &mut App) {
    app.ui_rects = UiRects::default();
    match app.view {
        View::List => render_list(f, f.area(), app),
        View::ThreadDetail(_) => render_thread_detail(f, f.area(), app),
        View::NodeDetail { .. } => render_node_detail(f, f.area(), app),
        View::CreateThread => render_create_thread(f, f.area(), app),
        View::EditThreadBody => render_edit_thread_body(f, f.area(), app),
        View::CreateNode { .. } => render_create_node(f, f.area(), app),
        View::EditNodeBody { .. } => render_edit_node_body(f, f.area(), app),
        View::CreateLink { .. } => render_create_link(f, f.area(), app),
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

    let filter_label = app.kind_filter.as_deref().unwrap_or("all");
    let help_text = format!(
        " [q]quit  [enter]detail  [c]create thread  [r]refresh  [f]filter:{filter_label}  [j/k]navigate"
    );
    // Track filter label position for mouse click cycling
    let filter_prefix = " [q]quit  [enter]detail  [c]create thread  [r]refresh  ";
    let filter_start = filter_prefix.len() as u16;
    let filter_len = format!("[f]filter:{filter_label}").len() as u16;
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
    let select_hint = if app.mouse_capture_disabled {
        " SELECT MODE"
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(format!(
            " [esc/q]back [enter]node [c]create [l]link [m]{md_indicator} [S]select [r]refresh [j/k]nodes{select_hint}",
        )),
        chunks[0],
    );

    let thread_id = if let View::ThreadDetail(ref id) = app.view {
        id.as_str()
    } else {
        ""
    };
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

    let body_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {thread_id} "));
    if app.markdown_mode {
        let md_text = markdown_to_text(app.thread_text.as_str());
        f.render_widget(
            Paragraph::new(md_text)
                .block(body_block)
                .wrap(Wrap { trim: false })
                .scroll((app.thread_scroll, 0)),
            main[0],
        );
    } else {
        f.render_widget(
            Paragraph::new(app.thread_text.as_str())
                .block(body_block)
                .wrap(Wrap { trim: false })
                .scroll((app.thread_scroll, 0)),
            main[0],
        );
    }

    let rows: Vec<Row> = app
        .tree_entries
        .iter()
        .map(|entry| {
            let node = &app.thread_nodes[entry.node_index];
            let type_str = node.node_type.to_string();
            let status_str = node_status(node);
            let dim = node_row_modifier(node);
            // Prefix the type column with tree connectors for replies
            let type_display = if entry.prefix.is_empty() {
                type_str.clone()
            } else {
                format!("{}{}", entry.prefix, type_str)
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
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" nodes ({}) ", app.thread_nodes.len())),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(table, main[1], &mut app.node_table_state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ratatui::backend::TestBackend;
    use tempfile::TempDir;

    fn make_row(id: &str, kind: &str, status: &str, title: &str) -> ThreadRow {
        ThreadRow {
            id: id.into(),
            kind: kind.into(),
            status: status.into(),
            title: title.into(),
            body: None,
            branch: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            created_by: "human/alice".into(),
            open_objections: 0,
            open_actions: 0,
            has_summary: false,
            tip_sha: "abc".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let w = width as usize;
        (0..height)
            .map(|y| {
                buf.content()[(y as usize * w)..((y as usize + 1) * w)]
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn setup_repo() -> (
        TempDir,
        GitOps,
        crate::internal::config::RepoPaths,
        rusqlite::Connection,
        std::path::PathBuf,
    ) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        let init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(init.status.success());

        for (key, val) in [
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ] {
            let status = std::process::Command::new("git")
                .args(["config", key, val])
                .current_dir(&path)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(status.status.success());
        }

        let git = GitOps::new(path.clone());
        let repo_paths = crate::internal::config::RepoPaths::from_repo_root(&path);
        crate::internal::init::init_forum(&repo_paths).unwrap();
        let db_path = repo_paths.git_forum.join("index.db");
        let conn = index::open_db(&db_path).unwrap();
        (dir, git, repo_paths, conn, db_path)
    }

    #[test]
    fn list_view_contains_thread_id() {
        let mut app = App::new(vec![make_row("RFC-0001", "rfc", "draft", "Test RFC")]);
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("RFC-0001"), "expected RFC-0001 in:\n{out}");
    }

    #[test]
    fn list_view_contains_title() {
        let mut app = App::new(vec![make_row("RFC-0001", "rfc", "draft", "Test RFC")]);
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("Test RFC"), "expected 'Test RFC' in:\n{out}");
    }

    #[test]
    fn list_view_shows_thread_count() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
        ];
        let mut app = App::new(rows);
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("2 threads"), "expected '2 threads' in:\n{out}");
    }

    #[test]
    fn mouse_single_click_selects_row() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "Test RFC",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        reindex::run_reindex(&git, &db_path).unwrap();
        let mut app = App::new(index::list_threads(&conn).unwrap());
        let _ = render_to_string(&mut app, 80, 20);
        let area = app.ui_rects.list_table.unwrap();

        // Single click should select, not open
        handle_mouse(
            &mut app,
            mouse_event(
                MouseEventKind::Down(MouseButton::Left),
                area.x + 2,
                area.y + 2,
            ),
            &git,
            &conn,
            &db_path,
        )
        .unwrap();

        assert_eq!(app.view, View::List);
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn mouse_double_click_opens_thread_detail() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "Test RFC",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        reindex::run_reindex(&git, &db_path).unwrap();
        let mut app = App::new(index::list_threads(&conn).unwrap());
        let _ = render_to_string(&mut app, 80, 20);
        let area = app.ui_rects.list_table.unwrap();

        let click = mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 2,
            area.y + 2,
        );
        // First click selects
        handle_mouse(&mut app, click, &git, &conn, &db_path).unwrap();
        assert_eq!(app.view, View::List);
        // Second click (same position, quick) opens
        handle_mouse(&mut app, click, &git, &conn, &db_path).unwrap();
        assert_eq!(app.view, View::ThreadDetail("RFC-0001".into()));
    }

    #[test]
    fn click_column_header_sorts_list() {
        let mut row_a = make_row("ISSUE-0001", "issue", "open", "Alpha");
        row_a.created_at = "2026-01-01T00:00:00Z".into();
        row_a.updated_at = "2026-01-03T00:00:00Z".into();
        let mut row_b = make_row("RFC-0001", "rfc", "draft", "Beta");
        row_b.created_at = "2026-01-02T00:00:00Z".into();
        row_b.updated_at = "2026-01-02T00:00:00Z".into();
        let mut app = App::new(vec![row_a, row_b]);
        // Default sort: Updated descending → ISSUE-0001 first (updated 01-03)
        assert_eq!(app.visible_threads()[0].id, "ISSUE-0001");

        let _ = render_to_string(&mut app, 100, 20);

        // Click on the "CREATED" column header (4th column)
        let header_rect = app.ui_rects.column_headers[3].unwrap();
        let click = mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            header_rect.x + 1,
            header_rect.y,
        );
        handle_mouse(
            &mut app,
            click,
            &GitOps::new(std::path::PathBuf::from("/")),
            &rusqlite::Connection::open_in_memory().unwrap(),
            std::path::Path::new("/tmp/test.db"),
        )
        .unwrap();

        // Now sorted by Created ascending → ISSUE-0001 first (created 01-01)
        assert_eq!(app.sort_column, SortColumn::Created);
        assert!(app.sort_ascending);
        assert_eq!(app.visible_threads()[0].id, "ISSUE-0001");
        assert_eq!(app.visible_threads()[1].id, "RFC-0001");

        // Click same header again → toggles to descending
        handle_mouse(
            &mut app,
            click,
            &GitOps::new(std::path::PathBuf::from("/")),
            &rusqlite::Connection::open_in_memory().unwrap(),
            std::path::Path::new("/tmp/test.db"),
        )
        .unwrap();
        assert!(!app.sort_ascending);
        assert_eq!(app.visible_threads()[0].id, "RFC-0001");
    }

    #[test]
    fn list_view_filter_hides_non_matching() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
        ];
        let mut app = App::new(rows);
        app.kind_filter = Some("issue".into());
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("ISSUE-0001"));
        assert!(out.contains("1 threads"));
    }

    #[test]
    fn detail_view_shows_content() {
        let mut app = App::new(vec![]);
        app.view = View::ThreadDetail("RFC-0001".into());
        app.thread_text = "RFC-0001 Test RFC\nkind: rfc\n".into();
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("RFC-0001"));
    }

    #[test]
    fn thread_detail_arrow_keys_scroll_body() {
        let mut app = App::new(vec![]);
        app.view = View::ThreadDetail("RFC-0001".into());
        app.thread_text = (0..20)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.scroll_thread_down();
        app.scroll_thread_down();
        assert_eq!(app.thread_scroll, 2);
        app.scroll_thread_up();
        assert_eq!(app.thread_scroll, 1);
    }

    #[test]
    fn mouse_wheel_scrolls_thread_body() {
        let mut app = App::new(vec![]);
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        app.view = View::ThreadDetail("RFC-0001".into());
        app.thread_text = (0..20)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = render_to_string(&mut app, 120, 24);
        let area = app.ui_rects.thread_body.unwrap();

        handle_mouse(
            &mut app,
            mouse_event(MouseEventKind::ScrollDown, area.x + 1, area.y + 1),
            &git,
            &conn,
            dir.path(),
        )
        .unwrap();

        assert_eq!(app.thread_scroll, 1);
    }

    #[test]
    fn thread_detail_view_shows_nodes_table() {
        let mut app = App::new(vec![]);
        app.view = View::ThreadDetail("RFC-0001".into());
        app.thread_text = "RFC-0001 Test RFC\nkind: rfc\n".into();
        app.thread_nodes = vec![Node {
            node_id: "abcdef1234567890".into(),
            node_type: crate::internal::event::NodeType::Question,
            body: "What is this?".into(),
            actor: "human/alice".into(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            resolved: false,
            retracted: false,
            incorporated: false,
            reply_to: None,
        }];
        app.tree_entries = build_tree_entries(&app.thread_nodes);
        app.node_table_state.select(Some(0));
        let out = render_to_string(&mut app, 160, 30);
        assert!(out.contains("nodes (1)"));
        assert!(out.contains("abcdef12"));
        assert!(out.contains("What is this?"));
    }

    #[test]
    fn node_detail_view_shows_node_body() {
        let mut app = App::new(vec![]);
        app.view = View::NodeDetail {
            thread_id: "RFC-0001".into(),
            node_id: "abcdef123456".into(),
        };
        app.node_detail_text = "abcdef123456 question\nbody:\n  What is this?\n".into();
        let out = render_to_string(&mut app, 100, 20);
        assert!(out.contains("What is this?"));
        assert!(out.contains("node abcdef123456"));
        assert!(out.contains("[x]resolve"));
    }

    #[test]
    fn create_thread_view_shows_form_fields() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("create thread"));
        assert!(out.contains("kind: rfc"));
        assert!(out.contains("title:"));
        assert!(out.contains("body: (empty)"));
        assert!(out.contains("submit: [Create thread]"));
        assert!(out.contains("thread kinds"));
        assert!(out.contains("> rfc"));
    }

    #[test]
    fn create_node_view_shows_form_fields() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("create node"));
        assert!(out.contains("type: claim"));
        assert!(out.contains("body: (empty)"));
        assert!(out.contains("submit: [Create node]"));
        assert!(out.contains("node types"));
        assert!(out.contains("> claim"));
    }

    #[test]
    fn create_link_view_shows_auto_resolvable_targets() {
        let rows = vec![
            make_row("RFC-0001", "rfc", "draft", "Source RFC"),
            make_row("ISSUE-0001", "issue", "open", "Implement parser"),
            make_row("ISSUE-0002", "issue", "open", "Add tests"),
        ];
        let mut app = App::new(rows);
        app.begin_create_link_from_thread("RFC-0001");
        app.link_form.field = LinkFormField::Target;
        let out = render_to_string(&mut app, 120, 24);
        assert!(out.contains("create link from RFC-0001"));
        assert!(out.contains("relation: implements"));
        assert!(out.contains("target kind: issue"));
        assert!(out.contains("issue targets (2)"));
        assert!(out.contains("> ISSUE-0001  Implement parser"));
    }

    #[test]
    fn single_line_preview_handles_multibyte_text() {
        let preview =
            single_line_preview("実装開始: CMake + ImGui + GLFW スケルトンアプリの構築", 20);
        assert!(preview.starts_with("実装開始"));
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn edit_thread_body_view_shows_editor() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.body = "line 1\nline 2".into();
        app.view = View::EditThreadBody;
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("edit thread body"));
        assert!(out.contains("line 1"));
        assert!(out.contains("line 2"));
        assert!(out.contains("ctrl+s"));
    }

    #[test]
    fn edit_node_body_view_shows_editor() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");
        app.node_form.body = "line 1\nline 2".into();
        app.view = View::EditNodeBody {
            thread_id: "RFC-0001".into(),
        };
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("edit node body"));
        assert!(out.contains("line 1"));
        assert!(out.contains("line 2"));
        assert!(out.contains("ctrl+s"));
    }

    #[test]
    fn enter_on_body_field_opens_multiline_editor() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");
        app.node_form.field = NodeFormField::Body;

        handle_edit_transition_via_create_node(&mut app);

        assert_eq!(
            app.view,
            View::EditNodeBody {
                thread_id: "RFC-0001".into()
            }
        );
    }

    #[test]
    fn enter_on_type_field_does_not_submit() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");

        handle_edit_transition_via_create_node(&mut app);

        assert_eq!(
            app.view,
            View::CreateNode {
                thread_id: "RFC-0001".into()
            }
        );
        assert_eq!(app.node_form.field, NodeFormField::Type);
    }

    #[test]
    fn enter_on_thread_body_field_opens_multiline_editor() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.field = ThreadFormField::Body;

        handle_edit_transition_via_create_thread(&mut app);

        assert_eq!(app.view, View::EditThreadBody);
    }

    #[test]
    fn enter_on_thread_kind_field_does_not_submit() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();

        handle_edit_transition_via_create_thread(&mut app);

        assert_eq!(app.view, View::CreateThread);
        assert_eq!(app.thread_form.field, ThreadFormField::Kind);
    }

    fn handle_edit_transition_via_create_thread(app: &mut App) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        handle_create_thread_key(
            app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &conn,
            dir.path(),
        )
        .unwrap();
    }

    fn handle_edit_transition_via_create_node(app: &mut App) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        handle_create_node_key(
            app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &conn,
            dir.path(),
            "RFC-0001",
        )
        .unwrap();
    }

    #[test]
    fn edit_node_body_supports_multiline_text() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");
        app.view = View::EditNodeBody {
            thread_id: "RFC-0001".into(),
        };

        handle_edit_node_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE),
            "RFC-0001",
        )
        .unwrap();
        handle_edit_node_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            "RFC-0001",
        )
        .unwrap();
        handle_edit_node_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE),
            "RFC-0001",
        )
        .unwrap();
        handle_edit_node_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            "RFC-0001",
        )
        .unwrap();

        assert_eq!(app.node_form.body, "A\nB");
        assert_eq!(
            app.view,
            View::CreateNode {
                thread_id: "RFC-0001".into()
            }
        );
        assert_eq!(app.node_form.field, NodeFormField::Body);
    }

    #[test]
    fn edit_thread_body_supports_multiline_text() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.view = View::EditThreadBody;

        handle_edit_thread_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE),
        )
        .unwrap();
        handle_edit_thread_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();
        handle_edit_thread_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE),
        )
        .unwrap();
        handle_edit_thread_body_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        )
        .unwrap();

        assert_eq!(app.thread_form.body, "A\nB");
        assert_eq!(app.view, View::CreateThread);
        assert_eq!(app.thread_form.field, ThreadFormField::Body);
    }

    #[test]
    fn submit_create_thread_creates_thread_and_opens_detail() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.kind_index = 1;
        app.thread_form.title = "Created in TUI".into();
        app.thread_form.body = "Body from TUI".into();

        submit_create_thread(&mut app, &git, &conn, &db_path).unwrap();

        assert_eq!(app.view, View::ThreadDetail("RFC-0001".into()));
        assert!(app.thread_text.contains("Created in TUI"));
        let rows = index::list_threads(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Created in TUI");
    }

    #[test]
    fn mouse_click_on_thread_submit_creates_thread() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.title = "Created with mouse".into();
        app.thread_form.body = "Body from mouse".into();
        let _ = render_to_string(&mut app, 120, 24);
        let area = app.ui_rects.thread_submit.unwrap();

        handle_mouse(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), area.x + 1, area.y),
            &git,
            &conn,
            &db_path,
        )
        .unwrap();

        assert_eq!(app.view, View::ThreadDetail("RFC-0001".into()));
        assert!(app.thread_text.contains("Created with mouse"));
    }

    #[test]
    fn submit_create_node_adds_node_and_keeps_thread_detail() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        let thread_id = crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "RFC from setup",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        reindex::run_reindex(&git, &db_path).unwrap();

        let mut app = App::new(index::list_threads(&conn).unwrap());
        open_thread_detail(&mut app, &git, &thread_id, None).unwrap();
        app.begin_create_node(&thread_id);
        app.node_form.node_type_index = 1;
        app.node_form.body = "Node from TUI\nwith more detail".into();
        app.node_form.field = NodeFormField::Submit;
        handle_create_node_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &conn,
            &db_path,
            &thread_id,
        )
        .unwrap();

        assert_eq!(app.view, View::ThreadDetail(thread_id.clone()));
        assert_eq!(app.thread_nodes.len(), 1);
        assert_eq!(app.thread_nodes[0].body, "Node from TUI\nwith more detail");
        assert_eq!(app.node_table_state.selected(), Some(0));
    }

    #[test]
    fn submit_create_link_from_thread_adds_link_and_returns_to_thread_detail() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        let source_id = crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "Source RFC",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Issue,
            "Implementation issue",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("u"),
        )
        .unwrap();
        reindex::run_reindex(&git, &db_path).unwrap();

        let mut app = App::new(index::list_threads(&conn).unwrap());
        open_thread_detail(&mut app, &git, &source_id, None).unwrap();
        app.begin_create_link_from_thread(&source_id);
        app.link_form.field = LinkFormField::Submit;

        handle_create_link_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &source_id,
            &LinkOrigin::ThreadDetail {
                selected_node_id: None,
            },
        )
        .unwrap();

        assert_eq!(app.view, View::ThreadDetail(source_id.clone()));
        assert!(app.thread_text.contains("links: 1"));
        assert!(app.thread_text.contains("ISSUE-0001  implements"));
    }

    #[test]
    fn submit_create_link_from_node_returns_to_node_detail() {
        let (_dir, git, _paths, conn, db_path) = setup_repo();
        let source_id = crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "Source RFC",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Issue,
            "Implementation issue",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("u"),
        )
        .unwrap();
        let node_id = crate::internal::say::say_node(
            &git,
            &source_id,
            crate::internal::event::NodeType::Claim,
            "Investigate parser shape",
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 2, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("n"),
            None,
        )
        .unwrap();
        reindex::run_reindex(&git, &db_path).unwrap();

        let mut app = App::new(index::list_threads(&conn).unwrap());
        open_node_detail(&mut app, &git, &source_id, &node_id).unwrap();
        app.begin_create_link_from_node(&source_id, &node_id);
        app.link_form.field = LinkFormField::Submit;

        handle_create_link_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &source_id,
            &LinkOrigin::NodeDetail {
                node_id: node_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(
            app.view,
            View::NodeDetail {
                thread_id: source_id.clone(),
                node_id: node_id.clone(),
            }
        );
        assert!(app.node_detail_text.contains("thread links: 1"));
        assert!(app.node_detail_text.contains("ISSUE-0001  implements"));
    }

    #[test]
    fn apply_node_status_action_updates_node_detail() {
        let (_dir, git, _paths, _conn, _db_path) = setup_repo();
        let thread_id = crate::internal::create::create_thread(
            &git,
            crate::internal::event::ThreadKind::Rfc,
            "RFC from setup",
            None,
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("t"),
        )
        .unwrap();
        let node_id = crate::internal::say::say_node(
            &git,
            &thread_id,
            crate::internal::event::NodeType::Objection,
            "Needs more evidence",
            "human/test-user",
            &crate::internal::clock::FixedClock {
                instant: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            },
            &crate::internal::id::SequentialIdGenerator::new("n"),
            None,
        )
        .unwrap();

        let mut app = App::new(vec![]);
        open_node_detail(&mut app, &git, &thread_id, &node_id).unwrap();
        apply_node_status_action(
            &mut app,
            &git,
            &thread_id,
            &node_id,
            NodeStatusAction::Resolve,
        )
        .unwrap();
        assert!(app.node_detail_text.contains("status:   resolved"));

        apply_node_status_action(
            &mut app,
            &git,
            &thread_id,
            &node_id,
            NodeStatusAction::Reopen,
        )
        .unwrap();
        assert!(app.node_detail_text.contains("status:   open"));

        apply_node_status_action(
            &mut app,
            &git,
            &thread_id,
            &node_id,
            NodeStatusAction::Retract,
        )
        .unwrap();
        assert!(app.node_detail_text.contains("status:   retracted"));
    }

    #[test]
    fn cycle_filter_cycles_correctly() {
        let mut app = App::new(vec![]);
        assert_eq!(app.kind_filter, None);
        app.cycle_filter();
        assert_eq!(app.kind_filter.as_deref(), Some("issue"));
        app.cycle_filter();
        assert_eq!(app.kind_filter.as_deref(), Some("rfc"));
        app.cycle_filter();
        assert_eq!(app.kind_filter, None);
    }

    #[test]
    fn move_down_wraps_at_end() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "A"),
            make_row("RFC-0001", "rfc", "draft", "B"),
        ];
        let mut app = App::new(rows);
        app.move_down();
        assert_eq!(app.table_state.selected(), Some(1));
        app.move_down(); // already at last
        assert_eq!(app.table_state.selected(), Some(1));
    }

    #[test]
    fn move_node_down_stops_at_end() {
        let mut app = App::new(vec![]);
        app.thread_nodes = vec![
            Node {
                node_id: "a".into(),
                node_type: crate::internal::event::NodeType::Question,
                body: "A".into(),
                actor: "human/alice".into(),
                created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                resolved: false,
                retracted: false,
                incorporated: false,
                reply_to: None,
            },
            Node {
                node_id: "b".into(),
                node_type: crate::internal::event::NodeType::Question,
                body: "B".into(),
                actor: "human/alice".into(),
                created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                resolved: false,
                retracted: false,
                incorporated: false,
                reply_to: None,
            },
        ];
        app.tree_entries = build_tree_entries(&app.thread_nodes);
        app.node_table_state.select(Some(0));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
    }
}
