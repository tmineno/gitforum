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
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::TimeZone;
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

use crate::internal::clock::{Clock, StepClock};
use crate::internal::config::RepoPaths;
use crate::internal::git_ops::GitOps;
use crate::internal::node::{NodeKind, NodeStatus};
use crate::internal::snapshot::{self, list as snapshot_list};

use super::effects::RecordingEffects;
use super::render::render;
use super::state::{snapshot_append_link, snapshot_append_node, snapshot_create_thread};
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

const ACTOR: &str = "human/alice";

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
//  Fixtures
// ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fixture {
    Full,
    Empty,
}

/// Fixture repositories, built once per test and copied for every case.
struct Templates {
    full: (TempDir, GitOps, RepoPaths, PathBuf),
    empty: (TempDir, GitOps, RepoPaths, PathBuf),
}

impl Templates {
    fn build() -> Self {
        let full = super::tests::setup_repo();
        build_full_fixture(&full.1);
        let empty = super::tests::setup_repo();
        Self { full, empty }
    }

    fn path(&self, fixture: Fixture) -> &Path {
        match fixture {
            Fixture::Full => self.full.0.path(),
            Fixture::Empty => self.empty.0.path(),
        }
    }
}

/// Threads in every lifecycle and several statuses, every node kind with
/// nested replies, links, a body longer than the viewport, a Markdown
/// table, CJK and emoji. Created through the TUI's own snapshot writers
/// with a StepClock, so ids and timestamps are the same on every run.
fn build_full_fixture(git: &GitOps) {
    let start = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let clock = StepClock::new(start, chrono::Duration::minutes(1));

    // More rows than an 80x24 list shows, so paging has somewhere to go.
    for i in 0..22 {
        snapshot_create_thread(
            git,
            &format!("Filler thread {i:02}"),
            None,
            "execution",
            &[],
            ACTOR,
            &clock,
        )
        .unwrap();
    }

    let long_body = (1..=80)
        .map(|i| format!("Line {i:02}: long enough that the body pane has to scroll."))
        .collect::<Vec<_>>()
        .join("\n");
    let table = "| 項目 | value |\n|---|---|\n| 日本語 | 🚀 emoji |\n| wide | 表のセルが長い場合 |";

    let record = snapshot_create_thread(
        git,
        "決定: ID は @ なしで表示する",
        Some("短い本文。"),
        "record",
        &[],
        ACTOR,
        &clock,
    )
    .unwrap();
    let execution = snapshot_create_thread(
        git,
        "Crash on narrow terminals",
        Some("Steps:\n1. Open the TUI\n2. Shrink the window"),
        "execution",
        &["bug".to_string()],
        ACTOR,
        &clock,
    )
    .unwrap();
    let proposal = snapshot_create_thread(
        git,
        "キャッシュ方針の提案 🚀",
        Some(&format!("## Goal\n\n{table}\n\n{long_body}")),
        "proposal",
        &[],
        ACTOR,
        &clock,
    )
    .unwrap();

    let node = |thread: &str, kind: NodeKind, body: &str| {
        snapshot_append_node(git, thread, kind, body, ACTOR, &clock).unwrap()
    };
    node(&execution, NodeKind::Action, &long_body);
    let objection = node(&proposal, NodeKind::Objection, "Benchmarks are missing.");
    let reply = node(&proposal, NodeKind::Comment, "ベンチマークを追加します。");
    let nested = node(&proposal, NodeKind::Comment, "Nested reply 🚀");
    let action = node(&proposal, NodeKind::Action, "Add a benchmark");
    let retracted = node(&proposal, NodeKind::Comment, "Withdrawn remark");
    node(&proposal, NodeKind::Approval, "LGTM");

    snapshot_append_link(git, &execution, &proposal, "implements", ACTOR, &clock).unwrap();
    snapshot_append_link(git, &record, &proposal, "relates-to", ACTOR, &clock).unwrap();

    // Statuses and reply nesting have no TUI writer; set them directly. Each
    // edit bumps updated_at, so the proposal (edited last, the thread with
    // nodes) is the first row of the default newest-first list.
    let set = |id: &str, edit: &dyn Fn(&mut snapshot::ThreadDocument)| {
        let mut doc = snapshot::read_snapshot(git, id).unwrap();
        edit(&mut doc);
        doc.snapshot.updated_at = clock.now();
        snapshot::store::write_snapshot(git, id, &doc, "fixture: statuses and replies").unwrap();
    };
    set(&record, &|doc| doc.snapshot.status = "done".into());
    set(&execution, &|doc| doc.snapshot.status = "working".into());
    set(&proposal, &|doc| {
        doc.snapshot.status = "review".into();
        for n in &mut doc.nodes {
            if n.record.id == reply {
                n.record.reply_to = Some(objection.clone());
            } else if n.record.id == nested {
                n.record.reply_to = Some(reply.clone());
            } else if n.record.id == action {
                n.record.status = NodeStatus::Resolved;
            } else if n.record.id == retracted {
                n.record.status = NodeStatus::Retracted;
            }
        }
    });
}

/// A list row whose thread no longer exists: another process removed it
/// after the TUI loaded its list, and the list has not been refreshed.
/// Opening it is the error a user can reach from a healthy repository.
/// Oldest `updated_at`, so it is the last row of the default list.
fn stale_row() -> snapshot_list::ThreadRow {
    snapshot_list::ThreadRow {
        id: "zzzzzzzz".into(),
        kind: "task".into(),
        lifecycle: "execution".into(),
        lifecycle_explicit: true,
        tags: Vec::new(),
        status: "open".into(),
        title: "Removed by another process".into(),
        body: None,
        branch: None,
        created_at: "2000-01-01T00:00:00Z".into(),
        created_by: ACTOR.into(),
        updated_at: "2000-01-01T00:00:00Z".into(),
        visibility: crate::internal::thread::Visibility::Private,
        from_published: false,
    }
}

/// Copy a fixture repository (worktree and `.git`) into `to`.
fn copy_tree(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry.unwrap();
        let dest = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
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
