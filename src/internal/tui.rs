use std::path::Path;

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
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, TableState};
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
    pub node_table_state: TableState,
    pub node_detail_text: String,
    pub node_detail_scroll: u16,
    thread_form: ThreadForm,
    node_form: NodeForm,
    link_form: LinkForm,
    ui_rects: UiRects,
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
        }
    }

    pub fn visible_threads(&self) -> Vec<&ThreadRow> {
        self.threads
            .iter()
            .filter(|t| {
                self.kind_filter
                    .as_deref()
                    .map(|k| t.kind == k)
                    .unwrap_or(true)
            })
            .collect()
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

    fn selected_node_id(&self) -> Option<String> {
        self.node_table_state
            .selected()
            .and_then(|i| self.thread_nodes.get(i))
            .map(|node| node.node_id.clone())
    }

    fn move_node_down(&mut self) {
        let n = self.thread_nodes.len();
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
        let n = self.thread_nodes.len();
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
        let selected =
            node_id.and_then(|id| self.thread_nodes.iter().position(|n| n.node_id == id));
        self.node_table_state
            .select(match (selected, self.thread_nodes.is_empty()) {
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

pub(crate) fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
    db_path: &Path,
) -> ForumResult<()> {
    loop {
        terminal.draw(|f| render(f, app))?;

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
                app.node_detail_text.clear();
                app.node_detail_scroll = 0;
            }
            KeyCode::Char('j') => app.move_node_down(),
            KeyCode::Char('k') => app.move_node_up(),
            KeyCode::Down => app.scroll_thread_down(),
            KeyCode::Up => app.scroll_thread_up(),
            KeyCode::Char('c') => app.begin_create_node(&thread_id),
            KeyCode::Char('l') => app.begin_create_link_from_thread(&thread_id),
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
                if let Some(area) = app.ui_rects.list_table {
                    if let Some(index) = table_row_at(area, mouse.row) {
                        let visible_len = app.visible_threads().len();
                        if index < visible_len {
                            app.table_state.select(Some(index));
                            if let Some(thread_id) = app.selected_thread_id() {
                                open_thread_detail(app, git, &thread_id, None)?;
                            }
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => app.move_down(),
            MouseEventKind::ScrollUp => app.move_up(),
            _ => {}
        },
        View::ThreadDetail(thread_id) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(area) = app.ui_rects.thread_nodes {
                    if let Some(index) = table_row_at(area, mouse.row) {
                        if index < app.thread_nodes.len() {
                            app.node_table_state.select(Some(index));
                            if let Some(node_id) = app.selected_node_id() {
                                open_node_detail(app, git, &thread_id, &node_id)?;
                            }
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(area) = app.ui_rects.thread_body {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.scroll_thread_down();
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(area) = app.ui_rects.thread_body {
                    if rect_contains(area, mouse.column, mouse.row) {
                        app.scroll_thread_up();
                    }
                }
            }
            _ => {}
        },
        View::NodeDetail { .. } => match mouse.kind {
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
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && app
                    .ui_rects
                    .thread_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
            {
                app.thread_form.field = ThreadFormField::Submit;
                submit_create_thread(app, git, _conn, _db_path)?;
            }
        }
        View::CreateNode { thread_id } => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && app
                    .ui_rects
                    .node_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
            {
                app.node_form.field = NodeFormField::Submit;
                submit_create_node(app, git, _conn, _db_path, &thread_id)?;
            }
        }
        View::CreateLink { thread_id, origin } => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && app
                    .ui_rects
                    .link_submit
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
            {
                app.link_form.field = LinkFormField::Submit;
                submit_create_link(app, git, &thread_id, &origin)?;
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
    app.node_detail_text.clear();
    app.node_detail_scroll = 0;
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
        NodeStatusAction::Reopen if lookup.node.resolved || lookup.node.retracted => {
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

fn node_status(node: &Node) -> &'static str {
    if node.retracted {
        "retracted"
    } else if node.resolved {
        "resolved"
    } else {
        "open"
    }
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
    let node_id = say::say_node(git, thread_id, node_type, body, &actor, &clock, &ids)?;
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
    let help = Line::from(format!(
        " [q]quit  [enter]detail  [c]create thread  [r]refresh  [f]filter:{filter_label}  [j/k]navigate"
    ));
    f.render_widget(Paragraph::new(help), chunks[0]);

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
                    Cell::from(t.kind.clone()),
                    Cell::from(t.status.clone()),
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
        Constraint::Min(20),
    ];
    let header = Row::new(["ID", "KIND", "STATUS", "TITLE"])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" git-forum "))
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );

    app.ui_rects.list_table = Some(chunks[1]);
    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    f.render_widget(Paragraph::new(format!(" {count} threads")), chunks[2]);
}

pub(crate) fn render_thread_detail(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(
        Paragraph::new(
            " [esc/q]back  [enter]node  [c]create node  [l]link  [r]refresh  [j/k]select node  [up/down]scroll body",
        ),
        chunks[0],
    );

    let thread_id = if let View::ThreadDetail(ref id) = app.view {
        id.as_str()
    } else {
        ""
    };
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    app.ui_rects.thread_body = Some(main[0]);
    app.ui_rects.thread_nodes = Some(main[1]);

    f.render_widget(
        Paragraph::new(app.thread_text.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {thread_id} ")),
            )
            .scroll((app.thread_scroll, 0)),
        main[0],
    );

    let rows: Vec<Row> = app
        .thread_nodes
        .iter()
        .map(|node| {
            Row::new(vec![
                Cell::from(short_id(&node.node_id)),
                Cell::from(node.node_type.to_string()),
                Cell::from(node_status(node)),
                Cell::from(single_line_preview(&node.body, 36)),
            ])
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

    f.render_widget(
        Paragraph::new(
            " [esc/q]back  [c]create node  [l]link  [x]resolve  [o]reopen  [R]retract  [r]refresh  [j/k]scroll",
        ),
        chunks[0],
    );

    let title = if let View::NodeDetail { ref node_id, .. } = app.view {
        short_id(node_id)
    } else {
        String::new()
    };
    app.ui_rects.node_detail = Some(chunks[1]);
    f.render_widget(
        Paragraph::new(app.node_detail_text.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" node {title} ")),
            )
            .scroll((app.node_detail_scroll, 0)),
        chunks[1],
    );
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
    fn mouse_click_on_list_row_opens_thread_detail() {
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

        assert_eq!(app.view, View::ThreadDetail("RFC-0001".into()));
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
        }];
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
            },
            Node {
                node_id: "b".into(),
                node_type: crate::internal::event::NodeType::Question,
                body: "B".into(),
                actor: "human/alice".into(),
                created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                resolved: false,
                retracted: false,
            },
        ];
        app.node_table_state.select(Some(0));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
    }
}
