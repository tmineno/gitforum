//! TUI state persistence — save/load view state per repository.
//!
//! State file: `.git/forum/tui-state.toml` (per-worktree).
//! All operations are best-effort: failures are silently ignored.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{App, FilterCriteria, SortColumn, View};

/// Serializable snapshot of TUI state.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TuiState {
    /// Last active view: "list", "thread", or "node".
    #[serde(default)]
    pub view: String,
    /// Thread ID if the last view was ThreadDetail or NodeDetail.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Node ID if the last view was NodeDetail.
    #[serde(default)]
    pub node_id: Option<String>,
    /// Selected row index in the list view table.
    #[serde(default)]
    pub list_selected: Option<usize>,
    /// Thread body scroll position.
    #[serde(default)]
    pub thread_scroll: u16,
    /// Node detail scroll position.
    #[serde(default)]
    pub node_detail_scroll: u16,
    /// Selected row in the node tree table.
    #[serde(default)]
    pub node_table_selected: Option<usize>,
    /// Pane split percentage (20..80).
    #[serde(default = "default_split")]
    pub detail_split: u16,
    /// Horizontal split mode (body top / tree bottom).
    #[serde(default)]
    pub split_horizontal: bool,
    /// Tree pane full-width mode.
    #[serde(default)]
    pub tree_fullscreen: bool,
    /// Markdown rendering toggle.
    #[serde(default)]
    pub markdown_mode: bool,
    /// Collapsed node IDs in the tree view.
    #[serde(default)]
    pub collapsed: Vec<String>,
    /// Kind filter selections.
    #[serde(default)]
    pub filter_kinds: Vec<String>,
    /// Status filter selections.
    #[serde(default)]
    pub filter_statuses: Vec<String>,
    /// Sort column name.
    #[serde(default = "default_sort_column")]
    pub sort_column: String,
    /// Sort ascending.
    #[serde(default)]
    pub sort_ascending: bool,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            view: String::new(),
            thread_id: None,
            node_id: None,
            list_selected: None,
            thread_scroll: 0,
            node_detail_scroll: 0,
            node_table_selected: None,
            detail_split: 60,
            split_horizontal: false,
            tree_fullscreen: false,
            markdown_mode: false,
            collapsed: Vec::new(),
            filter_kinds: Vec::new(),
            filter_statuses: Vec::new(),
            sort_column: "updated".to_string(),
            sort_ascending: false,
        }
    }
}

fn default_split() -> u16 {
    60
}

fn default_sort_column() -> String {
    "updated".to_string()
}

impl TuiState {
    /// Capture current app state into a serializable snapshot.
    pub fn from_app(app: &App) -> Self {
        let (view, thread_id, node_id) = match &app.view {
            View::List => ("list".to_string(), None, None),
            View::ThreadDetail(tid) => ("thread".to_string(), Some(tid.clone()), None),
            View::NodeDetail { thread_id, node_id } => (
                "node".to_string(),
                Some(thread_id.clone()),
                Some(node_id.clone()),
            ),
            // Don't persist form views
            _ => ("list".to_string(), None, None),
        };

        TuiState {
            view,
            thread_id,
            node_id,
            list_selected: app.table_state.selected(),
            thread_scroll: app.thread_scroll,
            node_detail_scroll: app.node_detail_scroll,
            node_table_selected: app.node_table_state.selected(),
            detail_split: app.detail_split,
            split_horizontal: app.split_horizontal,
            tree_fullscreen: app.tree_fullscreen,
            markdown_mode: app.markdown_mode,
            collapsed: app.collapsed.iter().cloned().collect(),
            filter_kinds: app.filter.kinds.iter().cloned().collect(),
            filter_statuses: app.filter.statuses.iter().cloned().collect(),
            sort_column: sort_column_to_str(app.sort_column).to_string(),
            sort_ascending: app.sort_ascending,
        }
    }

    /// Apply persisted state to the app (display-only fields; view navigation is handled separately).
    pub fn apply_to_app(&self, app: &mut App) {
        app.detail_split = self.detail_split.clamp(20, 80);
        app.split_horizontal = self.split_horizontal;
        app.markdown_mode = self.markdown_mode;
        app.sort_column = sort_column_from_str(&self.sort_column);
        app.sort_ascending = self.sort_ascending;
        app.filter = FilterCriteria {
            kinds: self.filter_kinds.iter().cloned().collect(),
            statuses: self.filter_statuses.iter().cloned().collect(),
        };
        if let Some(sel) = self.list_selected {
            let n = app.visible_threads().len();
            if n > 0 {
                app.table_state.select(Some(sel.min(n - 1)));
            }
        }
    }
}

/// Resolve the state file path from db_path.
/// db_path is `.git/forum/index.db`, so parent is `.git/forum/`.
fn state_file_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("tui-state.toml")
}

/// Save TUI state to disk. Silently ignores errors.
pub(super) fn save_state(app: &App, db_path: &Path) {
    let state = TuiState::from_app(app);
    let path = state_file_path(db_path);
    if let Ok(content) = toml::to_string_pretty(&state) {
        let _ = std::fs::write(&path, content);
    }
}

/// Load TUI state from disk. Returns None on any error or missing file.
pub(super) fn load_state(db_path: &Path) -> Option<TuiState> {
    let path = state_file_path(db_path);
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

fn sort_column_to_str(col: SortColumn) -> &'static str {
    match col {
        SortColumn::Id => "id",
        SortColumn::Kind => "kind",
        SortColumn::Status => "status",
        SortColumn::Created => "created",
        SortColumn::Updated => "updated",
        SortColumn::Title => "title",
    }
}

fn sort_column_from_str(s: &str) -> SortColumn {
    match s {
        "id" => SortColumn::Id,
        "kind" => SortColumn::Kind,
        "status" => SortColumn::Status,
        "created" => SortColumn::Created,
        "title" => SortColumn::Title,
        _ => SortColumn::Updated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_default_state() {
        let state = TuiState::default();
        let toml_str = toml::to_string_pretty(&state).unwrap();
        let loaded: TuiState = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.view, state.view);
        assert_eq!(loaded.detail_split, 60);
        assert_eq!(loaded.sort_column, "updated");
    }

    #[test]
    fn roundtrip_with_data() {
        let state = TuiState {
            view: "thread".to_string(),
            thread_id: Some("RFC-abc123".to_string()),
            node_id: None,
            list_selected: Some(3),
            thread_scroll: 42,
            node_detail_scroll: 0,
            node_table_selected: Some(1),
            detail_split: 45,
            split_horizontal: true,
            tree_fullscreen: false,
            markdown_mode: true,
            collapsed: vec!["node1".to_string(), "node2".to_string()],
            filter_kinds: vec!["rfc".to_string()],
            filter_statuses: vec!["open".to_string(), "draft".to_string()],
            sort_column: "kind".to_string(),
            sort_ascending: true,
        };
        let toml_str = toml::to_string_pretty(&state).unwrap();
        let loaded: TuiState = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.view, "thread");
        assert_eq!(loaded.thread_id.as_deref(), Some("RFC-abc123"));
        assert_eq!(loaded.detail_split, 45);
        assert!(loaded.split_horizontal);
        assert!(loaded.markdown_mode);
        assert_eq!(loaded.collapsed.len(), 2);
        assert_eq!(loaded.filter_kinds, vec!["rfc"]);
        assert_eq!(loaded.sort_column, "kind");
        assert!(loaded.sort_ascending);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let result = load_state(Path::new("/nonexistent/path/index.db"));
        assert!(result.is_none());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");

        let state = TuiState {
            view: "node".to_string(),
            thread_id: Some("ASK-xyz".to_string()),
            node_id: Some("abc123".to_string()),
            detail_split: 70,
            markdown_mode: true,
            ..TuiState::default()
        };

        // Write manually since we don't have an App
        let path = state_file_path(&db_path);
        let content = toml::to_string_pretty(&state).unwrap();
        std::fs::write(&path, content).unwrap();

        let loaded = load_state(&db_path).unwrap();
        assert_eq!(loaded.view, "node");
        assert_eq!(loaded.thread_id.as_deref(), Some("ASK-xyz"));
        assert_eq!(loaded.node_id.as_deref(), Some("abc123"));
        assert_eq!(loaded.detail_split, 70);
        assert!(loaded.markdown_mode);
    }

    #[test]
    fn sort_column_roundtrip() {
        for col in &[
            SortColumn::Id,
            SortColumn::Kind,
            SortColumn::Status,
            SortColumn::Created,
            SortColumn::Updated,
            SortColumn::Title,
        ] {
            let s = sort_column_to_str(*col);
            assert_eq!(sort_column_from_str(s), *col);
        }
    }

    #[test]
    fn unknown_sort_column_defaults_to_updated() {
        assert_eq!(sort_column_from_str("garbage"), SortColumn::Updated);
    }

    #[test]
    fn partial_toml_loads_with_defaults() {
        let toml_str = r#"
view = "list"
markdown_mode = true
"#;
        let state: TuiState = toml::from_str(toml_str).unwrap();
        assert_eq!(state.view, "list");
        assert!(state.markdown_mode);
        assert_eq!(state.detail_split, 60); // default
        assert_eq!(state.sort_column, "updated"); // default
        assert!(!state.sort_ascending); // default
    }

    use crate::internal::index::ThreadRow;

    fn make_thread_row(id: &str, kind: &str) -> ThreadRow {
        ThreadRow {
            id: id.to_string(),
            kind: kind.to_string(),
            status: "open".to_string(),
            title: format!("Thread {id}"),
            body: None,
            branch: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            created_by: "test".to_string(),
            open_objections: 0,
            open_actions: 0,
            has_summary: false,
            tip_sha: "abc123".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn from_app_captures_list_view() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let app = App::new(threads);
        let state = TuiState::from_app(&app);
        assert_eq!(state.view, "list");
        assert!(state.thread_id.is_none());
        assert!(state.node_id.is_none());
    }

    #[test]
    fn from_app_captures_thread_detail_view() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);
        app.view = View::ThreadDetail("RFC-001".to_string());
        app.thread_scroll = 15;
        app.detail_split = 40;
        app.markdown_mode = true;
        let state = TuiState::from_app(&app);
        assert_eq!(state.view, "thread");
        assert_eq!(state.thread_id.as_deref(), Some("RFC-001"));
        assert_eq!(state.thread_scroll, 15);
        assert_eq!(state.detail_split, 40);
        assert!(state.markdown_mode);
    }

    #[test]
    fn from_app_captures_node_detail_view() {
        let threads = vec![make_thread_row("ASK-001", "ask")];
        let mut app = App::new(threads);
        app.view = View::NodeDetail {
            thread_id: "ASK-001".to_string(),
            node_id: "node42".to_string(),
        };
        app.node_detail_scroll = 7;
        let state = TuiState::from_app(&app);
        assert_eq!(state.view, "node");
        assert_eq!(state.thread_id.as_deref(), Some("ASK-001"));
        assert_eq!(state.node_id.as_deref(), Some("node42"));
        assert_eq!(state.node_detail_scroll, 7);
    }

    #[test]
    fn apply_to_app_restores_display_settings() {
        let threads = vec![
            make_thread_row("RFC-001", "rfc"),
            make_thread_row("ASK-001", "ask"),
        ];
        let mut app = App::new(threads);

        let saved = TuiState {
            detail_split: 35,
            split_horizontal: true,
            markdown_mode: true,
            sort_column: "kind".to_string(),
            sort_ascending: true,
            filter_kinds: vec!["rfc".to_string()],
            filter_statuses: vec!["open".to_string()],
            list_selected: Some(0),
            ..TuiState::default()
        };
        saved.apply_to_app(&mut app);

        assert_eq!(app.detail_split, 35);
        assert!(app.split_horizontal);
        assert!(app.markdown_mode);
        assert_eq!(app.sort_column, SortColumn::Kind);
        assert!(app.sort_ascending);
        assert!(app.filter.kinds.contains("rfc"));
        assert!(app.filter.statuses.contains("open"));
    }

    #[test]
    fn apply_to_app_clamps_split() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);

        let saved = TuiState {
            detail_split: 99, // out of range
            ..TuiState::default()
        };
        saved.apply_to_app(&mut app);
        assert_eq!(app.detail_split, 80); // clamped
    }

    #[test]
    fn apply_to_app_list_selected_clamped_to_visible() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);

        let saved = TuiState {
            list_selected: Some(999),
            ..TuiState::default()
        };
        saved.apply_to_app(&mut app);
        assert_eq!(app.table_state.selected(), Some(0)); // clamped to last visible
    }

    #[test]
    fn from_app_form_view_falls_back_to_list() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);
        app.view = View::CreateThread;
        let state = TuiState::from_app(&app);
        assert_eq!(state.view, "list");
    }

    #[test]
    fn save_and_load_via_app() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);
        app.view = View::ThreadDetail("RFC-001".to_string());
        app.markdown_mode = true;
        app.detail_split = 42;

        save_state(&app, &db_path);

        let loaded = load_state(&db_path).unwrap();
        assert_eq!(loaded.view, "thread");
        assert_eq!(loaded.thread_id.as_deref(), Some("RFC-001"));
        assert!(loaded.markdown_mode);
        assert_eq!(loaded.detail_split, 42);
    }

    #[test]
    fn corrupted_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");
        let path = state_file_path(&db_path);
        std::fs::write(&path, "{{{{not valid toml}}}}").unwrap();
        assert!(load_state(&db_path).is_none());
    }
}
