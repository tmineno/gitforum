//! TUI state persistence — save/load view state per repository.
//!
//! State file: `.git/forum/tui-state.toml` (per-worktree).
//! All operations are best-effort: failures are silently ignored.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{App, FilterCriteria, SortColumn, View};

/// Persisted-schema version. Bumped to 2 in JOB-d4cdyi5b for the
/// kind→lifecycle/tags split. v1 reads continue to succeed via the
/// `filter_kinds` field; writes always emit v2.
pub(super) const SCHEMA_VERSION: u32 = 2;

/// Serializable snapshot of TUI state.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TuiState {
    /// Schema version (default 1 for back-compat reads).
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
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
    /// Legacy v1 kind filter selections (still read for migration; never written by v2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter_kinds: Vec<String>,
    /// Lifecycle filter selections (v2).
    #[serde(default)]
    pub filter_lifecycles: Vec<String>,
    /// Tag filter selections (v2).
    #[serde(default)]
    pub filter_tags: Vec<String>,
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

fn default_schema_version() -> u32 {
    1
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
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
            filter_lifecycles: Vec::new(),
            filter_tags: Vec::new(),
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
            schema_version: SCHEMA_VERSION,
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
            // v2 never serialises kinds; the field stays present for back-compat
            // schemas only.
            filter_kinds: Vec::new(),
            filter_lifecycles: app.filter.lifecycles.iter().cloned().collect(),
            filter_tags: app.filter.tags.iter().cloned().collect(),
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
        // v1 → v2 migration: translate `filter_kinds` to lifecycles + tags
        // per SPEC-2.0 §2.3.3. v2 fields take precedence when both present.
        let mut lifecycles: std::collections::HashSet<String> =
            self.filter_lifecycles.iter().cloned().collect();
        let mut tags: std::collections::HashSet<String> =
            self.filter_tags.iter().cloned().collect();
        for kind in &self.filter_kinds {
            let (lc, conv_tag) = legacy_kind_to_lifecycle_and_tag(kind);
            lifecycles.insert(lc.to_string());
            if let Some(tag) = conv_tag {
                tags.insert(tag.to_string());
            }
        }
        app.filter = FilterCriteria {
            lifecycles,
            tags,
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

/// SPEC-2.0 §2.3.3: each 1.x kind maps to a lifecycle and an optional
/// conventional tag. Used by `apply_to_app` for v1→v2 filter migration.
fn legacy_kind_to_lifecycle_and_tag(kind: &str) -> (&'static str, Option<&'static str>) {
    match kind {
        "rfc" => ("proposal", Some("cross-cutting")),
        "issue" => ("execution", Some("bug")),
        "task" => ("execution", Some("task")),
        "dec" => ("record", None),
        _ => ("execution", None),
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
        SortColumn::Status => "status",
        SortColumn::Created => "created",
        SortColumn::Updated => "updated",
        SortColumn::Title => "title",
    }
}

fn sort_column_from_str(s: &str) -> SortColumn {
    match s {
        "id" => SortColumn::Id,
        "status" => SortColumn::Status,
        "created" => SortColumn::Created,
        "title" => SortColumn::Title,
        // v1 used "kind"; v2 used "lifecycle". Both columns were
        // removed in v3.1; old persisted state falls back to the
        // default Updated sort.
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
            schema_version: SCHEMA_VERSION,
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
            filter_kinds: Vec::new(),
            filter_lifecycles: vec!["proposal".to_string()],
            filter_tags: vec!["cross-cutting".to_string()],
            filter_statuses: vec!["open".to_string(), "draft".to_string()],
            sort_column: "lifecycle".to_string(),
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
        assert_eq!(loaded.filter_lifecycles, vec!["proposal"]);
        assert_eq!(loaded.filter_tags, vec!["cross-cutting"]);
        assert_eq!(loaded.sort_column, "lifecycle");
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
            SortColumn::Status,
            SortColumn::Created,
            SortColumn::Updated,
            SortColumn::Title,
        ] {
            let s = sort_column_to_str(*col);
            assert_eq!(sort_column_from_str(s), *col);
        }
    }

    /// Persisted "lifecycle"/"kind" from v1/v2 schema falls back to the
    /// default Updated sort after the column was removed in v3.1.
    #[test]
    fn sort_column_legacy_lifecycle_falls_back_to_updated() {
        assert_eq!(sort_column_from_str("lifecycle"), SortColumn::Updated);
        assert_eq!(sort_column_from_str("kind"), SortColumn::Updated);
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

    use crate::internal::snapshot::list::ThreadRow;

    fn make_thread_row(id: &str, kind: &str) -> ThreadRow {
        // task `913c4s9v`: ThreadRow now lives in `snapshot::list`,
        // mirroring the v2 field surface minus the index-only counts
        // (`open_objections`/`open_actions`/`has_summary`/`tip_sha`)
        // that TUI production code never read.
        let lifecycle = match kind {
            "rfc" => "proposal",
            "dec" => "record",
            _ => "execution",
        };
        ThreadRow {
            id: id.to_string(),
            kind: kind.to_string(),
            lifecycle: lifecycle.to_string(),
            lifecycle_explicit: false,
            tags: Vec::new(),
            status: "open".to_string(),
            title: format!("Thread {id}"),
            body: None,
            branch: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            created_by: "test".to_string(),
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
            sort_column: "status".to_string(),
            sort_ascending: true,
            filter_lifecycles: vec!["proposal".to_string()],
            filter_statuses: vec!["open".to_string()],
            list_selected: Some(0),
            ..TuiState::default()
        };
        saved.apply_to_app(&mut app);

        assert_eq!(app.detail_split, 35);
        assert!(app.split_horizontal);
        assert!(app.markdown_mode);
        assert_eq!(app.sort_column, SortColumn::Status);
        assert!(app.sort_ascending);
        assert!(app.filter.lifecycles.contains("proposal"));
        assert!(app.filter.statuses.contains("open"));
    }

    /// SPEC-2.0 §2.3.3: a v1 state file with `filter_kinds = ["rfc"]` must
    /// load as `filter_lifecycles = ["proposal"]` + `filter_tags =
    /// ["cross-cutting"]`. Writes always emit v2.
    #[test]
    fn v1_filter_kinds_translates_to_v2_lifecycles_and_tags() {
        let threads = vec![make_thread_row("RFC-001", "rfc")];
        let mut app = App::new(threads);
        let v1 = TuiState {
            schema_version: 1,
            filter_kinds: vec!["rfc".to_string()],
            ..TuiState::default()
        };
        v1.apply_to_app(&mut app);
        assert!(app.filter.lifecycles.contains("proposal"));
        assert!(app.filter.tags.contains("cross-cutting"));

        // Re-serialise: v2 must drop filter_kinds and emit filter_lifecycles.
        let v2 = TuiState::from_app(&app);
        assert_eq!(v2.schema_version, SCHEMA_VERSION);
        assert!(v2.filter_kinds.is_empty());
        assert!(v2.filter_lifecycles.contains(&"proposal".to_string()));
        assert!(v2.filter_tags.contains(&"cross-cutting".to_string()));
    }

    /// v1 → v2 round-trip on disk: write v1 TOML, read it back, observe v2.
    #[test]
    fn v1_on_disk_to_v2_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");
        let path = state_file_path(&db_path);
        let v1_toml = r#"
view = "list"
filter_kinds = ["issue", "task"]
sort_column = "kind"
"#;
        std::fs::write(&path, v1_toml).unwrap();

        let loaded = load_state(&db_path).unwrap();
        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.filter_kinds, vec!["issue", "task"]);

        let mut app = App::new(vec![make_thread_row("ISSUE-001", "issue")]);
        loaded.apply_to_app(&mut app);
        // both issue + task → execution; bug + task tags
        assert!(app.filter.lifecycles.contains("execution"));
        assert!(app.filter.tags.contains("bug"));
        assert!(app.filter.tags.contains("task"));
        // "kind" was a v1 alias for the removed Lifecycle sort column;
        // it now falls back to the Updated default.
        assert_eq!(app.sort_column, SortColumn::Updated);
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
