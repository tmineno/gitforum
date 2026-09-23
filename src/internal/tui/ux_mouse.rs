//! Mouse vs keyboard consistency (doc/spec/TUI-UX-TESTING.md, INV-9,
//! INV-11, INV-12; AT-14, AT-16, AT-17).
//!
//! Each row of table M runs twice from the same fixture state: once with
//! the mouse action, once with the keys the table pairs it with. The two
//! sessions must end in the same observable state. Rows the code is known
//! to violate are listed in [`KNOWN_VIOLATIONS`] with their ticket; a row
//! that stops violating fails the test until its entry is removed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use tempfile::TempDir;

use crate::internal::config::RepoPaths;
use crate::internal::git_ops::GitOps;
use crate::internal::snapshot::list as snapshot_list;

use super::effects::RecordingEffects;
use super::render::render;
use super::ux_fixture::{copy_tree, stale_row, Fixture, Templates};
use super::{dispatch_event, App, FilterField, LinkFormField, NodeFormField, View};

/// Rows that the current code is known to violate: (row, ticket).
const KNOWN_VIOLATIONS: &[(&str, &str)] = &[
    // INV-12: Esc hints outside the detail views are not clickable.
    ("back-label-filter-bar", "4jq8b3kj"),
    ("back-label-create-thread", "4jq8b3kj"),
    ("back-label-create-node", "4jq8b3kj"),
    ("back-label-create-link", "4jq8b3kj"),
    ("back-label-edit-thread-body", "4jq8b3kj"),
    ("back-label-edit-node-body", "4jq8b3kj"),
];

/// One TUI session on its own copy of the full fixture, at 80x24.
struct Session {
    _dir: TempDir,
    git: GitOps,
    db_path: PathBuf,
    app: App,
    terminal: Terminal<TestBackend>,
}

impl Session {
    fn open(templates: &Templates) -> Self {
        let dir = TempDir::new().unwrap();
        copy_tree(templates.path(Fixture::Full), dir.path());
        let git = GitOps::new(dir.path().to_path_buf());
        let db_path = RepoPaths::from_repo_root(dir.path())
            .git_forum
            .join("index.db");
        let mut rows = snapshot_list::list_threads(&git).unwrap();
        rows.push(stale_row());
        let mut app = App::new(rows);
        app.effects = Box::new(RecordingEffects::default());
        let terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut s = Self {
            _dir: dir,
            git,
            db_path,
            app,
            terminal,
        };
        s.draw();
        s
    }

    fn draw(&mut self) {
        let app = &mut self.app;
        self.terminal.draw(|f| render(f, app)).unwrap();
    }

    fn send(&mut self, event: Event) {
        dispatch_event(&mut self.app, event, &self.git, &self.db_path);
        self.draw();
    }

    fn key(&mut self, code: KeyCode) -> &mut Self {
        self.send(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        self
    }

    fn keys(&mut self, codes: &[KeyCode]) -> &mut Self {
        for &code in codes {
            self.key(code);
        }
        self
    }

    fn ctrl(&mut self, c: char) -> &mut Self {
        self.send(Event::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL,
        )));
        self
    }

    fn text(&mut self, s: &str) -> &mut Self {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
        self
    }

    fn mouse(&mut self, kind: MouseEventKind, (column, row): (u16, u16)) -> &mut Self {
        self.send(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }));
        self
    }

    fn click(&mut self, at: (u16, u16)) -> &mut Self {
        self.mouse(MouseEventKind::Down(MouseButton::Left), at)
    }

    fn double_click(&mut self, at: (u16, u16)) -> &mut Self {
        self.app.last_click = None;
        self.click(at).click(at)
    }

    fn wheel_down(&mut self, at: (u16, u16)) -> &mut Self {
        self.mouse(MouseEventKind::ScrollDown, at)
    }

    fn wheel_up(&mut self, at: (u16, u16)) -> &mut Self {
        self.mouse(MouseEventKind::ScrollUp, at)
    }

    /// Press Tab until `done` holds (at most 8 times).
    fn tab_until(&mut self, done: impl Fn(&App) -> bool) -> &mut Self {
        for _ in 0..8 {
            if done(&self.app) {
                return self;
            }
            self.key(KeyCode::Tab);
        }
        panic!("Tab never reached the wanted field");
    }

    /// Top-left cell of the first on-screen occurrence of `needle`.
    fn find(&self, needle: &str) -> (u16, u16) {
        let buf = self.terminal.backend().buffer();
        let want: Vec<String> = needle.chars().map(String::from).collect();
        for y in 0..buf.area.height {
            let row: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(x) = row.windows(want.len()).position(|w| w == want.as_slice()) {
                return (x as u16, y);
            }
        }
        panic!("{needle:?} is not on screen:\n{}", self.screen());
    }

    fn screen(&self) -> String {
        let buf = self.terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rect(&self, pick: impl Fn(&super::UiRects) -> Option<Rect>) -> Rect {
        pick(&self.app.ui_rects).expect("click area not recorded on this screen")
    }

    fn observe(&self) -> Observed {
        let app = &self.app;
        let title = |id: &str| {
            app.threads
                .iter()
                .find(|t| t.id == id)
                .map_or_else(|| id.to_string(), |t| t.title.clone())
        };
        // Thread ids of things created during the row depend on the wall
        // clock, so views name threads by title (spec failure mode 11).
        let view = match &app.view {
            View::ThreadDetail(id) => format!("ThreadDetail({})", title(id)),
            View::NodeDetail { thread_id, node_id } => {
                format!("NodeDetail({}, {node_id})", title(thread_id))
            }
            View::CreateNode { thread_id } => format!("CreateNode({})", title(thread_id)),
            View::EditNodeBody { thread_id } => format!("EditNodeBody({})", title(thread_id)),
            View::CreateLink { thread_id, origin } => {
                format!("CreateLink({}, {origin:?})", title(thread_id))
            }
            other => format!("{other:?}"),
        };
        let mut titles: Vec<String> = snapshot_list::list_threads(&self.git)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        titles.sort();
        Observed {
            view,
            confirm_discard: app.confirm_discard,
            error_flash: app.error_flash.is_some(),
            // The list selection is compared only while the list is on
            // screen, by the title of the selected row: an index into a
            // hidden list can move with wall-clock ids.
            list_selected: matches!(app.view, View::List)
                .then(|| app.selected_thread_id().map(|id| title(&id)))
                .flatten(),
            node_selected: app.node_table_state.selected(),
            node_count: app.thread_nodes.len(),
            thread_scroll: app.thread_scroll,
            node_detail_scroll: app.node_detail_scroll,
            thread_form: format!("{:?}", app.thread_form),
            node_form: format!("{:?}", app.node_form),
            link_form: format!("{:?}", app.link_form),
            filter: format!("{:?} / {:?}", app.filter, app.filter_bar),
            sort: format!("{:?} asc={}", app.sort_column, app.sort_ascending),
            repo_threads: titles,
        }
    }
}

/// The state INV-9 compares.
#[derive(Debug, PartialEq)]
struct Observed {
    view: String,
    confirm_discard: bool,
    error_flash: bool,
    list_selected: Option<String>,
    node_selected: Option<usize>,
    node_count: usize,
    thread_scroll: u16,
    node_detail_scroll: u16,
    thread_form: String,
    node_form: String,
    link_form: String,
    filter: String,
    sort: String,
    repo_threads: Vec<String>,
}

fn first_cell(r: Rect) -> (u16, u16) {
    (r.x, r.y)
}

/// Row `i` of a bordered table with a header line.
fn table_row(r: Rect, i: u16) -> (u16, u16) {
    (r.x + 2, r.y + 2 + i)
}

/// Item `i` of a bordered list.
fn list_item(r: Rect, i: u16) -> (u16, u16) {
    (r.x + 2, r.y + 1 + i)
}

/// A table M row: shared setup, then the mouse side and the key side.
struct Row {
    name: &'static str,
    setup: fn(&mut Session),
    mouse: fn(&mut Session),
    keys: fn(&mut Session),
}

fn no_setup(_: &mut Session) {}

fn open_thread(s: &mut Session) {
    s.key(KeyCode::Enter);
}

fn open_node(s: &mut Session) {
    s.keys(&[KeyCode::Enter, KeyCode::Char('j'), KeyCode::Enter]);
}

fn open_filter(s: &mut Session) {
    s.key(KeyCode::Char('f'));
}

fn create_thread(s: &mut Session) {
    s.key(KeyCode::Char('c'));
}

fn create_node(s: &mut Session) {
    s.keys(&[KeyCode::Enter, KeyCode::Char('c')]);
}

fn create_link(s: &mut Session) {
    s.keys(&[KeyCode::Enter, KeyCode::Char('l')]);
}

fn esc(s: &mut Session) {
    s.key(KeyCode::Esc);
}

fn rows() -> Vec<Row> {
    use KeyCode::{Char, Down, Enter, Esc, Tab, Up};
    vec![
        // ---- List ----
        Row {
            name: "list-row-click",
            setup: no_setup,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.list_table), 2);
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Char('j'), Char('j')]);
            },
        },
        Row {
            name: "list-row-double-click",
            setup: no_setup,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.list_table), 2);
                s.double_click(at);
            },
            keys: |s| {
                s.keys(&[Char('j'), Char('j'), Enter]);
            },
        },
        Row {
            name: "list-wheel-down",
            setup: no_setup,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.list_table), 0);
                s.wheel_down(at);
            },
            keys: |s| {
                s.key(Char('j'));
            },
        },
        Row {
            name: "list-wheel-up",
            setup: |s| {
                s.keys(&[Char('j'), Char('j')]);
            },
            mouse: |s| {
                let at = table_row(s.rect(|r| r.list_table), 0);
                s.wheel_up(at);
            },
            keys: |s| {
                s.key(Char('k'));
            },
        },
        Row {
            name: "list-filter-label",
            setup: no_setup,
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.filter_label));
                s.click(at);
            },
            keys: |s| {
                s.key(Char('f'));
            },
        },
        // ---- Filter bar ----
        Row {
            name: "filterbar-lifecycle-item",
            setup: open_filter,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.filter_kind_area), 1);
                s.click(at);
            },
            keys: |s| {
                s.tab_until(|a| {
                    a.filter_bar
                        .as_ref()
                        .is_some_and(|b| b.field == FilterField::Lifecycle)
                })
                .keys(&[Char('j'), Char(' ')]);
            },
        },
        Row {
            name: "filterbar-status-item",
            setup: open_filter,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.filter_status_area), 2);
                s.click(at);
            },
            keys: |s| {
                s.tab_until(|a| {
                    a.filter_bar
                        .as_ref()
                        .is_some_and(|b| b.field == FilterField::Status)
                })
                .keys(&[Char('j'), Char('j'), Char(' ')]);
            },
        },
        Row {
            name: "filterbar-click-outside",
            setup: open_filter,
            mouse: |s| {
                s.click((0, 23));
            },
            keys: esc,
        },
        // ---- ThreadDetail ----
        Row {
            name: "thread-node-row-click",
            setup: open_thread,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.thread_nodes), 2);
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Char('j'), Char('j')]);
            },
        },
        Row {
            name: "thread-node-row-double-click",
            setup: open_thread,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.thread_nodes), 1);
                s.double_click(at);
            },
            keys: |s| {
                s.keys(&[Char('j'), Enter]);
            },
        },
        Row {
            name: "thread-body-wheel-down",
            setup: open_thread,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.thread_body), 0);
                s.wheel_down(at);
            },
            keys: |s| {
                s.key(Down);
            },
        },
        Row {
            name: "thread-body-wheel-up",
            setup: |s| {
                s.keys(&[Enter, Down, Down]);
            },
            mouse: |s| {
                let at = list_item(s.rect(|r| r.thread_body), 0);
                s.wheel_up(at);
            },
            keys: |s| {
                s.key(Up);
            },
        },
        Row {
            name: "thread-nodes-wheel-down",
            setup: open_thread,
            mouse: |s| {
                let at = table_row(s.rect(|r| r.thread_nodes), 0);
                s.wheel_down(at);
            },
            keys: |s| {
                s.key(Char('j'));
            },
        },
        Row {
            name: "thread-nodes-wheel-up",
            setup: |s| {
                s.keys(&[Enter, Char('j'), Char('j')]);
            },
            mouse: |s| {
                let at = table_row(s.rect(|r| r.thread_nodes), 0);
                s.wheel_up(at);
            },
            keys: |s| {
                s.key(Char('k'));
            },
        },
        Row {
            name: "thread-back-label",
            setup: open_thread,
            mouse: |s| {
                let at = s.find("[esc/q]back");
                s.click(at);
            },
            keys: esc,
        },
        // ---- NodeDetail ----
        Row {
            name: "node-back-label",
            setup: open_node,
            mouse: |s| {
                let at = s.find("[esc/q]back");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "node-wheel-down",
            setup: open_node,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.node_detail), 0);
                s.wheel_down(at);
            },
            keys: |s| {
                s.key(Char('j'));
            },
        },
        Row {
            name: "node-wheel-up",
            setup: |s| {
                open_node(s);
                s.keys(&[Char('j'), Char('j')]);
            },
            mouse: |s| {
                let at = list_item(s.rect(|r| r.node_detail), 0);
                s.wheel_up(at);
            },
            keys: |s| {
                s.key(Char('k'));
            },
        },
        // ---- Forms: field labels, choice lists, submit ----
        Row {
            name: "create-thread-field-label",
            setup: create_thread,
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.form_fields[2]));
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Tab, Tab]);
            },
        },
        Row {
            name: "create-node-field-label",
            setup: create_node,
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.form_fields[1]));
                s.click(at);
            },
            keys: |s| {
                s.key(Tab);
            },
        },
        Row {
            name: "create-link-field-label",
            setup: create_link,
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.form_fields[2]));
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Tab, Tab]);
            },
        },
        Row {
            name: "create-thread-choice",
            setup: create_thread,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.dropdown), 2);
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Down, Down]);
            },
        },
        Row {
            name: "create-node-choice",
            setup: create_node,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.dropdown), 1);
                s.click(at);
            },
            keys: |s| {
                s.key(Down);
            },
        },
        Row {
            name: "create-link-choice",
            setup: create_link,
            mouse: |s| {
                let at = list_item(s.rect(|r| r.dropdown), 2);
                s.click(at);
            },
            keys: |s| {
                s.keys(&[Down, Down]);
            },
        },
        Row {
            name: "create-thread-submit",
            setup: |s| {
                create_thread(s);
                s.keys(&[Tab, Tab]).text("Made by the mouse row");
            },
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.thread_submit));
                s.click(at);
            },
            keys: |s| {
                s.tab_until(|a| a.thread_form.field == super::ThreadFormField::Submit)
                    .key(Enter);
            },
        },
        Row {
            name: "create-node-submit",
            setup: |s| {
                create_node(s);
                s.keys(&[Tab, Enter]).text("A note").ctrl('s');
            },
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.node_submit));
                s.click(at);
            },
            keys: |s| {
                s.tab_until(|a| a.node_form.field == NodeFormField::Submit)
                    .key(Enter);
            },
        },
        Row {
            name: "create-link-submit",
            setup: create_link,
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.link_submit));
                s.click(at);
            },
            keys: |s| {
                s.tab_until(|a| a.link_form.field == LinkFormField::Submit)
                    .key(Enter);
            },
        },
        // ---- INV-11: clicks on overlays ----
        Row {
            name: "confirm-discard-click",
            setup: |s| {
                create_thread(s);
                s.keys(&[Tab, Tab]).text("Half-written").key(Esc);
                assert!(s.app.confirm_discard, "setup must show the confirmation");
            },
            mouse: |s| {
                let at = first_cell(s.rect(|r| r.thread_submit));
                s.click(at);
            },
            keys: |s| {
                s.key(Char('n'));
            },
        },
        Row {
            name: "error-flash-click",
            setup: |s| {
                s.keys(&[KeyCode::End, Enter]);
                assert!(s.app.error_flash.is_some(), "setup must show an error");
            },
            mouse: |s| {
                s.click((2, 3));
            },
            keys: |s| {
                s.key(Char('j'));
            },
        },
        // ---- INV-12: Esc hints are clickable everywhere ----
        Row {
            name: "back-label-filter-bar",
            setup: open_filter,
            mouse: |s| {
                let at = s.find("[esc]cancel");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "back-label-create-thread",
            setup: create_thread,
            mouse: |s| {
                let at = s.find("[esc]cancel");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "back-label-create-node",
            setup: create_node,
            mouse: |s| {
                let at = s.find("[esc]cancel");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "back-label-create-link",
            setup: create_link,
            mouse: |s| {
                let at = s.find("[esc]cancel");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "back-label-edit-thread-body",
            setup: |s| {
                create_thread(s);
                s.keys(&[Tab, Tab, Tab, Enter]);
            },
            mouse: |s| {
                let at = s.find("[esc]back");
                s.click(at);
            },
            keys: esc,
        },
        Row {
            name: "back-label-edit-node-body",
            setup: |s| {
                create_node(s);
                s.keys(&[Tab, Enter]);
            },
            mouse: |s| {
                let at = s.find("[esc]back");
                s.click(at);
            },
            keys: esc,
        },
    ]
}

/// Run one row on two sessions; Some(details) when the two sides end in
/// different states (INV-9, and INV-12 for the back-label rows).
fn compare(row: &Row, templates: &Templates) -> Option<String> {
    let mut by_mouse = Session::open(templates);
    let mut by_keys = Session::open(templates);
    (row.setup)(&mut by_mouse);
    (row.setup)(&mut by_keys);
    assert_eq!(
        by_mouse.observe(),
        by_keys.observe(),
        "{}: setup diverged",
        row.name
    );
    (row.mouse)(&mut by_mouse);
    (row.keys)(&mut by_keys);
    let (m, k) = (by_mouse.observe(), by_keys.observe());
    (m != k).then(|| {
        format!(
            "mouse: {m:#?}\nkeys:  {k:#?}\nscreen after mouse:\n{}",
            by_mouse.screen()
        )
    })
}

/// AT-14 / AT-16 / AT-17: every table M row gives the same state by mouse
/// and by keys, except the rows listed in KNOWN_VIOLATIONS.
#[test]
fn mouse_matches_keyboard() {
    let templates = Templates::build();
    let rows = rows();
    // Rows are independent: a few workers each take the next row.
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let failing: BTreeMap<&str, String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| {
                    let mut found = Vec::new();
                    while let Some(row) = rows.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if let Some(details) = compare(row, &templates) {
                            found.push((row.name, details));
                        }
                    }
                    found
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });

    let known: BTreeMap<&str, &str> = KNOWN_VIOLATIONS.iter().copied().collect();
    let new: Vec<&&str> = failing.keys().filter(|n| !known.contains_key(*n)).collect();
    let stale: Vec<&&str> = known.keys().filter(|n| !failing.contains_key(*n)).collect();
    let details: String = new
        .iter()
        .map(|n| format!("\n==== {n} ====\n{}", failing[**n]))
        .collect();
    assert!(
        new.is_empty() && stale.is_empty(),
        "mouse and keys disagree on rows not in KNOWN_VIOLATIONS: {new:?}\n\
         KNOWN_VIOLATIONS rows that now agree (remove them): {stale:?}{details}"
    );
}

/// AT-2, INV-9 and INV-12: rows whose two sides differ are reported, and a
/// row whose sides agree is not.
#[test]
fn compare_reports_planted_mismatches() {
    let templates = Templates::build();
    let planted = [
        // INV-9: the mouse side leaves the selection where the keys move it.
        Row {
            name: "planted-inv9",
            setup: no_setup,
            mouse: |_| {},
            keys: |s| {
                s.key(KeyCode::Down);
            },
        },
        // INV-12: a hint that does nothing when clicked, paired with Esc.
        Row {
            name: "planted-inv12",
            setup: open_thread,
            mouse: |s| {
                let at = s.find("[enter]node");
                s.click(at);
            },
            keys: esc,
        },
    ];
    for row in &planted {
        assert!(
            compare(row, &templates).is_some(),
            "{} was not reported",
            row.name
        );
    }
    let agree = Row {
        name: "planted-agree",
        setup: open_thread,
        mouse: esc,
        keys: esc,
    };
    assert_eq!(compare(&agree, &templates), None);
}
