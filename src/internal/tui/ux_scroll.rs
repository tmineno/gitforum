//! INV-13 / AT-18 of doc/spec/TUI-UX-TESTING.md: scrolling a detail pane
//! down stops exactly where its last drawn line reaches the bottom row, and
//! switching Markdown, the split or the terminal size never leaves the
//! scroll past the new end.
//!
//! The limit comes from a second render of the same state in a terminal
//! of the same width and enough rows for the whole pane, so it follows
//! whatever the pane draws (header, sections, footer, word wrap, wide
//! characters, Markdown).

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use tempfile::TempDir;

use crate::internal::config::RepoPaths;
use crate::internal::git_ops::GitOps;

use super::effects::RecordingEffects;
use super::render::render;
use super::ux_fixture::{copy_tree, Fixture, Templates};
use super::{dispatch_event, App, View};

/// First height of the reference render (see `Session::limit`).
const TALL: u16 = 128;
const SIZES: [(u16, u16); 5] = [(40, 12), (60, 20), (80, 24), (120, 40), (200, 60)];
const THREADS: [&str; 3] = [
    "キャッシュ方針の提案 🚀",
    "Crash on narrow terminals",
    "決定: ID は @ なしで表示する",
];
/// (thread, node row) for the node detail cases: the execution's long
/// ASCII action, and the proposal's first two (short, Japanese) nodes.
const NODES: [(&str, usize); 3] = [
    ("Crash on narrow terminals", 1),
    ("キャッシュ方針の提案 🚀", 1),
    ("キャッシュ方針の提案 🚀", 2),
];

#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Body,
    Node,
}

struct Session {
    _dir: TempDir,
    git: GitOps,
    db_path: std::path::PathBuf,
    app: App,
}

impl Session {
    /// The full fixture with `title` open (and node `row` of it, if given).
    fn open(templates: &Templates, title: &str, node_row: Option<usize>) -> Self {
        let dir = TempDir::new().unwrap();
        copy_tree(templates.path(Fixture::Full), dir.path());
        let git = GitOps::new(dir.path().to_path_buf());
        let db_path = RepoPaths::from_repo_root(dir.path())
            .git_forum
            .join("index.db");
        let mut app = App::new(templates.listing(Fixture::Full).rows());
        app.effects = Box::new(RecordingEffects::default());
        let mut s = Self {
            _dir: dir,
            git,
            db_path,
            app,
        };
        let row = s
            .app
            .visible_threads()
            .iter()
            .position(|t| t.title == title)
            .unwrap_or_else(|| panic!("{title:?} is not in the fixture"));
        s.app.table_state.select(Some(row));
        s.key(KeyCode::Enter);
        if let Some(n) = node_row {
            for _ in 0..n {
                s.key(KeyCode::Char('j'));
            }
            s.key(KeyCode::Enter);
            assert!(matches!(s.app.view, View::NodeDetail { .. }));
        }
        s
    }

    fn key(&mut self, code: KeyCode) {
        let e = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        dispatch_event(&mut self.app, e, &self.git, &self.db_path);
    }

    fn wheel_down(&mut self, at: (u16, u16)) {
        let e = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        });
        dispatch_event(&mut self.app, e, &self.git, &self.db_path);
    }

    /// Draw at `w`x`h`; returns the screen as text rows.
    fn draw(&mut self, w: u16, h: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let app = &mut self.app;
        terminal.draw(|f| render(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn rect(&self, pane: Pane) -> Rect {
        match pane {
            Pane::Body => self.app.ui_rects.thread_body,
            Pane::Node => self.app.ui_rects.node_detail,
        }
        .expect("the pane is on screen")
    }

    fn scroll(&self, pane: Pane) -> usize {
        match pane {
            Pane::Body => self.app.thread_scroll,
            Pane::Node => self.app.node_detail_scroll,
        }
        .into()
    }

    fn set_scroll(&mut self, pane: Pane, v: u16) {
        match pane {
            Pane::Body => self.app.thread_scroll = v,
            Pane::Node => self.app.node_detail_scroll = v,
        }
    }

    /// INV-13's limit at width `w` and height `h`, from renders of the
    /// same state scrolled to the top. The height doubles until doubling no
    /// longer moves the last non-blank row (a blank row inside the text can
    /// land on the pane's last row, so "the last row is blank" is not
    /// enough). Leaves the pane drawn at `w`x`h` with the scroll it had.
    fn limit(&mut self, pane: Pane, w: u16, h: u16) -> usize {
        let scroll = self.scroll(pane);
        let last_at = |s: &mut Self, tall: u16| {
            s.set_scroll(pane, 0);
            let screen = s.draw(w, tall);
            let rows = inner_rows(&screen, s.rect(pane));
            rows.iter().rposition(|r| !r.trim().is_empty())
        };
        let mut tall = TALL;
        let mut last = last_at(self, tall);
        loop {
            tall = tall.checked_mul(2).expect("the pane never fits");
            let taller = last_at(self, tall);
            if taller == last {
                break;
            }
            last = taller;
        }
        self.set_scroll(pane, u16::try_from(scroll).unwrap());
        self.draw(w, h);
        let view = usize::from(self.rect(pane).height.saturating_sub(2));
        last.map_or(0, |l| (l + 1).saturating_sub(view))
    }
}

/// The rows inside a bordered pane, as text.
fn inner_rows(screen: &[String], r: Rect) -> Vec<String> {
    (r.y + 1..(r.y + r.height).saturating_sub(1))
        .map(|y| {
            screen[usize::from(y)]
                .chars()
                .skip(usize::from(r.x) + 1)
                .take(usize::from(r.width.saturating_sub(2)))
                .collect()
        })
        .collect()
}

fn center(r: Rect) -> (u16, u16) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

/// INV-13 (a) for one state: the wheel and the key both stop at the limit.
fn check_stop(s: &mut Session, pane: Pane, w: u16, h: u16, name: &str) -> Vec<String> {
    let mut failures = Vec::new();
    s.set_scroll(pane, 0);
    let limit = s.limit(pane, w, h);
    let at = center(s.rect(pane));
    for _ in 0..limit + 20 {
        s.wheel_down(at);
    }
    s.draw(w, h);
    let by_wheel = s.scroll(pane);
    s.set_scroll(pane, 0);
    s.draw(w, h);
    match pane {
        Pane::Body => s.key(KeyCode::End),
        Pane::Node => {
            for _ in 0..limit + 20 {
                s.key(KeyCode::Char('j'));
            }
        }
    }
    s.draw(w, h);
    let by_key = s.scroll(pane);
    if by_wheel != limit || by_key != limit {
        failures.push(format!(
            "{name}: limit {limit}, wheel stops at {by_wheel}, key stops at {by_key}"
        ));
    }
    failures
}

/// AT-18, INV-13 (a).
#[test]
fn scroll_stops_at_the_last_line() {
    let templates = Templates::build();
    let body = |title: &str| {
        let mut s = Session::open(&templates, title, None);
        let mut failures = Vec::new();
        for (w, h) in SIZES {
            for md in [false, true] {
                for horizontal in [false, true] {
                    s.app.markdown_mode = md;
                    s.app.split_horizontal = horizontal;
                    let split = if horizontal { "h" } else { "v" };
                    let name = format!("{title} body {w}x{h} md={md} split={split}");
                    failures.extend(check_stop(&mut s, Pane::Body, w, h, &name));
                }
            }
        }
        failures
    };
    let node = |(title, row): (&str, usize)| {
        let mut s = Session::open(&templates, title, Some(row));
        let mut failures = Vec::new();
        for (w, h) in SIZES {
            for md in [false, true] {
                s.app.markdown_mode = md;
                let name = format!("{title} node {row} {w}x{h} md={md}");
                failures.extend(check_stop(&mut s, Pane::Node, w, h, &name));
            }
        }
        failures
    };
    // One session per thread or node, each on its own thread.
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = THREADS
            .into_iter()
            .map(|title| scope.spawn(move || body(title)))
            .chain(NODES.into_iter().map(|n| scope.spawn(move || node(n))))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} cases do not stop at the limit:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A change of display; returns the terminal size to draw at afterwards.
type Switch = fn(&mut Session) -> (u16, u16);

/// AT-18, INV-13 (b): scrolled to the end at 80x24, then a switch that
/// changes what the body pane draws.
#[test]
fn switching_the_display_keeps_the_scroll_within_the_limit() {
    let templates = Templates::build();
    let switches: [(&str, Switch); 4] = [
        ("markdown on", |s| {
            s.key(KeyCode::Char('m'));
            (80, 24)
        }),
        ("split horizontal", |s| {
            s.key(KeyCode::Char('t'));
            (80, 24)
        }),
        ("resize to 120x40", |_| (120, 40)),
        ("resize to 200x60", |_| (200, 60)),
    ];
    let mut failures = Vec::new();
    for (name, switch) in switches {
        let mut s = Session::open(&templates, THREADS[0], None);
        s.draw(80, 24);
        s.key(KeyCode::End);
        s.draw(80, 24);
        let before = s.scroll(Pane::Body);
        let (w, h) = switch(&mut s);
        let screen = s.draw(w, h);
        let after = s.scroll(Pane::Body);
        let blank = inner_rows(&screen, s.rect(Pane::Body))
            .iter()
            .all(|r| r.trim().is_empty());
        let limit = s.limit(Pane::Body, w, h);
        if after > limit || blank {
            failures.push(format!(
                "{name}: scroll {before} -> {after}, new limit {limit}, pane blank={blank}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
