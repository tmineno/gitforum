//! UX invariant suite for the TUI (doc/spec/TUI-UX-TESTING.md, ADR-012).
//!
//! Generated key / mouse / resize sequences run through `dispatch_event`,
//! the same per-event path as `run_app`, against a fresh copy of a fixture
//! repository, and the screen is re-rendered after every event. Effects go
//! to `RecordingEffects`, so a run never touches the clipboard or terminal.
//!
//! Checked so far: INV-1 (no panic in event handling or rendering) and
//! AT-3 (every mode is reached at the default case count).
//!
//! Knobs: `PROPTEST_CASES` (default [`DEFAULT_CASES`]) and `TUI_UX_SEED`
//! (a u64, or `random`; default [`DEFAULT_SEED`], so CI runs are repeatable).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Instant;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use proptest::prelude::*;
use proptest::test_runner::{
    Config, FileFailurePersistence, RngAlgorithm, TestCaseResult, TestRng, TestRunner,
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
use super::{dispatch_event, App, EventOutcome, UiRects, View};

/// Cases per run when `PROPTEST_CASES` is unset.
const DEFAULT_CASES: u32 = 64;
/// RNG seed when `TUI_UX_SEED` is unset.
const DEFAULT_SEED: u64 = 0x7475_695f_7578; // "tui_ux"
/// Longest generated op sequence after the prefix.
const MAX_OPS: usize = 40;

/// Terminal sizes (spec: 1x1, narrower than the longest help line, 80x24,
/// 200x60). Index 0 is what shrinking moves toward.
const SIZES: [(u16, u16); 4] = [(80, 24), (1, 1), (40, 12), (200, 60)];

// ------------------------------------------------------------------
//  Modes (spec "用語")
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    List,
    FilterBar,
    ThreadDetail,
    NodeDetail,
    CreateThread,
    EditThreadBody,
    CreateNode,
    EditNodeBody,
    CreateLink,
    TextSelect,
    ConfirmDiscard,
    ErrorFlash,
}

const ALL_MODES: [Mode; 12] = [
    Mode::List,
    Mode::FilterBar,
    Mode::ThreadDetail,
    Mode::NodeDetail,
    Mode::CreateThread,
    Mode::EditThreadBody,
    Mode::CreateNode,
    Mode::EditNodeBody,
    Mode::CreateLink,
    Mode::TextSelect,
    Mode::ConfirmDiscard,
    Mode::ErrorFlash,
];

/// The mode that decides how the next input is read. Overlays win over the
/// view underneath, in the order `dispatch_event` checks them.
fn mode_of(app: &App) -> Mode {
    if app.confirm_discard {
        return Mode::ConfirmDiscard;
    }
    if app.error_flash.is_some() {
        return Mode::ErrorFlash;
    }
    if app.mouse_capture_disabled {
        return Mode::TextSelect;
    }
    match &app.view {
        View::List if app.filter_bar.is_some() => Mode::FilterBar,
        View::List => Mode::List,
        View::ThreadDetail(_) => Mode::ThreadDetail,
        View::NodeDetail { .. } => Mode::NodeDetail,
        View::CreateThread => Mode::CreateThread,
        View::EditThreadBody => Mode::EditThreadBody,
        View::CreateNode { .. } => Mode::CreateNode,
        View::EditNodeBody { .. } => Mode::EditNodeBody,
        View::CreateLink { .. } => Mode::CreateLink,
    }
}

// ------------------------------------------------------------------
//  Operations
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseKind {
    Click,
    Release,
    Drag,
    ScrollUp,
    ScrollDown,
}

/// Where a mouse event lands, resolved against the screen at the time it
/// runs: inside one of the recorded click areas, or anywhere in the frame.
/// `fx` / `fy` are positions in 1/256ths of the area.
#[derive(Debug, Clone, Copy)]
enum Target {
    Area { index: usize, fx: u8, fy: u8 },
    Anywhere { fx: u8, fy: u8 },
}

#[derive(Debug, Clone)]
enum Op {
    Key(KeyCode, KeyModifiers),
    Mouse(MouseKind, Target),
    Resize(usize),
}

fn key(code: KeyCode) -> Op {
    Op::Key(code, KeyModifiers::NONE)
}

fn ch(c: char) -> Op {
    key(KeyCode::Char(c))
}

/// Fixed openings that reach the deeper modes, which random keys rarely
/// hit. Index 0 (none) is what shrinking moves toward.
fn prefixes() -> Vec<Vec<Op>> {
    use KeyCode::{End, Enter, Esc, Tab};
    vec![
        vec![],
        vec![key(Enter)],                                        // ThreadDetail
        vec![key(Enter), ch('j'), key(Enter)],                   // NodeDetail
        vec![ch('c')],                                           // CreateThread
        vec![ch('c'), key(Tab), key(Tab), key(Tab), key(Enter)], // EditThreadBody
        vec![key(Enter), ch('c')],                               // CreateNode
        vec![key(Enter), ch('c'), key(Tab), key(Enter)],         // EditNodeBody
        vec![key(Enter), ch('l')],                               // CreateLink
        vec![ch('f')],                                           // FilterBar
        vec![key(Enter), ch('S')],                               // TextSelect
        vec![ch('c'), key(Tab), key(Tab), ch('x'), key(Esc)],    // ConfirmDiscard
        vec![key(End), key(Enter)],                              // ErrorFlash (opens stale_row)
    ]
}

fn op_strategy() -> impl Strategy<Value = Op> {
    use KeyCode::*;
    let navigation = prop::sample::select(vec![
        Char('j'),
        Char('k'),
        Up,
        Down,
        PageUp,
        PageDown,
        Home,
        End,
        Enter,
        Esc,
        Tab,
        Backspace,
        Char(' '),
    ]);
    let commands = prop::sample::select(vec![
        'f', 'c', 'r', 'y', 'l', 'm', 'S', 'z', 't', 'e', 'x', 'o', 'R', 'n', 'Y', 'q',
    ]);
    let text = prop::sample::select(vec!['a', 'Z', '0', '-', ',', '日', '本', '🚀']);
    let quit = prop::sample::select(vec![
        (Char('Q'), KeyModifiers::NONE),
        (Char('c'), KeyModifiers::CONTROL),
    ]);
    let mouse_kind = prop::sample::select(vec![
        MouseKind::Click,
        MouseKind::Release,
        MouseKind::Drag,
        MouseKind::ScrollUp,
        MouseKind::ScrollDown,
    ]);
    let target = prop_oneof![
        3 => (0usize..32, any::<u8>(), any::<u8>())
            .prop_map(|(index, fx, fy)| Target::Area { index, fx, fy }),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(fx, fy)| Target::Anywhere { fx, fy }),
    ];
    prop_oneof![
        8 => navigation.prop_map(key),
        4 => commands.prop_map(ch),
        3 => text.prop_map(ch),
        1 => Just(Op::Key(Char('s'), KeyModifiers::CONTROL)),
        3 => (mouse_kind, target).prop_map(|(k, t)| Op::Mouse(k, t)),
        1 => (0..SIZES.len()).prop_map(Op::Resize),
        // Quit keys end the case, so keep them rare.
        1 => quit.prop_map(|(c, m)| Op::Key(c, m)),
    ]
}

#[derive(Debug, Clone)]
struct Case {
    fixture: Fixture,
    size: usize,
    prefix: usize,
    ops: Vec<Op>,
}

fn case_strategy() -> impl Strategy<Value = Case> {
    let n_prefixes = prefixes().len();
    (
        prop_oneof![9 => Just(Fixture::Full), 1 => Just(Fixture::Empty)],
        0..SIZES.len(),
        0..n_prefixes,
        prop::collection::vec(op_strategy(), 0..MAX_OPS),
    )
        .prop_map(|(fixture, size, prefix, ops)| Case {
            fixture,
            size,
            prefix,
            ops,
        })
}

// ------------------------------------------------------------------
//  Execution
// ------------------------------------------------------------------

/// Non-empty click areas in a fixed order, so `Target::Area { index }`
/// picks the same area for the same screen.
fn click_areas(r: &UiRects) -> Vec<Rect> {
    let singles = [
        r.list_table,
        r.thread_body,
        r.thread_nodes,
        r.node_detail,
        r.thread_submit,
        r.node_submit,
        r.link_submit,
        r.dropdown,
        r.filter_label,
        r.filter_popup,
        r.filter_kind_area,
        r.filter_status_area,
        r.help_line,
    ];
    singles
        .into_iter()
        .chain(r.column_headers)
        .chain(r.form_fields)
        .flatten()
        .filter(|a| a.width > 0 && a.height > 0)
        .collect()
}

fn resolve(target: Target, app: &App, frame: Rect) -> (u16, u16) {
    let (area, fx, fy) = match target {
        Target::Area { index, fx, fy } => {
            let areas = click_areas(&app.ui_rects);
            if areas.is_empty() {
                (frame, fx, fy)
            } else {
                (areas[index % areas.len()], fx, fy)
            }
        }
        Target::Anywhere { fx, fy } => (frame, fx, fy),
    };
    let scale = |start: u16, len: u16, f: u8| start + (len as u32 * f as u32 / 256) as u16;
    (
        scale(area.x, area.width, fx),
        scale(area.y, area.height, fy),
    )
}

fn to_event(op: &Op, app: &App, frame: Rect) -> Option<Event> {
    match *op {
        Op::Key(code, modifiers) => Some(Event::Key(KeyEvent::new(code, modifiers))),
        Op::Mouse(kind, target) => {
            let (column, row) = resolve(target, app, frame);
            let kind = match kind {
                MouseKind::Click => MouseEventKind::Down(MouseButton::Left),
                MouseKind::Release => MouseEventKind::Up(MouseButton::Left),
                MouseKind::Drag => MouseEventKind::Drag(MouseButton::Left),
                MouseKind::ScrollUp => MouseEventKind::ScrollUp,
                MouseKind::ScrollDown => MouseEventKind::ScrollDown,
            };
            Some(Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }))
        }
        Op::Resize(_) => None,
    }
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal.draw(|f| render(f, app)).unwrap();
}

/// Run one case. Panics (INV-1) propagate to the proptest runner, which
/// shrinks the case and reports the smallest one.
fn run_case(case: &Case, templates: &Templates, tally: &RefCell<BTreeMap<Mode, u64>>) {
    let dir = TempDir::new().unwrap();
    copy_tree(templates.path(case.fixture), dir.path());
    let git = GitOps::new(dir.path().to_path_buf());
    let db_path = RepoPaths::from_repo_root(dir.path())
        .git_forum
        .join("index.db");

    let mut rows = snapshot_list::list_threads(&git).unwrap();
    if case.fixture == Fixture::Full {
        rows.push(stale_row());
    }
    let mut app = App::new(rows);
    app.effects = Box::new(RecordingEffects::default());

    let (w, h) = SIZES[case.size];
    let mut frame = Rect::new(0, 0, w, h);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    draw(&mut terminal, &mut app);
    let visit = |app: &App| *tally.borrow_mut().entry(mode_of(app)).or_default() += 1;
    visit(&app);

    let prefix = &prefixes()[case.prefix];
    let mut prev_was_click = false;
    for op in prefix.iter().chain(&case.ops) {
        let is_click = matches!(op, Op::Mouse(MouseKind::Click, _));
        // Double-click is timed with Instant::now(). Model it as "two clicks
        // in a row", independent of how fast this machine runs the ops.
        if is_click && !prev_was_click {
            app.last_click = None;
        }
        prev_was_click = is_click;

        if let Op::Resize(i) = *op {
            let (w, h) = SIZES[i];
            frame = Rect::new(0, 0, w, h);
            terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        } else if let Some(event) = to_event(op, &app, frame) {
            // ExternalEdit would suspend the terminal for $EDITOR; the suite
            // never runs an editor, so it continues like Continue.
            if dispatch_event(&mut app, event, &git, &db_path) == EventOutcome::Quit {
                return;
            }
        }
        draw(&mut terminal, &mut app);
        visit(&app);
    }
}

fn seed_rng() -> (TestRng, String) {
    let seed = match std::env::var("TUI_UX_SEED") {
        Ok(v) if v == "random" => rand_seed(),
        Ok(v) => v
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("TUI_UX_SEED must be a u64 or `random`, got {v:?}")),
        Err(_) => DEFAULT_SEED,
    };
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    (
        TestRng::from_seed(RngAlgorithm::ChaCha, &bytes),
        seed.to_string(),
    )
}

fn rand_seed() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(Instant::now())
}

#[test]
fn tui_ux_invariants() {
    let templates = Templates::build();
    let mut config = Config {
        source_file: Some(file!()),
        failure_persistence: Some(Box::new(FileFailurePersistence::SourceParallel(
            "proptest-regressions",
        ))),
        ..Config::default()
    };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = DEFAULT_CASES;
    }
    let cases = config.cases;
    let (rng, seed) = seed_rng();
    let mut runner = TestRunner::new_with_rng(config, rng);

    let tally = RefCell::new(BTreeMap::new());
    let started = Instant::now();
    let result = runner.run(&case_strategy(), |case| -> TestCaseResult {
        run_case(&case, &templates, &tally);
        Ok(())
    });
    let elapsed = started.elapsed();
    if let Err(e) = result {
        panic!("{e}\nreproduce with TUI_UX_SEED={seed} PROPTEST_CASES={cases}");
    }

    let tally = tally.into_inner();
    eprintln!(
        "tui_ux: seed={seed} cases={cases} elapsed={:.3}s per_case={:.2}ms modes={tally:?}",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / cases as f64,
    );
    // AT-3: a mode the run never reached is a mode nothing above checked.
    let missing: Vec<Mode> = ALL_MODES
        .into_iter()
        .filter(|m| !tally.contains_key(m))
        .collect();
    assert!(
        missing.is_empty(),
        "modes never reached with seed={seed} cases={cases}: {missing:?}"
    );
}
