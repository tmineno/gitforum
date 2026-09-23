mod effects;
mod input;
mod markdown;
mod persist;
pub(crate) mod render;
mod state;
#[cfg(test)]
mod ux_checks;
#[cfg(test)]
mod ux_fixture;
#[cfg(test)]
mod ux_invariants;
#[cfg(test)]
mod ux_mouse;
#[cfg(test)]
mod ux_scroll;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use ratatui::Terminal;

use super::error::{ForumError, ForumResult};
use super::git_ops::GitOps;
use super::snapshot::list::ThreadRow;
use super::snapshot::store::NodeWithBody;

use input::{handle_key, handle_mouse};
use render::render;

/// Number of lines/rows to scroll per PageUp/PageDown press.
const PAGE_SCROLL: u16 = 20;
use state::{auto_refresh, default_thread_lifecycle_index};

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
    Lifecycle,
    Tags,
    Title,
    Body,
    Submit,
}

#[derive(Debug, Clone)]
struct ThreadForm {
    lifecycle_index: usize,
    tags: String,
    title: String,
    body: String,
    field: ThreadFormField,
    /// Inline form-level error from §2.3.5 tag validation, blocks submit.
    tag_error: Option<String>,
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
    Lifecycle,
    Tag,
    Status,
}

#[derive(Debug, Clone, Default)]
pub(super) struct FilterCriteria {
    pub lifecycles: HashSet<String>,
    pub tags: HashSet<String>,
    pub statuses: HashSet<String>,
}

#[derive(Debug, Clone)]
struct FilterBar {
    field: FilterField,
    cursor: usize,
    lifecycles: HashSet<String>,
    tags: HashSet<String>,
    statuses: HashSet<String>,
}

pub(super) const FILTER_LIFECYCLE_LABELS: [&str; 3] = ["proposal", "execution", "record"];
pub(super) const FILTER_STATUS_LABELS: [&str; 8] = [
    "draft",
    "open",
    "working",
    "review",
    "done",
    "rejected",
    "withdrawn",
    "deprecated",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortColumn {
    Id,
    Status,
    Visibility,
    Created,
    Updated,
    Title,
}

pub(super) const SORT_COLUMNS: [SortColumn; 6] = [
    SortColumn::Id,
    SortColumn::Status,
    SortColumn::Visibility,
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
    Dec,
    Task,
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

/// An error flash message shown as an overlay in the TUI.
#[derive(Debug, Clone)]
pub(crate) struct ErrorFlash {
    pub message: String,
    pub hint: Option<String>,
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub threads: Vec<ThreadRow>,
    pub table_state: TableState,
    filter: FilterCriteria,
    filter_bar: Option<FilterBar>,
    pub thread_text: String,
    /// Structured section list for the thread-detail body pane (task
    /// `px3ss55s`). Populated by `open_thread_detail` from
    /// `show::render_full_sections` so the body-pane composer can
    /// style the `OpenObjections` / `OpenActions` / `Conversations`
    /// variants while passing every other block through as plain
    /// text. `thread_text` mirrors the joined CLI text of these
    /// sections and is kept for tests / debugging fallback.
    pub thread_sections: Vec<crate::internal::commands::show::Section>,
    pub thread_scroll: u16,
    /// Largest `thread_scroll` that still shows the body pane's last
    /// line, counted when the pane was last drawn (INV-13).
    pub thread_scroll_max: Option<u16>,
    pub thread_nodes: Vec<NodeWithBody>,
    /// Tree-ordered entries for the nodes panel (may differ from thread_nodes order).
    tree_entries: Vec<TreeEntry>,
    pub node_table_state: TableState,
    pub node_detail_text: String,
    pub node_detail_scroll: u16,
    /// The same limit for `node_detail_scroll`.
    pub node_detail_scroll_max: Option<u16>,
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
    /// Cached `thread_id -> tip SHA` map for the list view's incremental
    /// refresh (task `913c4s9v`, RFC `7ymtc4b2`). Populated by
    /// `state::refresh_thread_list` and compared in `state::auto_refresh`
    /// against the current `for-each-ref` snapshot to detect ref churn.
    pub(crate) list_tip_shas: std::collections::HashMap<String, String>,
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
    /// Whether to use horizontal split (body top / tree bottom) instead of vertical (body left / tree right).
    split_horizontal: bool,
    /// Metadata of the currently viewed thread (for display as root row in nodes pane).
    thread_title: String,
    /// SPEC-2.0 §11: lifecycle facet for the open thread (replaces 1.x kind).
    /// `None` only before `open_thread_detail` runs.
    thread_lifecycle: Option<String>,
    /// SPEC-2.0 §11: derived tag set for the open thread. For unmigrated 1.x
    /// threads (no `facet_set` event) this is the §2.3.3 conventional tag.
    thread_tags: Vec<String>,
    thread_status: String,
    /// Error flash message displayed as an overlay, dismissed on next keypress.
    pub(crate) error_flash: Option<ErrorFlash>,
    /// Brief info flash (e.g. "Copied: RFC-0025"), dismissed on next keypress.
    pub(crate) info_flash: Option<String>,
    /// When true, a "Discard unsaved changes?" confirmation overlay is shown.
    /// The next `y` keypress performs the pending discard; any other key cancels.
    pub(crate) confirm_discard: bool,
    /// When set, the event loop should suspend the terminal, open `$EDITOR`
    /// for the thread body, and submit a revision on save.
    pub(crate) pending_external_edit: Option<String>,
    /// Clipboard and mouse-capture effects of input handling. Tests replace
    /// this with `effects::RecordingEffects`.
    effects: Box<dyn effects::TuiEffects>,
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
            thread_sections: Vec::new(),
            thread_scroll: 0,
            thread_scroll_max: None,
            thread_nodes: Vec::new(),
            tree_entries: Vec::new(),
            node_table_state: TableState::default(),
            node_detail_text: String::new(),
            node_detail_scroll: 0,
            node_detail_scroll_max: None,
            thread_form: ThreadForm {
                lifecycle_index: 0,
                tags: String::new(),
                title: String::new(),
                body: String::new(),
                field: ThreadFormField::Lifecycle,
                tag_error: None,
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
            list_tip_shas: std::collections::HashMap::new(),
            markdown_mode: false,
            mouse_capture_disabled: false,
            collapsed: HashSet::new(),
            visible_tree_indices: Vec::new(),
            tree_fullscreen: false,
            split_horizontal: false,
            thread_title: String::new(),
            thread_lifecycle: None,
            thread_tags: Vec::new(),
            thread_status: String::new(),
            error_flash: None,
            info_flash: None,
            confirm_discard: false,
            pending_external_edit: None,
            effects: Box::new(effects::TerminalEffects),
        }
    }

    pub fn visible_threads(&self) -> Vec<&ThreadRow> {
        let mut rows: Vec<&ThreadRow> = self
            .threads
            .iter()
            .filter(|t| {
                let lifecycle_ok = self.filter.lifecycles.is_empty()
                    || self
                        .filter
                        .lifecycles
                        .contains(render::row_lifecycle(t).as_str());
                let tag_ok = self.filter.tags.is_empty()
                    || self
                        .filter
                        .tags
                        .iter()
                        .all(|t1| render::row_tags(t).iter().any(|t2| t2 == t1));
                let status_ok =
                    self.filter.statuses.is_empty() || self.filter.statuses.contains(&t.status);
                lifecycle_ok && tag_ok && status_ok
            })
            .collect();
        let asc = self.sort_ascending;
        rows.sort_by(|a, b| {
            let ord = match self.sort_column {
                SortColumn::Id => a.id.cmp(&b.id),
                SortColumn::Status => a.status.cmp(&b.status),
                SortColumn::Visibility => render::visibility_marker(a.visibility)
                    .cmp(render::visibility_marker(b.visibility)),
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

    /// Discovered tags across the loaded thread set, sorted ascending.
    /// SPEC-2.0 §2.3.5 ships no preregistered tag list — discovered from data.
    fn discovered_tags(&self) -> Vec<String> {
        let mut set: HashSet<String> = HashSet::new();
        for t in &self.threads {
            for tag in render::row_tags(t) {
                set.insert(tag);
            }
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
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

    fn page_down(&mut self) {
        let n = self.visible_threads().len();
        if n == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|i| (i + PAGE_SCROLL as usize).min(n - 1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn page_up(&mut self) {
        let n = self.visible_threads().len();
        if n == 0 {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|i| i.saturating_sub(PAGE_SCROLL as usize))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    fn move_to_top(&mut self) {
        let n = self.visible_threads().len();
        if n > 0 {
            self.table_state.select(Some(0));
        }
    }

    fn move_to_bottom(&mut self) {
        let n = self.visible_threads().len();
        if n > 0 {
            self.table_state.select(Some(n - 1));
        }
    }

    fn open_filter_bar(&mut self) {
        self.filter_bar = Some(FilterBar {
            field: FilterField::Lifecycle,
            cursor: 0,
            lifecycles: self.filter.lifecycles.clone(),
            tags: self.filter.tags.clone(),
            statuses: self.filter.statuses.clone(),
        });
    }

    fn apply_filter_bar(&mut self) {
        if let Some(bar) = self.filter_bar.take() {
            self.filter.lifecycles = bar.lifecycles;
            self.filter.tags = bar.tags;
            self.filter.statuses = bar.statuses;
            let n = self.visible_threads().len();
            self.table_state.select(if n > 0 { Some(0) } else { None });
        }
    }

    fn cancel_filter_bar(&mut self) {
        self.filter_bar = None;
    }

    fn clear_filter_bar(&mut self) {
        self.filter_bar = None;
        self.filter.lifecycles.clear();
        self.filter.tags.clear();
        self.filter.statuses.clear();
        let n = self.visible_threads().len();
        self.table_state.select(if n > 0 { Some(0) } else { None });
    }

    fn toggle_filter_checkbox(&mut self) {
        let tags = self.discovered_tags();
        let Some(ref mut bar) = self.filter_bar else {
            return;
        };
        match bar.field {
            FilterField::Lifecycle => {
                if bar.cursor < FILTER_LIFECYCLE_LABELS.len() {
                    let label = FILTER_LIFECYCLE_LABELS[bar.cursor].to_string();
                    if !bar.lifecycles.remove(&label) {
                        bar.lifecycles.insert(label);
                    }
                }
            }
            FilterField::Tag => {
                if let Some(label) = tags.get(bar.cursor) {
                    if !bar.tags.remove(label) {
                        bar.tags.insert(label.clone());
                    }
                }
            }
            FilterField::Status => {
                if bar.cursor < FILTER_STATUS_LABELS.len() {
                    let label = FILTER_STATUS_LABELS[bar.cursor].to_string();
                    if !bar.statuses.remove(&label) {
                        bar.statuses.insert(label);
                    }
                }
            }
        }
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

    /// Returns the node ID of the currently selected row, or None if the thread
    /// root row (row 0) is selected or nothing is selected.
    fn selected_node_id(&self) -> Option<String> {
        self.node_table_state
            .selected()
            .and_then(|i| {
                // Row 0 is the thread root; node rows start at index 1
                if i == 0 {
                    return None;
                }
                self.visible_tree_indices.get(i - 1)
            })
            .and_then(|&ti| self.tree_entries.get(ti))
            .map(|entry| self.thread_nodes[entry.node_index].record.id.clone())
    }

    /// Returns true if the current form view has unsaved user input.
    fn has_unsaved_form_input(&self) -> bool {
        match &self.view {
            View::CreateThread => {
                !self.thread_form.title.is_empty() || !self.thread_form.body.is_empty()
            }
            View::CreateNode { .. } => !self.node_form.body.is_empty(),
            View::CreateLink { .. } => !self.link_form.manual_target.is_empty(),
            _ => false,
        }
    }

    fn move_node_down(&mut self) {
        // +1 for the thread root row at index 0
        let n = self.visible_tree_indices.len() + 1;
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
        // +1 for the thread root row at index 0
        let n = self.visible_tree_indices.len() + 1;
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
                .position(|&ti| {
                    self.thread_nodes[self.tree_entries[ti].node_index]
                        .record
                        .id
                        == id
                })
                .map(|pos| pos + 1) // +1 for thread root row at index 0
        });
        self.node_table_state
            .select(match (selected, self.visible_tree_indices.is_empty()) {
                (Some(index), _) => Some(index),
                // Default to row 0 (thread root)
                (None, _) => Some(0),
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
            let node_id = &self.thread_nodes[entry.node_index].record.id;
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
            .and_then(|i| i.checked_sub(1)) // offset for thread root row
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
        self.clamp_thread_scroll();
    }

    fn scroll_thread_up(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_sub(1);
    }

    fn scroll_thread_page_down(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_add(PAGE_SCROLL);
        self.clamp_thread_scroll();
    }

    fn scroll_thread_page_up(&mut self) {
        self.thread_scroll = self.thread_scroll.saturating_sub(PAGE_SCROLL);
    }

    /// Clamp thread_scroll to the limit the last draw of the body pane
    /// counted (INV-13). Before the pane has been drawn there is no limit;
    /// the next draw clamps anyway.
    fn clamp_thread_scroll(&mut self) {
        if let Some(max) = self.thread_scroll_max {
            self.thread_scroll = self.thread_scroll.min(max);
        }
    }

    /// Clamp node_detail_scroll the same way, for the node detail pane.
    fn clamp_node_detail_scroll(&mut self) {
        if let Some(max) = self.node_detail_scroll_max {
            self.node_detail_scroll = self.node_detail_scroll.min(max);
        }
    }

    fn begin_create_thread(&mut self) {
        self.thread_form = ThreadForm {
            lifecycle_index: default_thread_lifecycle_index(
                self.filter.lifecycles.iter().next().map(|s| s.as_str()),
            ),
            tags: String::new(),
            title: String::new(),
            body: String::new(),
            field: ThreadFormField::Lifecycle,
            tag_error: None,
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
/// Preconditions: `db_path` resolves the `.git/forum/` directory; the
/// TUI uses its parent only for the persisted UI state file
/// (`tui-state.toml`) — task `913c4s9v` (RFC `7ymtc4b2`) removed the
/// SQLite index dependency.
/// Postconditions: terminal is restored on exit.
/// Failure modes: ForumError::Io on terminal I/O failure;
/// ForumError::Repo on snapshot/replay errors.
/// Side effects: modifies terminal state; restores on exit.
pub fn run(git: &GitOps, db_path: &Path, initial_thread_id: Option<&str>) -> ForumResult<()> {
    let threads = load_threads(git)?;
    let initial_shas = state::snapshot_list_tip_shas(git)?;

    let mut app = App::new(threads);
    app.list_tip_shas = initial_shas;

    // Restore persisted state (display settings first, then view navigation)
    let persisted = persist::load_state(db_path);
    if let Some(ref saved) = persisted {
        saved.apply_to_app(&mut app);
    }

    if let Some(thread_id) = initial_thread_id {
        // CLI argument takes precedence over persisted view
        state::open_thread_detail(&mut app, git, thread_id, None)?;
    } else if let Some(ref saved) = persisted {
        // Restore persisted view if thread/node still exist
        let thread_exists = saved
            .thread_id
            .as_ref()
            .is_some_and(|tid| app.threads.iter().any(|t| t.id == *tid));
        match saved.view.as_str() {
            "thread" if thread_exists => {
                let tid = saved.thread_id.as_ref().unwrap();
                if state::open_thread_detail(&mut app, git, tid, None).is_ok() {
                    app.thread_scroll = saved.thread_scroll;
                    app.tree_fullscreen = saved.tree_fullscreen;
                    for nid in &saved.collapsed {
                        app.collapsed.insert(nid.clone());
                    }
                    app.recompute_visible_tree();
                    if let Some(sel) = saved.node_table_selected {
                        let n = app.visible_tree_indices.len() + 1;
                        if n > 0 {
                            app.node_table_state.select(Some(sel.min(n - 1)));
                        }
                    }
                }
            }
            "node" if thread_exists => {
                let tid = saved.thread_id.as_ref().unwrap();
                if let Some(ref nid) = saved.node_id {
                    if state::open_node_detail(&mut app, git, tid, nid).is_ok() {
                        app.node_detail_scroll = saved.node_detail_scroll;
                    }
                }
            }
            _ => {} // "list" or invalid — stay on list with restored filters/sort
        }
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app, git, db_path);

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

/// Suspend the terminal, open `$EDITOR` for the thread body, and submit a revision.
fn handle_external_edit<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git: &GitOps,
    _db_path: &Path,
    thread_id: &str,
) -> ForumResult<()>
where
    B::Error: Into<std::io::Error>,
{
    use super::actor;
    use super::clock::SystemClock;
    use super::editor;
    use super::thread;

    // Replay thread to get the current body
    let state = thread::replay_thread(git, thread_id)?;
    let current_body = state.body.as_deref().unwrap_or("");

    // Suspend terminal
    disable_raw_mode().ok();
    execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture).ok();
    terminal.show_cursor().ok();

    // Run editor
    let editor_result = editor::edit_body_with_content(current_body);

    // Restore terminal
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal
        .clear()
        .map_err(|e| -> std::io::Error { e.into() })?;

    // Process result — treat editor abort or unchanged content as no-op
    let new_body = match editor_result {
        Ok(body) if body.trim() == current_body.trim() => {
            app.info_flash = Some("No changes".into());
            return Ok(());
        }
        Ok(body) => body,
        Err(_) => {
            // Editor exited abnormally or body was empty (user quit without saving)
            app.info_flash = Some("Edit cancelled".into());
            return Ok(());
        }
    };

    let actor_id = actor::current_actor(git, git.default_actor());
    let clock = SystemClock;
    state::snapshot_revise_body(git, thread_id, &new_body, &actor_id, &clock)?;

    // task `913c4s9v`: snapshot writes update the ref atomically; the
    // next auto_refresh tick (or open_thread_detail call below) will
    // observe the new tip. No SQLite reindex step needed.
    let selected = app.selected_node_id();
    state::open_thread_detail(app, git, thread_id, selected.as_deref())?;
    app.info_flash = Some("Body revised".into());
    Ok(())
}

/// Convert a ForumError into an ErrorFlash with a context-specific CLI hint.
fn to_error_flash(app: &App, err: &ForumError) -> ErrorFlash {
    let message = err.to_string();
    let hint = match &app.view {
        View::ThreadDetail(id) | View::CreateNode { thread_id: id } => match err {
            ForumError::Policy(_) => Some(format!(
                "Try: git forum verify {id}  or  git forum show {id} --what-next"
            )),
            _ => Some(format!("Try: git forum show {id} --what-next")),
        },
        View::NodeDetail { thread_id, node_id } => match err {
            ForumError::Policy(_) => Some(format!(
                "Try: git forum verify {thread_id}  or  git forum show {thread_id} --what-next"
            )),
            ForumError::Repo(_) => {
                Some(format!("Try: git forum show {thread_id}  (node {node_id})"))
            }
            _ => None,
        },
        _ => None,
    };
    ErrorFlash { message, hint }
}

/// What the event loop must do after [`dispatch_event`] handled one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventOutcome {
    /// Keep running the event loop.
    Continue,
    /// Leave the event loop. TUI state has already been persisted.
    Quit,
    /// Edit this thread's body in `$EDITOR`. The caller must suspend the
    /// terminal, so this stays outside `dispatch_event`.
    ExternalEdit(String),
}

/// Handle one terminal event: dismiss overlays (info flash, discard
/// confirmation, error flash), then dispatch to the key/mouse handlers.
///
/// `run_app` handles input only through this function, so tests that call
/// it drive the same path as the real event loop (doc/spec/TUI-UX-TESTING.md,
/// ADR-012). Handler errors become an error flash instead of propagating.
pub(crate) fn dispatch_event(
    app: &mut App,
    event: Event,
    git: &GitOps,
    db_path: &Path,
) -> EventOutcome {
    match event {
        Event::Key(key) => {
            // Dismiss info flash on any keypress
            app.info_flash = None;
            // If a discard confirmation is showing, handle y/n
            if app.confirm_discard {
                app.confirm_discard = false;
                if key.code == KeyCode::Char('y') || key.code == KeyCode::Char('Y') {
                    // Perform the discard: reset form and navigate away
                    match input::confirm_discard_action(app, git) {
                        Ok(()) => {}
                        Err(e) => app.error_flash = Some(to_error_flash(app, &e)),
                    }
                }
                // On any other key, just dismiss the confirmation
                return EventOutcome::Continue;
            }
            // If an error flash is showing, dismiss it on any keypress
            if app.error_flash.is_some() {
                app.error_flash = None;
                return EventOutcome::Continue;
            }
            match handle_key(app, key, git, db_path) {
                Ok(true) => {
                    persist::save_state(app, db_path);
                    return EventOutcome::Quit;
                }
                Ok(false) => {}
                Err(e) => app.error_flash = Some(to_error_flash(app, &e)),
            }
            // Pending external editor request (terminal suspend required)
            match app.pending_external_edit.take() {
                Some(thread_id) => EventOutcome::ExternalEdit(thread_id),
                None => EventOutcome::Continue,
            }
        }
        Event::Mouse(mouse) => {
            // Dismiss info flash on any click
            app.info_flash = None;
            // If an error flash is showing, dismiss it on any click
            if app.error_flash.is_some() {
                app.error_flash = None;
                return EventOutcome::Continue;
            }
            match handle_mouse(app, mouse, git, db_path) {
                Ok(true) => {
                    persist::save_state(app, db_path);
                    return EventOutcome::Quit;
                }
                Ok(false) => {}
                Err(e) => app.error_flash = Some(to_error_flash(app, &e)),
            }
            EventOutcome::Continue
        }
        _ => EventOutcome::Continue,
    }
}

pub(crate) fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git: &GitOps,
    db_path: &Path,
) -> ForumResult<()>
where
    B::Error: Into<std::io::Error>,
{
    loop {
        terminal
            .draw(|f| render(f, app))
            .map_err(|e| -> std::io::Error { e.into() })?;

        // Auto-refresh: check if the viewed thread has changed
        if app.last_refresh.elapsed().as_millis() >= AUTO_REFRESH_INTERVAL_MS {
            auto_refresh(app, git)?;
            app.last_refresh = Instant::now();
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            match dispatch_event(app, event::read()?, git, db_path) {
                EventOutcome::Continue => {}
                EventOutcome::Quit => return Ok(()),
                EventOutcome::ExternalEdit(thread_id) => {
                    match handle_external_edit(terminal, app, git, db_path, &thread_id) {
                        Ok(()) => {}
                        Err(e) => app.error_flash = Some(to_error_flash(app, &e)),
                    }
                }
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

    use crate::internal::id_alloc;
    use crate::internal::node::NodeKind;
    // task `913c4s9v` (RFC `7ymtc4b2`): tests construct the thread
    // listing via the snapshot walker instead of `index::list_threads`
    // / `reindex::run_reindex`.
    use crate::internal::snapshot::list as snapshot_list;

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
        let lifecycle = match kind {
            "rfc" => "proposal",
            "dec" => "record",
            _ => "execution",
        };
        ThreadRow {
            id: id.into(),
            kind: kind.into(),
            lifecycle: lifecycle.into(),
            lifecycle_explicit: false,
            tags: Vec::new(),
            status: status.into(),
            title: title.into(),
            body: None,
            branch: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            created_by: "human/alice".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            visibility: crate::internal::thread::Visibility::Private,
            from_published: false,
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

    pub(super) fn setup_repo() -> (
        TempDir,
        GitOps,
        crate::internal::config::RepoPaths,
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

        (dir, git, repo_paths, db_path)
    }

    /// Snapshot fixture: write a fresh SPEC-3.0 thread snapshot with
    /// the given category/title. Replaces `create::create_thread`
    /// callers in TUI tests after task `1v400j3l` made the bridge
    /// in non-migrate paths bail on legacy chains.
    fn make_snapshot_thread(git: &GitOps, category: &str, title: &str, seed: u8) -> String {
        use crate::internal::snapshot::{write_snapshot, ThreadDocument};
        use crate::internal::thread::ThreadSnapshot;
        let id = format!(
            "tui{seed:02x}{seed:02x}{seed:02x}",
            seed = seed.max(1) // tokens cannot be all-zero
        );
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let initial_status = match category {
            "rfc" => "draft",
            _ => "open",
        };
        let doc = ThreadDocument::new(ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: id.clone(),
            title: title.to_string(),
            category: category.to_string(),
            status: initial_status.to_string(),
            tags: vec![],
            created_at: now,
            created_by: "human/test-user".into(),
            updated_at: now,
            updated_by: "human/test-user".into(),
            branch: None,
            supersedes: vec![],
            visibility: Default::default(),
        });
        write_snapshot(git, &id, &doc, "create test thread").unwrap();
        id
    }

    /// Snapshot fixture: append a node to an existing snapshot thread
    /// and return the new node id. Replaces `write_ops::say_node` in
    /// TUI tests.
    fn append_snapshot_node(git: &GitOps, thread_id: &str, kind: NodeKind, body: &str) -> String {
        use crate::internal::node::{NodeRecord, NodeStatus};
        use crate::internal::snapshot::{read_snapshot, write_snapshot, NodeWithBody};
        let mut doc = read_snapshot(git, thread_id).unwrap();
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap();
        let id = id_alloc::alloc_bare_thread_id("human/test-user", body, &now.to_rfc3339());
        doc.nodes.push(NodeWithBody {
            record: NodeRecord {
                id: id.clone(),
                kind,
                status: NodeStatus::Open,
                created_at: now,
                created_by: "human/test-user".into(),
                updated_at: None,
                updated_by: None,
                reply_to: None,
                legacy_label: None,
            },
            body: body.into(),
        });
        doc.snapshot.updated_at = now;
        doc.snapshot.updated_by = "human/test-user".into();
        write_snapshot(git, thread_id, &doc, "append test node").unwrap();
        id
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
        let out = render_to_string(&mut app, 110, 20);
        assert!(out.contains("Test RFC"), "expected 'Test RFC' in:\n{out}");
    }

    /// wy22s31p: at widths where the list's columns do not fit, redrawing
    /// the same state must give the same screen every time. Without
    /// ratatui's `layout-cache` feature every draw solves the layout again
    /// and may pick another of several equally good column widths.
    #[test]
    fn list_redraws_the_same_screen_when_the_columns_do_not_fit() {
        let mut app = App::new(vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
        ]);
        for (w, h) in [(60, 20), (40, 12), (79, 24)] {
            let first = render_to_string(&mut app, w, h);
            for n in 1..100 {
                let again = render_to_string(&mut app, w, h);
                assert_eq!(again, first, "{w}x{h}: draw {n} differs from the first");
            }
        }
    }

    /// TUI-LIST-COLUMNS AT-2: the spec's examples, plus a 10-character ID
    /// (AT-4) and the narrowest widths (rule 5).
    #[test]
    fn list_column_widths_follow_the_spec_examples() {
        use render::list_column_widths as widths;
        let (status, created, updated) = (1, 3, 4);
        let cases: [(u16, usize, u16, [Option<u16>; 6]); 10] = [
            (
                78,
                updated,
                8,
                [Some(13), Some(11), Some(5), Some(12), Some(12), Some(20)],
            ),
            (
                118,
                status,
                8,
                [Some(13), Some(11), Some(5), Some(12), Some(12), Some(60)],
            ),
            (
                58,
                updated,
                8,
                [Some(8), Some(10), Some(5), None, Some(10), Some(21)],
            ),
            (
                58,
                created,
                8,
                [Some(8), Some(10), None, Some(10), Some(10), Some(16)],
            ),
            (
                38,
                updated,
                8,
                [Some(8), None, None, None, Some(10), Some(18)],
            ),
            (
                38,
                status,
                8,
                [Some(8), Some(10), None, None, None, Some(18)],
            ),
            (20, updated, 8, [Some(8), None, None, None, None, Some(11)]),
            (
                58,
                updated,
                10,
                [Some(10), Some(10), Some(5), None, Some(10), Some(19)],
            ),
            (9, updated, 8, [Some(8), None, None, None, None, None]),
            (5, updated, 8, [Some(5), None, None, None, None, None]),
        ];
        for (inner, sort, id_len, want) in cases {
            assert_eq!(
                widths(inner, sort, id_len),
                want,
                "W={inner} sort={sort} L={id_len}"
            );
        }
    }

    /// TUI-LIST-COLUMNS rules 3, 5 and 6 over every width: the columns never
    /// take more than W, ID and TITLE are hidden last, and the sort column
    /// is hidden only after the other three.
    #[test]
    fn list_column_widths_fit_every_width() {
        use render::list_column_widths as widths;
        for inner in 0..=200u16 {
            for sort in 0..6 {
                for id_len in [0, 8, 10, 13, 20] {
                    let w = widths(inner, sort, id_len);
                    let shown: Vec<u16> = w.iter().flatten().copied().collect();
                    let total = shown.iter().sum::<u16>() + shown.len() as u16 - 1;
                    let at = format!("W={inner} sort={sort} L={id_len}: {w:?}");
                    assert!(total <= inner, "{at}");
                    assert!(w[0].is_some(), "{at}");
                    if w[5].is_none() {
                        assert!(w[1..5].iter().all(Option::is_none), "{at}");
                    }
                    if (1..5).contains(&sort) && w[sort].is_none() {
                        assert!(w[1..5].iter().all(Option::is_none), "{at}");
                    }
                    assert_eq!(widths(inner, sort, id_len), w, "{at}");
                }
            }
        }
    }

    /// TUI-LIST-COLUMNS AT-1, AT-2, AT-4 and AT-5: the header row as drawn,
    /// and a click on each shown header sorts by that column.
    #[test]
    fn list_headers_are_drawn_and_clicked_where_the_widths_put_them() {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        let labels = ["ID", "STATUS", "VIS", "CREATED", "UPDATED", "TITLE"];
        for (w, h, long_id) in [
            (40, 12, false),
            (60, 20, false),
            (60, 20, true),
            (80, 24, false),
            (200, 60, false),
        ] {
            let first = if long_id { "ISSUE-0001" } else { "abcd1234" };
            let mut app = App::new(vec![
                make_row(first, "issue", "open", "Bug"),
                make_row("efgh5678", "rfc", "draft", "Proposal"),
            ]);
            let screen = render_to_string(&mut app, w, h);
            let rows: Vec<&str> = screen.lines().collect();
            let sort = SORT_COLUMNS
                .iter()
                .position(|c| *c == app.sort_column)
                .unwrap();
            let widths = render::list_column_widths(w - 2, sort, if long_id { 10 } else { 8 });
            let header: Vec<char> = rows[2].chars().collect();
            let mut x = 1;
            for (i, cw) in widths.iter().enumerate() {
                let at = format!("{w}x{h} long_id={long_id} column {i}");
                let rect = app.ui_rects.column_headers[i];
                let Some(cw) = *cw else {
                    assert_eq!(rect, None, "{at}");
                    continue;
                };
                let rect = rect.unwrap_or_else(|| panic!("{at}: no click area"));
                assert_eq!((rect.x, rect.y, rect.width), (x, 2, cw), "{at}");
                let cell: String = header[usize::from(x)..usize::from(x + cw)].iter().collect();
                assert!(
                    cell.starts_with(labels[i]),
                    "{at}: {cell:?} in {:?}",
                    rows[2]
                );
                x += cw + 1;
            }
            if long_id {
                assert!(screen.contains("│ISSUE-0001 "), "{screen}");
            }
            for (i, rect) in app.ui_rects.column_headers.into_iter().enumerate() {
                let Some(rect) = rect else { continue };
                app.sort_column = SORT_COLUMNS[(i + 1) % 6];
                let click = MouseEventKind::Down(MouseButton::Left);
                handle_mouse(
                    &mut app,
                    mouse_event(click, rect.x, rect.y),
                    &git,
                    dir.path(),
                )
                .unwrap();
                assert_eq!(app.sort_column, SORT_COLUMNS[i], "{w}x{h} column {i}");
            }
        }
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
        let (_dir, git, _paths, db_path) = setup_repo();
        // task `913c4s9v`: snapshot fixture (v2 event chains return
        // LegacyEventChain on the snapshot read path so they don't
        // appear in the listing).
        make_snapshot_thread(&git, "rfc", "Test RFC", 1);
        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
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
            &db_path,
        )
        .unwrap();

        assert_eq!(app.view, View::List);
        assert_eq!(app.table_state.selected(), Some(0));
    }

    #[test]
    fn mouse_double_click_opens_thread_detail() {
        let (_dir, git, _paths, db_path) = setup_repo();
        let created_id = make_snapshot_thread(&git, "rfc", "Test RFC", 2);
        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
        let _ = render_to_string(&mut app, 80, 20);
        let area = app.ui_rects.list_table.unwrap();

        let click = mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 2,
            area.y + 2,
        );
        // First click selects
        handle_mouse(&mut app, click, &git, &db_path).unwrap();
        assert_eq!(app.view, View::List);
        // Second click (same position, quick) opens
        handle_mouse(&mut app, click, &git, &db_path).unwrap();
        assert_eq!(app.view, View::ThreadDetail(created_id));
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

        // Click on the "CREATED" column header (4th column: ID, STATUS, VIS, CREATED).
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
        app.filter.lifecycles.insert("execution".into());
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
        app.view = View::ThreadDetail("RFC-0001".into());
        // Enough drawn lines to exceed the viewport, so the scroll limit
        // is above 0. The pane draws the sections, not `thread_text`.
        app.thread_sections = vec![crate::internal::commands::show::Section::Text(
            (0..50).map(|n| format!("line {n}")).collect(),
        )];
        let _ = render_to_string(&mut app, 120, 24);
        let area = app.ui_rects.thread_body.unwrap();

        handle_mouse(
            &mut app,
            mouse_event(MouseEventKind::ScrollDown, area.x + 1, area.y + 1),
            &git,
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
        app.thread_nodes = vec![NodeWithBody {
            record: crate::internal::node::NodeRecord {
                id: "abcdef1234567890".into(),
                kind: crate::internal::node::NodeKind::Comment,
                created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                created_by: "human/alice".into(),
                legacy_label: Some("question".into()),
                ..Default::default()
            },
            body: "What is this?".into(),
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
        assert!(out.contains("lifecycle: execution"));
        assert!(out.contains("tags: (none)"));
        assert!(out.contains("title:"));
        assert!(out.contains("body: (empty)"));
        assert!(out.contains("submit: [Create thread]"));
        assert!(out.contains("thread lifecycles"));
        assert!(out.contains("> execution"));
    }

    #[test]
    fn create_node_view_shows_form_fields() {
        let mut app = App::new(vec![]);
        app.begin_create_node("RFC-0001");
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("create node"));
        // SPEC-2.0 §2.5: default node type is `comment` (was `claim` in 1.x).
        assert!(out.contains("type: comment"));
        assert!(out.contains("body: (empty)"));
        assert!(out.contains("submit: [Create node]"));
        assert!(out.contains("node types"));
        assert!(out.contains("> comment"));
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
    fn enter_on_thread_lifecycle_field_does_not_submit() {
        let mut app = App::new(vec![]);
        app.begin_create_thread();

        handle_edit_transition_via_create_thread(&mut app);

        assert_eq!(app.view, View::CreateThread);
        assert_eq!(app.thread_form.field, ThreadFormField::Lifecycle);
    }

    fn handle_edit_transition_via_create_thread(app: &mut App) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        handle_create_thread_key(
            app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            dir.path(),
        )
        .unwrap();
    }

    fn handle_edit_transition_via_create_node(app: &mut App) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        handle_create_node_key(
            app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
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
        let (_dir, git, _paths, _db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        // Lifecycle index 0 = proposal, with conventional tag → RFC preset.
        app.thread_form.lifecycle_index = 0;
        app.thread_form.tags = "cross-cutting".into();
        app.thread_form.title = "Created in TUI".into();
        app.thread_form.body = "Body from TUI".into();

        submit_create_thread(&mut app, &git).unwrap();

        match &app.view {
            View::ThreadDetail(id) => {
                assert!(
                    crate::internal::id_alloc::is_bare_token(id),
                    "expected bare token, got: {id}"
                )
            }
            other => panic!("expected ThreadDetail, got: {other:?}"),
        }
        assert!(app.thread_text.contains("Created in TUI"));
        let rows = snapshot_list::list_threads(&git).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Created in TUI");
    }

    #[test]
    fn mouse_click_on_thread_submit_creates_thread() {
        let (_dir, git, _paths, db_path) = setup_repo();
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
            &db_path,
        )
        .unwrap();

        match &app.view {
            View::ThreadDetail(id) => {
                assert!(
                    crate::internal::id_alloc::is_bare_token(id),
                    "expected bare token, got: {id}"
                )
            }
            other => panic!("expected ThreadDetail, got: {other:?}"),
        }
        assert!(app.thread_text.contains("Created with mouse"));
    }

    #[test]
    fn submit_create_node_adds_node_and_keeps_thread_detail() {
        let (_dir, git, _paths, db_path) = setup_repo();
        let thread_id = make_snapshot_thread(&git, "rfc", "RFC from setup", 0xa0);

        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
        open_thread_detail(&mut app, &git, &thread_id, None).unwrap();
        app.begin_create_node(&thread_id);
        app.node_form.node_type_index = 1;
        app.node_form.body = "Node from TUI\nwith more detail".into();
        app.node_form.field = NodeFormField::Submit;
        handle_create_node_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &db_path,
            &thread_id,
        )
        .unwrap();

        assert_eq!(app.view, View::ThreadDetail(thread_id.clone()));
        assert_eq!(app.thread_nodes.len(), 1);
        assert_eq!(app.thread_nodes[0].body, "Node from TUI\nwith more detail");
        // Row 0 is thread root; the newly created node is at row 1
        assert_eq!(app.node_table_state.selected(), Some(1));
    }

    #[test]
    fn submit_create_link_from_thread_adds_link_and_returns_to_thread_detail() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let source_id = make_snapshot_thread(&git, "rfc", "Source RFC", 0xa1);
        make_snapshot_thread(&git, "issue", "Implementation issue", 0xa2);

        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
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
        assert!(
            app.thread_text.contains("implements"),
            "expected 'implements' in thread_text"
        );
    }

    #[test]
    fn submit_create_link_from_node_returns_to_node_detail() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let source_id = make_snapshot_thread(&git, "rfc", "Source RFC", 0xa3);
        make_snapshot_thread(&git, "issue", "Implementation issue", 0xa4);
        let node_id = append_snapshot_node(
            &git,
            &source_id,
            NodeKind::Comment,
            "Investigate parser shape",
        );

        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
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
        assert!(app.node_detail_text.contains("### thread links (1)"));
        assert!(
            app.node_detail_text.contains("implements"),
            "expected 'implements' in node_detail_text"
        );
    }

    #[test]
    fn apply_node_status_action_updates_node_detail() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let thread_id = make_snapshot_thread(&git, "rfc", "RFC from setup", 0xa5);
        let node_id =
            append_snapshot_node(&git, &thread_id, NodeKind::Objection, "Needs more evidence");

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
        assert!(app.node_detail_text.contains("**status:**    resolved"));

        apply_node_status_action(
            &mut app,
            &git,
            &thread_id,
            &node_id,
            NodeStatusAction::Reopen,
        )
        .unwrap();
        assert!(app.node_detail_text.contains("**status:**    open"));

        apply_node_status_action(
            &mut app,
            &git,
            &thread_id,
            &node_id,
            NodeStatusAction::Retract,
        )
        .unwrap();
        assert!(app.node_detail_text.contains("**status:**    retracted"));
    }

    #[test]
    fn filter_bar_apply_sets_lifecycle_and_status() {
        let mut app = App::new(vec![]);
        assert!(app.filter.lifecycles.is_empty());
        assert!(app.filter.statuses.is_empty());

        app.open_filter_bar();
        assert!(app.filter_bar.is_some());

        if let Some(ref mut bar) = app.filter_bar {
            bar.lifecycles.insert("execution".into());
            bar.statuses.insert("draft".into());
        }
        app.apply_filter_bar();

        assert!(app.filter.lifecycles.contains("execution"));
        assert!(app.filter.statuses.contains("draft"));
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_cancel_preserves_existing_filter() {
        let mut app = App::new(vec![]);
        app.filter.lifecycles.insert("proposal".into());
        app.open_filter_bar();
        if let Some(ref mut bar) = app.filter_bar {
            bar.lifecycles.clear();
            bar.lifecycles.insert("execution".into());
        }
        app.cancel_filter_bar();
        assert!(app.filter.lifecycles.contains("proposal"));
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_clear_resets_both_dimensions() {
        let mut app = App::new(vec![]);
        app.filter.lifecycles.insert("execution".into());
        app.filter.statuses.insert("open".into());
        app.open_filter_bar();
        app.clear_filter_bar();
        assert!(app.filter.lifecycles.is_empty());
        assert!(app.filter.statuses.is_empty());
        assert!(app.filter_bar.is_none());
    }

    #[test]
    fn filter_bar_open_reflects_current_filter() {
        let mut app = App::new(vec![]);
        app.filter.lifecycles.insert("proposal".into());
        app.filter.statuses.insert("review".into());
        app.open_filter_bar();
        let bar = app.filter_bar.as_ref().unwrap();
        assert!(bar.lifecycles.contains("proposal"));
        assert!(bar.statuses.contains("review"));
    }

    #[test]
    fn filter_bar_toggle_checkbox() {
        let mut app = App::new(vec![]);
        app.open_filter_bar();
        // FILTER_LIFECYCLE_LABELS = ["proposal", "execution", "record"];
        // cursor 1 = execution.
        if let Some(ref mut bar) = app.filter_bar {
            bar.field = FilterField::Lifecycle;
            bar.cursor = 1;
        }
        app.toggle_filter_checkbox();
        assert!(app
            .filter_bar
            .as_ref()
            .unwrap()
            .lifecycles
            .contains("execution"));
        // Toggle it off
        app.toggle_filter_checkbox();
        assert!(!app
            .filter_bar
            .as_ref()
            .unwrap()
            .lifecycles
            .contains("execution"));
    }

    #[test]
    fn mouse_wheel_leaves_the_list_alone_while_the_filter_bar_is_open() {
        let mut app = App::new(vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("ISSUE-0002", "issue", "open", "Crash"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
        ]);
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        app.table_state.select(Some(1));
        app.open_filter_bar();
        let _ = render_to_string(&mut app, 80, 24);
        let popup = app.ui_rects.filter_popup.unwrap();
        let list = app.ui_rects.list_table.unwrap();
        let bar = format!("{:?}", app.filter_bar);
        // Over the popup, and over a list row outside it.
        let outside = (list.x + 1, list.y + 2);
        assert!(!input::rect_contains(popup, outside.0, outside.1));
        let inside = (popup.x + popup.width / 2, popup.y + popup.height / 2);
        for (column, row) in [inside, outside] {
            for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
                handle_mouse(&mut app, mouse_event(kind, column, row), &git, dir.path()).unwrap();
                assert_eq!(
                    app.table_state.selected(),
                    Some(1),
                    "{kind:?} at {column},{row}"
                );
                assert_eq!(
                    format!("{:?}", app.filter_bar),
                    bar,
                    "{kind:?} at {column},{row}"
                );
            }
        }
    }

    #[test]
    fn filter_multi_select_lifecycles() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
        ];
        let mut app = App::new(rows);
        app.filter.lifecycles.insert("execution".into());
        app.filter.lifecycles.insert("proposal".into());
        let visible = app.visible_threads();
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn visible_threads_filters_by_status() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "draft", "Proposal"),
            make_row("ISSUE-0002", "issue", "done", "Old bug"),
        ];
        let mut app = App::new(rows);
        app.filter.statuses.insert("open".into());
        let visible = app.visible_threads();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "ISSUE-0001");
    }

    #[test]
    fn visible_threads_filters_by_lifecycle_and_status() {
        let rows = vec![
            make_row("ISSUE-0001", "issue", "open", "Bug"),
            make_row("RFC-0001", "rfc", "open", "Proposal"),
            make_row("ISSUE-0002", "issue", "done", "Old bug"),
        ];
        let mut app = App::new(rows);
        app.filter.lifecycles.insert("execution".into());
        app.filter.statuses.insert("open".into());
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
            NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "a".into(),
                    kind: crate::internal::node::NodeKind::Comment,
                    created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                    created_by: "human/alice".into(),
                    legacy_label: Some("question".into()),
                    ..Default::default()
                },
                body: "A".into(),
            },
            NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "b".into(),
                    kind: crate::internal::node::NodeKind::Comment,
                    created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                    created_by: "human/alice".into(),
                    legacy_label: Some("question".into()),
                    ..Default::default()
                },
                body: "B".into(),
            },
        ];
        app.tree_entries = build_tree_entries(&app.thread_nodes);
        app.recompute_visible_tree();
        // Row 0 is thread root; 2 nodes at rows 1 and 2
        app.node_table_state.select(Some(0));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(1));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(2));
        app.move_node_down();
        assert_eq!(app.node_table_state.selected(), Some(2));
    }

    #[test]
    fn build_tree_entries_orders_siblings_chronologically() {
        // `read_snapshot` loads node files id-sorted for determinism, but
        // bare random IDs (e.g. `fg61bcmp`) carry no temporal meaning, so
        // the timeline must reorder by `created_at`. Regression for the
        // "comment node seems not sorted" report.
        let nodes = vec![
            // Created second, but id sorts first.
            NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "aaaaaaaa".into(),
                    kind: crate::internal::node::NodeKind::Comment,
                    created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                    created_by: "human/alice".into(),
                    ..Default::default()
                },
                body: "second".into(),
            },
            // Created first, but id sorts last.
            NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "zzzzzzzz".into(),
                    kind: crate::internal::node::NodeKind::Comment,
                    created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                    created_by: "human/alice".into(),
                    ..Default::default()
                },
                body: "first".into(),
            },
            // Reply to the first node, created last — should land under
            // its parent at the bottom.
            NodeWithBody {
                record: crate::internal::node::NodeRecord {
                    id: "mmmmmmmm".into(),
                    kind: crate::internal::node::NodeKind::Comment,
                    created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 2, 0).unwrap(),
                    created_by: "human/alice".into(),
                    reply_to: Some("zzzzzzzz".into()),
                    ..Default::default()
                },
                body: "reply".into(),
            },
        ];
        let entries = build_tree_entries(&nodes);
        let order: Vec<&str> = entries
            .iter()
            .map(|e| nodes[e.node_index].record.id.as_str())
            .collect();
        // First root by created_at, then its reply, then the later root.
        assert_eq!(order, vec!["zzzzzzzz", "mmmmmmmm", "aaaaaaaa"]);
    }

    #[test]
    fn error_flash_renders_overlay() {
        let mut app = App::new(vec![make_row("RFC-0001", "rfc", "draft", "Test RFC")]);
        app.error_flash = Some(ErrorFlash {
            message: "policy error: 2 open objection(s)".into(),
            hint: Some("Try: git forum verify RFC-0001".into()),
        });
        let out = render_to_string(&mut app, 80, 24);
        assert!(
            out.contains("policy error"),
            "expected error message in:\n{out}"
        );
        assert!(
            out.contains("git forum verify"),
            "expected CLI hint in:\n{out}"
        );
        assert!(
            out.contains("Press any key"),
            "expected dismiss instruction in:\n{out}"
        );
    }

    #[test]
    fn to_error_flash_includes_thread_hint() {
        let mut app = App::new(vec![]);
        app.view = View::ThreadDetail("ISSUE-0042".into());
        let err = ForumError::Policy("2 open objection(s)".into());
        let flash = to_error_flash(&app, &err);
        assert!(flash.message.contains("2 open objection"));
        let hint = flash.hint.unwrap();
        assert!(hint.contains("ISSUE-0042"));
        assert!(hint.contains("verify"));
    }

    #[test]
    fn to_error_flash_no_hint_on_list_view() {
        let mut app = App::new(vec![]);
        app.view = View::List;
        let err = ForumError::Repo("not found".into());
        let flash = to_error_flash(&app, &err);
        assert!(flash.hint.is_none());
    }

    // ------------------------------------------------------------------
    // JOB-d4cdyi5b AC#10 — Track H acceptance tests
    // ------------------------------------------------------------------

    /// AC#10: list-view row renders the tag chip prefix and the bare ID
    /// (ADR-014: `@` is accepted on input, never printed). The LIFECYCLE
    /// column was removed in v3.1.x — lifecycle is still surfaced on the
    /// thread-detail header and via the filter popup.
    #[test]
    fn list_row_shows_tag_chip_with_bare_id() {
        let rows = vec![
            make_row("a7f3b2x1", "issue", "open", "fix the bug"),
            // Legacy ID → unchanged
            make_row("RFC-0001", "rfc", "draft", "propose a thing"),
        ];
        let mut app = App::new(rows);
        let out = render_to_string(&mut app, 110, 20);
        assert!(out.contains("a7f3b2x1"), "missing thread ID:\n{out}");
        assert!(!out.contains("@a7f3b2x1"), "ID rendered with @:\n{out}");
        assert!(
            out.contains("RFC-0001"),
            "legacy ID should render unchanged:\n{out}"
        );
        assert!(
            !out.contains("LIFECYCLE"),
            "LIFECYCLE column was removed but header is still rendered:\n{out}"
        );
        // Visibility lives in its own VIS column; the bracket
        // prefix on the title carries tags only.
        assert!(
            out.contains("[bug] fix the bug"),
            "missing [bug] tag chip on title:\n{out}"
        );
        assert!(
            out.contains("[cross-cutting]"),
            "missing [cross-cutting] tag chip on title:\n{out}"
        );
        assert!(out.contains("VIS"), "missing VIS column header:\n{out}");
        assert!(out.contains("priv"), "missing priv visibility cell:\n{out}");
    }

    // task `913c4s9v`: the
    // `thread_detail_header_shows_lifecycle_tags_linked_panel` test
    // (legacy v2 §2.3.3 kind→lifecycle/tags fallback) was removed.
    // It exercised `internal::create::create_thread` (deleted by
    // task `913c4s9v`) and the `state.lifecycle_explicit=false` branch
    // in `tui::state::open_thread_detail` (v2 peer-type removal tracked
    // by task `1v400j3l`). Equivalent v3.0 coverage —
    // snapshot fixtures with explicit category/tags — already lives
    // in `create_thread_form_*` tests below.

    /// AC#10: create-thread form integration — `(execution, [bug])` writes
    /// a SPEC-3.0 snapshot with `category=task` and the `bug` tag.
    /// task `1hg98odf`: the legacy preset / facet_set
    /// fork is gone; tests that pinned the v2.x event count are
    /// rewritten to assert the snapshot shape.
    #[test]
    fn create_thread_form_execution_bug_writes_snapshot() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        // Lifecycle 1 = execution; tag = bug → category=task + tag=bug.
        app.thread_form.lifecycle_index = 1;
        app.thread_form.tags = "bug".into();
        app.thread_form.title = "fix the bug".into();
        submit_create_thread(&mut app, &git).unwrap();

        let rows = snapshot_list::list_threads(&git).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "fix the bug");

        // SPEC-3.0 snapshot tip: `thread.toml` decodes, category=task, bug tag.
        let doc = crate::internal::snapshot::read_snapshot(&git, &rows[0].id).unwrap();
        assert_eq!(doc.snapshot.category, "task");
        assert!(doc.snapshot.tags.contains(&"bug".to_string()));
        assert_eq!(doc.snapshot.title, "fix the bug");
    }

    /// AC#10: multi-tag form input lands all tags in `thread.toml.tags`.
    /// task `1hg98odf`: the previous facet_set event
    /// is gone — tags are written directly into the snapshot.
    #[test]
    fn create_thread_form_non_preset_writes_tags_in_snapshot() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.lifecycle_index = 1; // execution
        app.thread_form.tags = "bug, urgent".into();
        app.thread_form.title = "two-tag bug".into();
        submit_create_thread(&mut app, &git).unwrap();

        let rows = snapshot_list::list_threads(&git).unwrap();
        assert_eq!(rows.len(), 1);

        let doc = crate::internal::snapshot::read_snapshot(&git, &rows[0].id).unwrap();
        assert!(doc.snapshot.tags.contains(&"bug".to_string()));
        assert!(doc.snapshot.tags.contains(&"urgent".to_string()));
    }

    /// AC#10: invalid tag input blocks submission and records a form-level
    /// error (§2.3.5 grammar). No thread is created.
    #[test]
    fn create_thread_form_invalid_tag_blocks_submit() {
        let (_dir, git, _paths, _db_path) = setup_repo();
        let mut app = App::new(vec![]);
        app.begin_create_thread();
        app.thread_form.lifecycle_index = 1;
        app.thread_form.tags = "BadTag".into(); // uppercase rejected by §2.3.5
        app.thread_form.title = "should not be created".into();
        submit_create_thread(&mut app, &git).unwrap();

        assert!(app.thread_form.tag_error.is_some());
        let rows = snapshot_list::list_threads(&git).unwrap();
        assert!(rows.is_empty(), "no thread should be written on tag error");
    }

    /// AC#10: create-node form default selection writes a `Comment` node
    /// (no `legacy_subtype`).
    #[test]
    fn create_node_form_default_writes_comment() {
        let (_dir, git, _paths, db_path) = setup_repo();
        let thread_id = make_snapshot_thread(&git, "rfc", "Comment default", 0xa6);

        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
        open_thread_detail(&mut app, &git, &thread_id, None).unwrap();
        app.begin_create_node(&thread_id);
        // Default selection is index 0 → Comment.
        app.node_form.body = "default-typed reply".into();
        app.node_form.field = NodeFormField::Submit;
        handle_create_node_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &git,
            &db_path,
            &thread_id,
        )
        .unwrap();

        assert_eq!(app.thread_nodes.len(), 1);
        assert_eq!(
            app.thread_nodes[0].record.kind,
            crate::internal::node::NodeKind::Comment
        );
        assert!(
            app.thread_nodes[0].record.legacy_label.is_none(),
            "default Comment must have no legacy_subtype"
        );
    }

    // ============================================================
    //  task `px3ss55s` — structured-section TUI body pane
    // ============================================================
    //
    // Buffer-style assertions complement the existing
    // `render_to_string` helper: that helper flattens the ratatui
    // buffer to symbols and discards style, so it cannot catch a
    // missing red foreground on an objection row, missing cyan on
    // an action row, or missing bold on a section header. The
    // helpers below preserve cell `Style` so we can verify the
    // node-kind palette is actually applied.

    fn render_to_buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::Terminal;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::render::render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_row_str(buf: &ratatui::buffer::Buffer, y: usize) -> String {
        let w = buf.area.width as usize;
        buf.content()[(y * w)..((y + 1) * w)]
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    }

    /// Find a row whose flattened symbols contain `needle`. Returns
    /// (y, byte_offset_in_row) for the first match.
    fn find_row(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(usize, usize)> {
        let h = buf.area.height as usize;
        for y in 0..h {
            let row = buffer_row_str(buf, y);
            if let Some(start) = row.find(needle) {
                return Some((y, start));
            }
        }
        None
    }

    /// Style at cell `(y, col)` where `col` indexes ASCII columns
    /// (caller asserts the content is ASCII; ratatui cells are
    /// per-symbol so this matches `buffer_row_str.find()` byte
    /// offsets for ASCII rows).
    fn cell_style_at(buf: &ratatui::buffer::Buffer, y: usize, col: usize) -> ratatui::style::Style {
        let w = buf.area.width as usize;
        buf.content()[y * w + col].style()
    }

    fn rich_state_for_body_pane() -> crate::internal::thread::ThreadState {
        use crate::internal::node::{NodeKind, NodeRecord};
        use crate::internal::snapshot::store::NodeWithBody;
        use crate::internal::thread::{ThreadLink, ThreadState};
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut state = ThreadState {
            id: "RFC-test01".into(),
            category: "rfc".into(),
            title: "Cache eviction proposal".into(),
            body: Some("This is the thread body text.".into()),
            status: "review".into(),
            created_at: t,
            created_by: "human/alice".into(),
            updated_at: t,
            ..ThreadState::default()
        };
        state.nodes = vec![
            NodeWithBody {
                record: NodeRecord {
                    id: "obj00001".into(),
                    kind: NodeKind::Objection,
                    created_at: t,
                    created_by: "ai/reviewer".into(),
                    ..Default::default()
                },
                body: "Benchmarks are missing.".into(),
            },
            NodeWithBody {
                record: NodeRecord {
                    id: "act00002".into(),
                    kind: NodeKind::Action,
                    created_at: t,
                    created_by: "human/bob".into(),
                    ..Default::default()
                },
                body: "Backfill index column.".into(),
            },
            NodeWithBody {
                record: NodeRecord {
                    id: "cmt00003".into(),
                    kind: NodeKind::Comment,
                    created_at: t,
                    created_by: "human/carol".into(),
                    ..Default::default()
                },
                body: "Standalone comment leaf.".into(),
            },
        ];
        state.evidence_items = vec![crate::internal::evidence::EvidenceRecord {
            id: "evid0001".into(),
            kind: crate::internal::evidence::EvidenceKind::Commit,
            ref_target: "abc1234".into(),
            created_at: t,
            created_by: "human/alice".into(),
        }];
        state.links = vec![ThreadLink {
            target_thread_id: "RFC-other1".into(),
            rel: "implements".into(),
        }];
        state
    }

    fn open_thread_app_with_state(state: &crate::internal::thread::ThreadState) -> App {
        use crate::internal::commands::show;
        let mut app = App::new(vec![]);
        app.view = View::ThreadDetail(state.id.clone());
        let sections = show::render_full_sections(state, &show::ShowOptions::default());
        let mut text_lines: Vec<String> = Vec::new();
        for s in &sections {
            text_lines.extend(s.to_text_lines());
        }
        app.thread_text = text_lines.join("\n");
        app.thread_sections = sections;
        app.thread_title = state.title.clone();
        app.thread_lifecycle = Some("proposal".into());
        app.thread_tags = state.tags.clone();
        app.thread_status = state.status.clone();
        app.thread_nodes = state.nodes.clone();
        app
    }

    #[test]
    fn body_pane_section_headers_are_bold_in_markdown_mode() {
        // task `px3ss55s`: structured sections emit markdown text that
        // is parsed by `markdown_to_text` when the user has the
        // markdown toggle on (`m` keybinding). `markdown_to_text`
        // styles `## Header` as bold + cyan; this test confirms the
        // section text reaches the markdown parser.
        let state = rich_state_for_body_pane();
        let mut app = open_thread_app_with_state(&state);
        app.markdown_mode = true;
        let buf = render_to_buffer(&mut app, 140, 60);

        for header in [
            "Open objections (1)",
            "Open actions (1)",
            "Conversations (3)",
        ] {
            let (y, x) = find_row(&buf, header).unwrap_or_else(|| {
                panic!(
                    "header `{header}` not found in body pane buffer\n{}",
                    (0..buf.area.height as usize)
                        .map(|y| buffer_row_str(&buf, y))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            });
            let style = cell_style_at(&buf, y, x);
            assert!(
                style.add_modifier.contains(ratatui::style::Modifier::BOLD),
                "section header `{header}` at ({y},{x}) is not bold; style = {:?}",
                style
            );
        }
    }

    #[test]
    fn body_pane_preserves_non_section_content() {
        // task `px3ss55s` AC: the body pane keeps thread body,
        // evidence, and links visible after the structured-section
        // change. This guards against accidentally dropping the
        // existing `render_show` content during the section rewrite.
        let state = rich_state_for_body_pane();
        let mut app = open_thread_app_with_state(&state);
        let out = render_to_string(&mut app, 140, 60);

        for needle in [
            "This is the thread body text.", // body
            "evid0001",                      // evidence id
            "commit",                        // evidence kind
            "abc1234",                       // evidence ref
            "RFC-other1",                    // link target
            "implements",                    // link rel
        ] {
            assert!(
                out.contains(needle),
                "expected `{needle}` to be visible in body pane; got:\n{out}"
            );
        }
    }

    #[test]
    fn body_pane_zero_nodes_elides_conversation_section() {
        // task `px3ss55s` AC: a thread with `build_conversations(...)`
        // empty (i.e. no nodes) renders no `Conversations` section.
        use crate::internal::thread::ThreadState;
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let state = ThreadState {
            id: "RFC-empty01".into(),
            category: "rfc".into(),
            title: "Empty thread".into(),
            body: Some("body".into()),
            status: "open".into(),
            created_at: t,
            created_by: "human/alice".into(),
            updated_at: t,
            ..ThreadState::default()
        };
        let mut app = open_thread_app_with_state(&state);
        let out = render_to_string(&mut app, 140, 60);
        assert!(
            !out.contains("Conversations ("),
            "zero-node thread should not render a Conversations section; got:\n{out}"
        );
        assert!(
            !out.contains("Open objections ("),
            "zero-node thread should not render an Open objections section; got:\n{out}"
        );
    }

    #[test]
    fn body_pane_standalone_leaf_counts_as_one_conversation() {
        // task `px3ss55s` AC: a standalone leaf node is a one-element
        // chain and counts as one conversation, matching CLI today.
        use crate::internal::node::{NodeKind, NodeRecord};
        use crate::internal::snapshot::store::NodeWithBody;
        use crate::internal::thread::ThreadState;
        let t = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut state = ThreadState {
            id: "RFC-leaf01".into(),
            category: "rfc".into(),
            title: "Leaf only".into(),
            body: Some("body".into()),
            status: "open".into(),
            created_at: t,
            created_by: "human/alice".into(),
            updated_at: t,
            ..ThreadState::default()
        };
        state.nodes = vec![NodeWithBody {
            record: NodeRecord {
                id: "leaf0001".into(),
                kind: NodeKind::Comment,
                created_at: t,
                created_by: "human/alice".into(),
                ..Default::default()
            },
            body: "Just a single leaf.".into(),
        }];
        let mut app = open_thread_app_with_state(&state);
        let out = render_to_string(&mut app, 140, 60);
        assert!(
            out.contains("Conversations (1)"),
            "single-leaf thread should render Conversations (1); got:\n{out}"
        );
    }

    // ============================================================
    //  dispatch_event — run_app's per-event path
    //  (doc/spec/TUI-UX-TESTING.md INV-4, INV-8; ADR-012)
    // ============================================================

    fn key_event(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_c() -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))
    }

    fn some_error_flash() -> Option<ErrorFlash> {
        Some(ErrorFlash {
            message: "boom".into(),
            hint: None,
        })
    }

    /// Repo with two threads and an App on the list view, row 0 selected.
    fn list_app_with_two_threads() -> (TempDir, GitOps, std::path::PathBuf, App) {
        let (dir, git, _paths, db_path) = setup_repo();
        make_snapshot_thread(&git, "task", "First", 1);
        make_snapshot_thread(&git, "task", "Second", 2);
        let app = App::new(snapshot_list::list_threads(&git).unwrap());
        assert_eq!(app.table_state.selected(), Some(0));
        (dir, git, db_path, app)
    }

    /// App on the create-thread form with a typed title.
    fn create_thread_app_with_title() -> (TempDir, GitOps, std::path::PathBuf, App) {
        let (dir, git, _paths, db_path) = setup_repo();
        let mut app = App::new(Vec::new());
        app.begin_create_thread();
        app.thread_form.title = "Half-written title".into();
        (dir, git, db_path, app)
    }

    #[test]
    fn dispatch_error_flash_consumes_key_without_acting() {
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        app.error_flash = some_error_flash();

        let out = dispatch_event(&mut app, key_event(KeyCode::Char('j')), &git, &db_path);
        assert_eq!(out, EventOutcome::Continue);
        assert!(app.error_flash.is_none());
        assert_eq!(app.view, View::List);
        assert_eq!(
            app.table_state.selected(),
            Some(0),
            "flash must eat the key"
        );

        // Control: the same key moves the selection once the flash is gone.
        dispatch_event(&mut app, key_event(KeyCode::Char('j')), &git, &db_path);
        assert_eq!(app.table_state.selected(), Some(1));
    }

    #[test]
    fn dispatch_error_flash_consumes_mouse_click_without_acting() {
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        let _ = render_to_string(&mut app, 80, 20);
        let area = app.ui_rects.list_table.unwrap();
        let click_row_1 = Event::Mouse(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 2,
            area.y + 3,
        ));
        app.error_flash = some_error_flash();

        let out = dispatch_event(&mut app, click_row_1.clone(), &git, &db_path);
        assert_eq!(out, EventOutcome::Continue);
        assert!(app.error_flash.is_none());
        assert_eq!(
            app.table_state.selected(),
            Some(0),
            "flash must eat the click"
        );

        // Control: the same click selects row 1 once the flash is gone.
        dispatch_event(&mut app, click_row_1, &git, &db_path);
        assert_eq!(app.table_state.selected(), Some(1));
    }

    #[test]
    fn dispatch_ctrl_c_quits_within_two_presses_from_error_flash() {
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        app.error_flash = some_error_flash();

        assert_eq!(
            dispatch_event(&mut app, ctrl_c(), &git, &db_path),
            EventOutcome::Continue,
            "first Ctrl-C only dismisses the error flash"
        );
        assert!(app.error_flash.is_none());
        assert_eq!(
            dispatch_event(&mut app, ctrl_c(), &git, &db_path),
            EventOutcome::Quit
        );
    }

    #[test]
    fn dispatch_ctrl_c_quits_form_with_unsaved_input_without_confirmation() {
        let (_dir, git, db_path, mut app) = create_thread_app_with_title();
        assert!(app.has_unsaved_form_input());

        assert_eq!(
            dispatch_event(&mut app, ctrl_c(), &git, &db_path),
            EventOutcome::Quit
        );
        assert!(!app.confirm_discard);
    }

    #[test]
    fn dispatch_confirm_discard_other_keys_keep_form() {
        for (label, event) in [
            ("n", key_event(KeyCode::Char('n'))),
            ("Esc", key_event(KeyCode::Esc)),
            ("Ctrl-C", ctrl_c()),
        ] {
            let (_dir, git, db_path, mut app) = create_thread_app_with_title();
            dispatch_event(&mut app, key_event(KeyCode::Esc), &git, &db_path);
            assert!(app.confirm_discard, "Esc with unsaved input must ask first");

            let out = dispatch_event(&mut app, event, &git, &db_path);
            assert_eq!(out, EventOutcome::Continue, "{label}");
            assert!(!app.confirm_discard, "{label}: confirmation must close");
            assert_eq!(app.view, View::CreateThread, "{label}");
            assert_eq!(app.thread_form.title, "Half-written title", "{label}");
        }
    }

    #[test]
    fn dispatch_confirm_discard_y_discards_form() {
        for ch in ['y', 'Y'] {
            let (_dir, git, db_path, mut app) = create_thread_app_with_title();
            dispatch_event(&mut app, key_event(KeyCode::Esc), &git, &db_path);
            assert!(app.confirm_discard);

            let out = dispatch_event(&mut app, key_event(KeyCode::Char(ch)), &git, &db_path);
            assert_eq!(out, EventOutcome::Continue, "{ch}");
            assert!(!app.confirm_discard, "{ch}");
            assert_eq!(app.view, View::List, "{ch}");
            assert!(app.thread_form.title.is_empty(), "{ch}");
        }
    }

    #[test]
    fn dispatch_info_flash_clears_and_key_still_acts() {
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        app.info_flash = Some("Copied: @x".into());

        dispatch_event(&mut app, key_event(KeyCode::Char('j')), &git, &db_path);
        assert!(app.info_flash.is_none());
        assert_eq!(app.table_state.selected(), Some(1));
    }

    #[test]
    fn dispatch_quit_persists_tui_state() {
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        let state_file = db_path.parent().unwrap().join("tui-state.toml");
        assert!(
            !state_file.exists(),
            "fixture must start without a state file"
        );

        let out = dispatch_event(&mut app, key_event(KeyCode::Char('q')), &git, &db_path);
        assert_eq!(out, EventOutcome::Quit);
        assert!(
            state_file.exists(),
            "quit must save {}",
            state_file.display()
        );
    }

    #[test]
    fn dispatch_e_in_thread_detail_requests_external_edit() {
        let (_dir, git, _paths, db_path) = setup_repo();
        let id = make_snapshot_thread(&git, "task", "Editable", 3);
        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
        open_thread_detail(&mut app, &git, &id, None).unwrap();

        let out = dispatch_event(&mut app, key_event(KeyCode::Char('e')), &git, &db_path);
        assert_eq!(out, EventOutcome::ExternalEdit(id));
        assert!(app.pending_external_edit.is_none());
    }

    /// AT-6(a): run_app must not handle input except through
    /// dispatch_event, or the tests above stop describing the real loop.
    #[test]
    fn run_app_handles_input_only_through_dispatch_event() {
        use syn::visit::Visit;

        struct Calls(Vec<String>);
        impl<'ast> Visit<'ast> for Calls {
            fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
                if let syn::Expr::Path(p) = &*call.func {
                    if let Some(seg) = p.path.segments.last() {
                        self.0.push(seg.ident.to_string());
                    }
                }
                syn::visit::visit_expr_call(self, call);
            }
        }

        let file = syn::parse_file(include_str!("mod.rs")).unwrap();
        let run_app = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(f) if f.sig.ident == "run_app" => Some(f),
                _ => None,
            })
            .expect("run_app not found in mod.rs");
        let mut calls = Calls(Vec::new());
        calls.visit_item_fn(run_app);

        // Control: the visitor must see the call we know is there.
        assert!(
            calls.0.iter().any(|c| c == "dispatch_event"),
            "visitor found no dispatch_event call; calls seen: {:?}",
            calls.0
        );
        for forbidden in ["handle_key", "handle_mouse"] {
            assert!(
                !calls.0.iter().any(|c| c == forbidden),
                "run_app calls {forbidden} directly; route it through dispatch_event"
            );
        }
    }

    // ============================================================
    //  App::effects — clipboard / mouse capture
    //  (doc/spec/TUI-UX-TESTING.md AT-6(b), AT-7; ADR-012)
    // ============================================================

    fn record_effects(app: &mut App) -> effects::RecordingEffects {
        let rec = effects::RecordingEffects::default();
        app.effects = Box::new(rec.clone());
        rec
    }

    #[test]
    fn yank_in_list_goes_through_effects() {
        use effects::EffectCall;
        let (_dir, git, db_path, mut app) = list_app_with_two_threads();
        let rec = record_effects(&mut app);
        // ADR-014: yanked text uses the display form of the ID.
        let shown = crate::internal::id::display_thread_id(&app.selected_thread_id().unwrap());

        dispatch_event(&mut app, key_event(KeyCode::Char('y')), &git, &db_path);
        assert_eq!(
            *rec.calls.borrow(),
            vec![EffectCall::Clipboard(shown.clone())]
        );
        assert_eq!(app.info_flash, Some(format!("Copied: {shown}")));
    }

    #[test]
    fn select_mode_toggles_mouse_capture_through_effects() {
        use effects::EffectCall;
        let (_dir, git, _paths, db_path) = setup_repo();
        let id = make_snapshot_thread(&git, "task", "Selectable", 4);
        let mut app = App::new(snapshot_list::list_threads(&git).unwrap());
        open_thread_detail(&mut app, &git, &id, None).unwrap();
        let rec = record_effects(&mut app);

        dispatch_event(&mut app, key_event(KeyCode::Char('S')), &git, &db_path);
        assert_eq!(*rec.calls.borrow(), vec![EffectCall::MouseCapture(false)]);
        assert!(app.mouse_capture_disabled);

        // Any key leaves select mode and turns capture back on.
        dispatch_event(&mut app, key_event(KeyCode::Char('j')), &git, &db_path);
        assert_eq!(
            *rec.calls.borrow(),
            vec![
                EffectCall::MouseCapture(false),
                EffectCall::MouseCapture(true)
            ]
        );
        assert!(!app.mouse_capture_disabled);
    }

    /// AT-6(b): input handling reaches the clipboard and the terminal only
    /// through `App::effects`, so tests using RecordingEffects touch neither.
    #[test]
    fn input_handling_has_no_direct_external_effects() {
        let forbidden = [
            "std::io::stdout",
            "execute!",
            "Command::new",
            "copy_to_clipboard",
        ];
        // Control: the scan must see these tokens where they do live.
        let effects_src = include_str!("effects.rs");
        for token in ["std::io::stdout", "execute!", "Command::new"] {
            assert!(
                effects_src.contains(token),
                "control failed: `{token}` not found in effects.rs"
            );
        }
        for (name, src) in [
            ("input.rs", include_str!("input.rs")),
            ("state.rs", include_str!("state.rs")),
        ] {
            for token in forbidden {
                assert!(
                    !src.contains(token),
                    "{name} contains `{token}`; route the effect through App::effects"
                );
            }
        }
    }
}
