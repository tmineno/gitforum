mod input;
mod markdown;
pub(crate) mod render;
mod state;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use ratatui::Terminal;

use super::error::ForumResult;
use super::git_ops::GitOps;
use super::index::{self, ThreadRow};
use super::node::Node;

use input::{handle_key, handle_mouse};
use render::render;
use state::{auto_refresh, default_thread_kind_index};

// Re-export for external tests
#[doc(hidden)]
pub use state::load_threads;

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
enum FilterField {
    Kind,
    Status,
}

#[derive(Debug, Clone, Default)]
struct FilterCriteria {
    kind: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone)]
struct FilterBar {
    field: FilterField,
    kind_index: usize,
    status_index: usize,
}

const FILTER_KIND_LABELS: [&str; 3] = ["all", "issue", "rfc"];
const FILTER_STATUS_LABELS: [&str; 10] = [
    "all",
    "open",
    "draft",
    "pending",
    "proposed",
    "under-review",
    "accepted",
    "closed",
    "rejected",
    "deprecated",
];

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
    /// Filter popup area (when open).
    filter_popup: Option<Rect>,
    /// Filter kind list area (inside popup).
    filter_kind_area: Option<Rect>,
    /// Filter status list area (inside popup).
    filter_status_area: Option<Rect>,
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
    /// Whether this node has child replies.
    has_children: bool,
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub threads: Vec<ThreadRow>,
    pub table_state: TableState,
    filter: FilterCriteria,
    filter_bar: Option<FilterBar>,
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
    /// Node IDs whose subtrees are collapsed in the tree view.
    collapsed: HashSet<String>,
    /// Maps visible row index -> index in `tree_entries` (accounts for collapsed subtrees).
    visible_tree_indices: Vec<usize>,
    /// Whether the tree pane is shown full-width (body pane hidden).
    tree_fullscreen: bool,
    /// Saved detail_split value to restore when leaving fullscreen tree mode.
    saved_detail_split: u16,
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
            filter: FilterCriteria::default(),
            filter_bar: None,
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
            collapsed: HashSet::new(),
            visible_tree_indices: Vec::new(),
            tree_fullscreen: false,
            saved_detail_split: 60,
        }
    }

    pub fn visible_threads(&self) -> Vec<&ThreadRow> {
        let mut rows: Vec<&ThreadRow> = self
            .threads
            .iter()
            .filter(|t| {
                let kind_ok = self
                    .filter
                    .kind
                    .as_deref()
                    .map(|k| t.kind == k)
                    .unwrap_or(true);
                let status_ok = self
                    .filter
                    .status
                    .as_deref()
                    .map(|s| t.status == s)
                    .unwrap_or(true);
                kind_ok && status_ok
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

    fn open_filter_bar(&mut self) {
        let kind_index = match self.filter.kind.as_deref() {
            None => 0,
            Some("issue") => 1,
            Some("rfc") => 2,
            _ => 0,
        };
        let status_index = FILTER_STATUS_LABELS
            .iter()
            .position(|&s| Some(s) == self.filter.status.as_deref())
            .unwrap_or(0);
        self.filter_bar = Some(FilterBar {
            field: FilterField::Kind,
            kind_index,
            status_index,
        });
    }

    fn apply_filter_bar(&mut self) {
        if let Some(bar) = self.filter_bar.take() {
            self.filter.kind = if bar.kind_index == 0 {
                None
            } else {
                Some(FILTER_KIND_LABELS[bar.kind_index].to_string())
            };
            self.filter.status = if bar.status_index == 0 {
                None
            } else {
                Some(FILTER_STATUS_LABELS[bar.status_index].to_string())
            };
            let n = self.visible_threads().len();
            self.table_state.select(if n > 0 { Some(0) } else { None });
        }
    }

    fn cancel_filter_bar(&mut self) {
        self.filter_bar = None;
    }

    fn clear_filter_bar(&mut self) {
        self.filter_bar = None;
        self.filter.kind = None;
        self.filter.status = None;
        let n = self.visible_threads().len();
        self.table_state.select(if n > 0 { Some(0) } else { None });
    }

    fn column_header_at(&self, column: u16, row: u16) -> Option<SortColumn> {
        for (i, rect) in self.ui_rects.column_headers.iter().enumerate() {
            if let Some(area) = rect {
                if input::rect_contains(*area, column, row) {
                    return Some(SORT_COLUMNS[i]);
                }
            }
        }
        None
    }

    fn selected_node_id(&self) -> Option<String> {
        self.node_table_state
            .selected()
            .and_then(|i| self.visible_tree_indices.get(i))
            .and_then(|&ti| self.tree_entries.get(ti))
            .map(|entry| self.thread_nodes[entry.node_index].node_id.clone())
    }

    fn move_node_down(&mut self) {
        let n = self.visible_tree_indices.len();
        if n == 0 {
            return;
        }
        let next = self
            .node_table_state
            .selected()
            .map(|i| (i + 1).min(n - 1))
            .unwrap_or(0);
        self.node_table_state.select(Some(next));
        self.thread_scroll = 0;
    }

    fn move_node_up(&mut self) {
        let n = self.visible_tree_indices.len();
        if n == 0 {
            return;
        }
        let next = self
            .node_table_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.node_table_state.select(Some(next));
        self.thread_scroll = 0;
    }

    fn select_node_by_id(&mut self, node_id: Option<&str>) {
        let selected = node_id.and_then(|id| {
            self.visible_tree_indices
                .iter()
                .position(|&ti| self.thread_nodes[self.tree_entries[ti].node_index].node_id == id)
        });
        self.node_table_state
            .select(match (selected, self.visible_tree_indices.is_empty()) {
                (Some(index), _) => Some(index),
                (None, false) => Some(0),
                (None, true) => None,
            });
    }

    /// Recompute which tree entries are visible based on collapsed state.
    fn recompute_visible_tree(&mut self) {
        let mut visible = Vec::with_capacity(self.tree_entries.len());
        let mut skip_depth: Option<u16> = None;
        for (i, entry) in self.tree_entries.iter().enumerate() {
            if let Some(sd) = skip_depth {
                if entry.depth > sd {
                    continue;
                }
                skip_depth = None;
            }
            let node_id = &self.thread_nodes[entry.node_index].node_id;
            if entry.has_children && self.collapsed.contains(node_id) {
                skip_depth = Some(entry.depth);
            }
            visible.push(i);
        }
        self.visible_tree_indices = visible;
    }

    /// Toggle collapsed state for the currently selected node.
    fn toggle_collapse(&mut self) {
        let node_id = match self.selected_node_id() {
            Some(id) => id,
            None => return,
        };
        // Only toggle if the node has children
        let has_children = self
            .node_table_state
            .selected()
            .and_then(|i| self.visible_tree_indices.get(i))
            .and_then(|&ti| self.tree_entries.get(ti))
            .map(|e| e.has_children)
            .unwrap_or(false);
        if !has_children {
            return;
        }
        if self.collapsed.contains(&node_id) {
            self.collapsed.remove(&node_id);
        } else {
            self.collapsed.insert(node_id.clone());
        }
        // Preserve selection on the same node after recompute
        self.recompute_visible_tree();
        self.select_node_by_id(Some(&node_id));
    }

    fn scroll_thread_down(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_add(1);
    }

    fn scroll_thread_up(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_sub(1);
    }

    fn begin_create_thread(&mut self) {
        self.thread_form = ThreadForm {
            kind_index: default_thread_kind_index(self.filter.kind.as_deref()),
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

/// Auto-refresh interval: check for thread changes every 2 seconds.
const AUTO_REFRESH_INTERVAL_MS: u128 = 2000;

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
        state::open_thread_detail(&mut app, git, thread_id, None)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::backend::TestBackend;
    use tempfile::TempDir;

    use crate::internal::index;
    use crate::internal::reindex;

    use super::input::{
        handle_create_node_key, handle_create_thread_key, handle_edit_node_body_key,
        handle_edit_thread_body_key, handle_mouse,
    };
    use super::state::{
        apply_node_status_action, build_tree_entries, open_node_detail, open_thread_detail,
        submit_create_thread,
    };
    use input::handle_create_link_key;

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
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
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
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
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
        app.filter.kind = Some("issue".into());
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
        app.recompute_visible_tree();
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
        let preview = render::single_line_preview(
            "実装開始: CMake + ImGui + GLFW スケルトンアプリの構築",
            20,
        );
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
    fn filter_bar_apply_sets_kind_and_status() {
        let mut app = App::new(vec![]);
        assert_eq!(app.filter.kind, None);
        assert_eq!(app.filter.status, None);

        app.open_filter_bar();
        assert!(app.filter_bar.is_some());

        // Select "issue" (index 1) for kind
        if let Some(ref mut bar) = app.filter_bar {
            bar.kind_index = 1;
            bar.status_index = 2; // "draft"
        }
        app.apply_filter_bar();

        assert_eq!(app.filter.kind.as_deref(), Some("issue"));
        assert_eq!(app.filter.status.as_deref(), Some("draft"));
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_cancel_preserves_existing_filter() {
        let mut app = App::new(vec![]);
        app.filter.kind = Some("rfc".into());
        app.open_filter_bar();
        if let Some(ref mut bar) = app.filter_bar {
            bar.kind_index = 1; // "issue"
        }
        app.cancel_filter_bar();
        assert_eq!(app.filter.kind.as_deref(), Some("rfc"));
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_clear_resets_both_dimensions() {
        let mut app = App::new(vec![]);
        app.filter.kind = Some("issue".into());
        app.filter.status = Some("open".into());
        app.open_filter_bar();
        app.clear_filter_bar();
        assert_eq!(app.filter.kind, None);
        assert_eq!(app.filter.status, None);
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_open_reflects_current_filter() {
        let mut app = App::new(vec![]);
        app.filter.kind = Some("rfc".into());
        app.filter.status = Some("pending".into());
        app.open_filter_bar();
        let bar = app.filter_bar.as_ref().unwrap();
        assert_eq!(bar.kind_index, 2); // "rfc"
        assert_eq!(bar.status_index, 3); // "pending"
    }

    #[test]
    fn visible_threads_filters_by_status() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
            make_row("ISSUE-0002", "issue", "closed", "Old bug"),
        ];
        let mut app = App::new(rows);
        app.filter.status = Some("open".into());
        let visible = app.visible_threads();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "ISSUE-0001");
    }

    #[test]
    fn visible_threads_filters_by_kind_and_status() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "open", "Proposal"),
            make_row("ISSUE-0002", "issue", "closed", "Old bug"),
        ];
        let mut app = App::new(rows);
        app.filter.kind = Some("issue".into());
        app.filter.status = Some("open".into());
        let visible = app.visible_threads();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "ISSUE-0001");
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
        app.recompute_visible_tree();
        app.node_table_state.select(Some(0));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
    }
}
