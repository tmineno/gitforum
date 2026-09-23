//! Invariant checks for the TUI UX suite (doc/spec/TUI-UX-TESTING.md:
//! INV-2, INV-4 to INV-8, INV-10, INV-11, INV-14). Each check is a pure
//! function of the state before and after one event and the screen after
//! it (drawn twice for INV-14), so the tests at the bottom can feed it
//! hand-built violations (AT-2, AT-15).
//! INV-1 is a panic and INV-3 needs live input; both live in ux_invariants.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::ux_invariants::{mode_of, Mode};
use super::{App, EventOutcome, LinkFormField, NodeFormField, ThreadFormField, UiRects, View};

/// INV-2 and INV-10 apply from this size up (spec: 80x24).
const MIN_W: u16 = 80;
const MIN_H: u16 = 24;

/// Table H: the exit / dismiss hint each mode must show.
fn hint(mode: Mode) -> &'static str {
    match mode {
        Mode::List => "[q]quit",
        Mode::FilterBar => "[esc]cancel",
        Mode::ThreadDetail | Mode::NodeDetail => "[esc/q]back",
        Mode::CreateThread | Mode::CreateNode | Mode::CreateLink => "[esc]cancel",
        Mode::EditThreadBody | Mode::EditNodeBody => "[esc]back",
        Mode::TextSelect => "press any key to return",
        Mode::ConfirmDiscard => "Press y to discard, any other key to cancel",
        Mode::ErrorFlash => "Press any key to dismiss",
    }
}

/// A recorded click area, named after its `UiRects` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Area {
    ListTable,
    ColumnHeader(usize),
    FilterLabel,
    FilterPopup,
    FilterKinds,
    FilterStatuses,
    HelpLine,
    ThreadBody,
    ThreadNodes,
    NodeDetail,
    ThreadSubmit,
    NodeSubmit,
    LinkSubmit,
    Dropdown,
    FormField(usize),
}

/// Every recorded area. The destructuring names every `UiRects` field, so a
/// new field does not compile until it is mapped here (spec failure mode 12).
pub(super) fn areas(r: &UiRects) -> Vec<(Area, Rect)> {
    let UiRects {
        list_table,
        thread_body,
        thread_nodes,
        node_detail,
        thread_submit,
        node_submit,
        link_submit,
        dropdown,
        column_headers,
        filter_label,
        filter_popup,
        filter_kind_area,
        filter_status_area,
        help_line,
        form_fields,
    } = *r;
    let mut out: Vec<(Area, Option<Rect>)> = vec![
        (Area::ListTable, list_table),
        (Area::FilterLabel, filter_label),
        (Area::FilterPopup, filter_popup),
        (Area::FilterKinds, filter_kind_area),
        (Area::FilterStatuses, filter_status_area),
        (Area::HelpLine, help_line),
        (Area::ThreadBody, thread_body),
        (Area::ThreadNodes, thread_nodes),
        (Area::NodeDetail, node_detail),
        (Area::ThreadSubmit, thread_submit),
        (Area::NodeSubmit, node_submit),
        (Area::LinkSubmit, link_submit),
        (Area::Dropdown, dropdown),
    ];
    out.extend(
        column_headers
            .into_iter()
            .enumerate()
            .map(|(i, r)| (Area::ColumnHeader(i), r)),
    );
    out.extend(
        form_fields
            .into_iter()
            .enumerate()
            .map(|(i, r)| (Area::FormField(i), r)),
    );
    out.into_iter()
        .filter_map(|(a, r)| r.map(|r| (a, r)))
        .filter(|(_, r)| r.width > 0 && r.height > 0)
        .collect()
}

/// Table M: which areas a screen may record. Overlays keep the areas of
/// the view underneath them.
fn allowed(view: &View, filter_open: bool, area: Area) -> bool {
    use Area::*;
    match view {
        View::List => {
            matches!(area, ListTable | ColumnHeader(_) | FilterLabel)
                || (filter_open
                    && matches!(area, FilterPopup | FilterKinds | FilterStatuses | HelpLine))
        }
        View::ThreadDetail(_) => matches!(area, HelpLine | ThreadBody | ThreadNodes),
        View::NodeDetail { .. } => matches!(area, HelpLine | NodeDetail),
        View::CreateThread => {
            matches!(area, HelpLine | ThreadSubmit | Dropdown | FormField(0..=3))
        }
        View::CreateNode { .. } => {
            matches!(area, HelpLine | NodeSubmit | Dropdown | FormField(0..=2))
        }
        View::CreateLink { .. } => {
            matches!(area, HelpLine | LinkSubmit | Dropdown | FormField(0..=3))
        }
        View::EditThreadBody | View::EditNodeBody { .. } => matches!(area, HelpLine),
    }
}

/// The label a table M "(ラベル)" row must show inside its area.
fn label(view: &View, area: Area) -> Option<&'static str> {
    const HEADERS: [&str; 6] = ["ID", "STATUS", "VIS", "CREATED", "UPDATED", "TITLE"];
    match (view, area) {
        (_, Area::ColumnHeader(i)) => HEADERS.get(i).copied(),
        (_, Area::FilterLabel) => Some("[f]filter:"),
        (View::ThreadDetail(_) | View::NodeDetail { .. }, Area::HelpLine) => Some("[esc/q]back"),
        (View::EditThreadBody | View::EditNodeBody { .. }, Area::HelpLine) => Some("[esc]back"),
        // The forms, and the list with the filter bar open.
        (_, Area::HelpLine) => Some("[esc]cancel"),
        (_, Area::ThreadSubmit | Area::NodeSubmit | Area::LinkSubmit) => Some("submit"),
        (View::CreateThread, Area::FormField(i)) => {
            ["lifecycle", "tags", "title", "body"].get(i).copied()
        }
        (View::CreateNode { .. }, Area::FormField(i)) => ["type", "body", "submit"].get(i).copied(),
        (View::CreateLink { .. }, Area::FormField(i)) => {
            ["relation", "target kind", "target", "submit"]
                .get(i)
                .copied()
        }
        _ => None,
    }
}

/// What the checks read from `App`, taken before and after each event.
#[derive(Debug, Clone)]
pub(super) struct Snapshot {
    pub(super) mode: Mode,
    pub(super) view: View,
    pub(super) filter_open: bool,
    pub(super) confirm_discard: bool,
    pub(super) error_flash: bool,
    /// Spec "フォームの未保存入力" for the form on screen (tags included).
    pub(super) unsaved: bool,
    pub(super) on_submit: bool,
    pub(super) forms: String,
    pub(super) list_selected: Option<usize>,
    pub(super) visible_len: usize,
    pub(super) node_selected: Option<usize>,
    pub(super) node_rows: usize,
    pub(super) thread_count: usize,
    pub(super) node_count: usize,
    pub(super) thread_text_empty: bool,
    pub(super) node_text_empty: bool,
    pub(super) rects: UiRects,
}

impl Snapshot {
    pub(super) fn of(app: &App) -> Self {
        let (unsaved, on_submit) = match &app.view {
            View::CreateThread | View::EditThreadBody => {
                let f = &app.thread_form;
                (
                    !(f.title.is_empty() && f.tags.is_empty() && f.body.is_empty()),
                    f.field == ThreadFormField::Submit,
                )
            }
            View::CreateNode { .. } | View::EditNodeBody { .. } => (
                !app.node_form.body.is_empty(),
                app.node_form.field == NodeFormField::Submit,
            ),
            View::CreateLink { .. } => (
                !app.link_form.manual_target.is_empty(),
                app.link_form.field == LinkFormField::Submit,
            ),
            _ => (false, false),
        };
        Self {
            mode: mode_of(app),
            view: app.view.clone(),
            filter_open: app.filter_bar.is_some(),
            confirm_discard: app.confirm_discard,
            error_flash: app.error_flash.is_some(),
            unsaved,
            on_submit,
            forms: format!(
                "{:?} {:?} {:?}",
                app.thread_form, app.node_form, app.link_form
            ),
            list_selected: app.table_state.selected(),
            visible_len: app.visible_threads().len(),
            node_selected: app.node_table_state.selected(),
            node_rows: app.visible_tree_indices.len() + 1,
            thread_count: app.threads.len(),
            node_count: app.thread_nodes.len(),
            thread_text_empty: app.thread_text.trim().is_empty(),
            node_text_empty: app.node_detail_text.trim().is_empty(),
            rects: app.ui_rects,
        }
    }

    /// A blank snapshot on `view`, for hand-built check inputs.
    #[cfg(test)]
    fn blank(view: View) -> Self {
        let mode = match view {
            View::List => Mode::List,
            View::ThreadDetail(_) => Mode::ThreadDetail,
            View::NodeDetail { .. } => Mode::NodeDetail,
            View::CreateThread => Mode::CreateThread,
            View::EditThreadBody => Mode::EditThreadBody,
            View::CreateNode { .. } => Mode::CreateNode,
            View::EditNodeBody { .. } => Mode::EditNodeBody,
            View::CreateLink { .. } => Mode::CreateLink,
        };
        Self {
            mode,
            view,
            filter_open: false,
            confirm_discard: false,
            error_flash: false,
            unsaved: false,
            on_submit: false,
            forms: String::new(),
            list_selected: None,
            visible_len: 0,
            node_selected: None,
            node_rows: 1,
            thread_count: 0,
            node_count: 0,
            thread_text_empty: true,
            node_text_empty: true,
            rects: UiRects::default(),
        }
    }
}

/// One processed event: the state around it and the screen after it.
pub(super) struct Step<'a> {
    pub(super) before: &'a Snapshot,
    pub(super) after: &'a Snapshot,
    pub(super) event: Option<&'a Event>,
    pub(super) outcome: Option<&'a EventOutcome>,
    /// The screen after a second draw with no event in between, as the event
    /// loop draws every 100 ms; `after.rects` come from this draw.
    pub(super) screen: &'a Buffer,
    /// The first draw after the event (INV-14 compares it with `screen`);
    /// `screen` itself on a step that quit.
    pub(super) first: &'a Buffer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Violation {
    pub(super) inv: &'static str,
    pub(super) detail: String,
}

fn violation(inv: &'static str, detail: impl Into<String>) -> Violation {
    Violation {
        inv,
        detail: detail.into(),
    }
}

// ------------------------------------------------------------------
//  Event and screen helpers
// ------------------------------------------------------------------

fn key_of(event: Option<&Event>) -> Option<KeyEvent> {
    match event {
        Some(Event::Key(k)) => Some(*k),
        _ => None,
    }
}

fn is_ctrl_c(k: &KeyEvent) -> bool {
    k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL)
}

/// Clicks and wheel turns: the mouse input INV-11 covers. Releases, drags
/// and moves may leave an overlay up (a move must not dismiss anything).
fn is_click_or_wheel(event: Option<&Event>) -> bool {
    matches!(
        event,
        Some(Event::Mouse(m)) if matches!(
            m.kind,
            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        )
    )
}

fn is_mouse(event: Option<&Event>) -> bool {
    matches!(event, Some(Event::Mouse(_)))
}

fn is_form(view: &View) -> bool {
    matches!(
        view,
        View::CreateThread
            | View::EditThreadBody
            | View::CreateNode { .. }
            | View::EditNodeBody { .. }
            | View::CreateLink { .. }
    )
}

fn row_text(screen: &Buffer, y: u16, x0: u16, x1: u16) -> String {
    (x0..x1).map(|x| screen[(x, y)].symbol()).collect()
}

fn screen_contains(screen: &Buffer, needle: &str) -> bool {
    let a = screen.area;
    (a.y..a.y + a.height).any(|y| row_text(screen, y, a.x, a.x + a.width).contains(needle))
}

pub(super) fn inside(frame: Rect, r: Rect) -> bool {
    r.x >= frame.x
        && r.y >= frame.y
        && r.x + r.width <= frame.x + frame.width
        && r.y + r.height <= frame.y + frame.height
}

fn full_size(screen: &Buffer) -> bool {
    screen.area.width >= MIN_W && screen.area.height >= MIN_H
}

/// Unchanged apart from the overlay itself (INV-8, INV-11).
fn same_underneath(a: &Snapshot, b: &Snapshot) -> bool {
    a.view == b.view
        && a.forms == b.forms
        && a.list_selected == b.list_selected
        && a.node_selected == b.node_selected
        && a.thread_count == b.thread_count
        && a.node_count == b.node_count
}

// ------------------------------------------------------------------
//  Checks
// ------------------------------------------------------------------

/// INV-2: the mode's exit / dismiss hint is on screen, from 80x24 up.
fn inv2(step: &Step) -> Option<Violation> {
    let want = hint(step.after.mode);
    (full_size(step.screen) && !screen_contains(step.screen, want))
        .then(|| violation("INV-2", format!("{:?} shows no {want:?}", step.after.mode)))
}

/// INV-4: Esc never quits; Ctrl-C quits at once, except that on an overlay
/// the first Ctrl-C only closes it.
fn inv4(step: &Step) -> Option<Violation> {
    let k = key_of(step.event)?;
    let quit = step.outcome == Some(&EventOutcome::Quit);
    if k.code == KeyCode::Esc && quit {
        return Some(violation("INV-4", "Esc quit the TUI"));
    }
    if is_ctrl_c(&k) {
        let on_overlay = matches!(step.before.mode, Mode::ConfirmDiscard | Mode::ErrorFlash);
        if on_overlay && (quit || step.after.confirm_discard || step.after.error_flash) {
            return Some(violation(
                "INV-4",
                format!(
                    "Ctrl-C on {:?} must only close the overlay",
                    step.before.mode
                ),
            ));
        }
        if !on_overlay && !quit {
            return Some(violation(
                "INV-4",
                format!("Ctrl-C on {:?} did not quit", step.before.mode),
            ));
        }
    }
    None
}

/// INV-5: unsaved form input leaves the forms only by submitting, by `y`
/// on the discard confirmation, or by Ctrl-C.
fn inv5(step: &Step) -> Option<Violation> {
    let (b, a) = (step.before, step.after);
    if !(b.unsaved && is_form(&b.view)) || is_form(&a.view) {
        return None;
    }
    let allowed = match step.event {
        Some(Event::Key(k)) => {
            (b.confirm_discard && matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y')))
                || (b.on_submit && k.code == KeyCode::Enter)
        }
        Some(Event::Mouse(m)) => {
            m.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left)
                && areas(&b.rects).into_iter().any(|(area, r)| {
                    matches!(
                        area,
                        Area::ThreadSubmit | Area::NodeSubmit | Area::LinkSubmit
                    ) && super::input::rect_contains(r, m.column, m.row)
                })
        }
        _ => false,
    };
    (!allowed).then(|| {
        violation(
            "INV-5",
            format!(
                "unsaved input in {:?} was dropped on the way to {:?}",
                b.view, a.view
            ),
        )
    })
}

/// INV-6: selections in range, and a pane with text never scrolls blank.
fn inv6(step: &Step) -> Option<Violation> {
    let a = step.after;
    match a.list_selected {
        None if a.visible_len > 0 => {
            return Some(violation("INV-6", "list has rows but no selection"));
        }
        Some(i) if i >= a.visible_len => {
            return Some(violation(
                "INV-6",
                format!("list selection {i} >= {} rows", a.visible_len),
            ));
        }
        _ => {}
    }
    if matches!(a.view, View::ThreadDetail(_)) {
        if let Some(i) = a.node_selected.filter(|&i| i >= a.node_rows) {
            return Some(violation(
                "INV-6",
                format!("node selection {i} >= {} rows", a.node_rows),
            ));
        }
    }
    let panes = [
        (Area::ThreadBody, a.thread_text_empty, "thread body"),
        (Area::NodeDetail, a.node_text_empty, "node detail"),
    ];
    for (want, empty, name) in panes {
        if empty || !matches!(a.mode, Mode::ThreadDetail | Mode::NodeDetail) {
            continue;
        }
        for (area, r) in areas(&a.rects) {
            if area != want || !inside(step.screen.area, r) || r.width < 3 || r.height < 3 {
                continue;
            }
            let blank = (r.y + 1..r.y + r.height - 1).all(|y| {
                row_text(step.screen, y, r.x + 1, r.x + r.width - 1)
                    .trim()
                    .is_empty()
            });
            if blank {
                return Some(violation("INV-6", format!("{name} pane scrolled blank")));
            }
        }
    }
    None
}

/// INV-7: every click area lies inside the frame.
fn inv7(step: &Step) -> Option<Violation> {
    let frame = step.screen.area;
    areas(&step.after.rects)
        .into_iter()
        .find(|(_, r)| !inside(frame, *r))
        .map(|(area, r)| {
            violation(
                "INV-7",
                format!("{area:?} {r:?} is outside the {frame:?} frame"),
            )
        })
}

/// INV-8: keys on an overlay (see also INV-4 for Ctrl-C).
fn inv8(step: &Step) -> Option<Violation> {
    let k = key_of(step.event)?;
    let (b, a) = (step.before, step.after);
    match b.mode {
        Mode::ErrorFlash if a.error_flash || !same_underneath(b, a) => Some(violation(
            "INV-8",
            "a key on the error flash did more than dismiss it",
        )),
        Mode::ConfirmDiscard => {
            let yes = matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y'));
            if a.confirm_discard {
                Some(violation("INV-8", "the confirmation stayed up after a key"))
            } else if yes && a.view == b.view {
                Some(violation(
                    "INV-8",
                    "y on the confirmation did not leave the form",
                ))
            } else if !yes && !same_underneath(b, a) {
                Some(violation(
                    "INV-8",
                    "a key other than y changed the form under the confirmation",
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// INV-10: every click area is in table M for this screen, and from 80x24
/// up a labelled area shows its label.
fn inv10(step: &Step) -> Option<Violation> {
    let a = step.after;
    for (area, r) in areas(&a.rects) {
        if !allowed(&a.view, a.filter_open, area) {
            return Some(violation(
                "INV-10",
                format!(
                    "{area:?} is recorded on {:?} but table M has no row for it",
                    a.view
                ),
            ));
        }
        let Some(want) = label(&a.view, area) else {
            continue;
        };
        if !full_size(step.screen) || !inside(step.screen.area, r) {
            continue;
        }
        let shown = (r.y..r.y + r.height)
            .any(|y| row_text(step.screen, y, r.x, r.x + r.width).contains(want));
        if !shown {
            return Some(violation(
                "INV-10",
                format!("{area:?} {r:?} does not show its label {want:?}"),
            ));
        }
    }
    None
}

/// INV-11: a click or wheel turn on an overlay only closes it.
fn inv11(step: &Step) -> Option<Violation> {
    let (b, a) = (step.before, step.after);
    if !matches!(b.mode, Mode::ConfirmDiscard | Mode::ErrorFlash) || !is_mouse(step.event) {
        return None;
    }
    if !same_underneath(b, a) {
        return Some(violation(
            "INV-11",
            format!("mouse input on {:?} changed the screen underneath", b.mode),
        ));
    }
    let still_up = a.confirm_discard || a.error_flash;
    (is_click_or_wheel(step.event) && still_up).then(|| {
        violation(
            "INV-11",
            format!("a click on {:?} did not close it", b.mode),
        )
    })
}

/// INV-14: drawing again with no input gives the same screen. The event
/// loop redraws every 100 ms without input, so a difference is a flicker.
fn inv14(step: &Step) -> Option<Violation> {
    let (a, b) = (step.first, step.screen);
    if a == b {
        return None;
    }
    if a.area != b.area {
        return Some(violation(
            "INV-14",
            format!(
                "drawn again, the screen went from {:?} to {:?}",
                a.area, b.area
            ),
        ));
    }
    let r = a.area;
    let y =
        (r.y..r.y + r.height).find(|&y| (r.x..r.x + r.width).any(|x| a[(x, y)] != b[(x, y)]))?;
    let (was, now) = (
        row_text(a, y, r.x, r.x + r.width),
        row_text(b, y, r.x, r.x + r.width),
    );
    Some(violation(
        "INV-14",
        format!("drawn again with no input, row {y} changed: {was:?} -> {now:?}"),
    ))
}

/// INV-5 for a step that quit: only Ctrl-C may drop unsaved input.
fn inv5_quit(step: &Step) -> Option<Violation> {
    let b = step.before;
    let ctrl_c = key_of(step.event).is_some_and(|k| is_ctrl_c(&k));
    (b.unsaved && is_form(&b.view) && !ctrl_c).then(|| {
        violation(
            "INV-5",
            format!(
                "a quit other than Ctrl-C dropped unsaved input in {:?}",
                b.view
            ),
        )
    })
}

/// Every per-step check, in spec order. A step that quit draws nothing more,
/// so only the quit itself is checked.
pub(super) fn check_step(step: &Step) -> Vec<Violation> {
    let checks: &[fn(&Step) -> Option<Violation>] = if step.outcome == Some(&EventOutcome::Quit) {
        &[inv4, inv5_quit]
    } else {
        &[inv2, inv4, inv5, inv6, inv7, inv8, inv10, inv11, inv14]
    };
    checks.iter().filter_map(|check| check(step)).collect()
}

// ------------------------------------------------------------------
//  AT-2 / AT-15: each check reports a hand-built violation
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEvent};
    use ratatui::style::Style;

    fn screen(lines: &[(u16, &str)]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        for &(y, text) in lines {
            buf.set_string(0, y, text, Style::default());
        }
        buf
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn click(column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn run(
        before: &Snapshot,
        after: &Snapshot,
        event: Option<&Event>,
        outcome: Option<&EventOutcome>,
        buf: &Buffer,
    ) -> Vec<&'static str> {
        check_step(&Step {
            before,
            after,
            event,
            outcome,
            screen: buf,
            first: buf,
        })
        .into_iter()
        .map(|v| v.inv)
        .collect()
    }

    fn list_ok() -> Snapshot {
        let mut s = Snapshot::blank(View::List);
        s.visible_len = 3;
        s.list_selected = Some(0);
        s
    }

    #[test]
    fn clean_step_reports_nothing() {
        let s = list_ok();
        let buf = screen(&[(0, " [q]quit")]);
        assert_eq!(run(&s, &s, None, None, &buf), Vec::<&str>::new());
    }

    #[test]
    fn inv2_missing_hint() {
        let s = list_ok();
        assert_eq!(run(&s, &s, None, None, &screen(&[])), vec!["INV-2"]);
    }

    #[test]
    fn inv4_esc_quits() {
        let s = list_ok();
        let buf = screen(&[(0, " [q]quit")]);
        let esc = key(KeyCode::Esc);
        assert_eq!(
            run(&s, &s, Some(&esc), Some(&EventOutcome::Quit), &buf),
            vec!["INV-4"]
        );
    }

    #[test]
    fn inv5_dropped_input() {
        let mut before = Snapshot::blank(View::CreateThread);
        before.unsaved = true;
        let after = list_ok();
        let buf = screen(&[(0, " [q]quit")]);
        let esc = key(KeyCode::Esc);
        assert_eq!(
            run(
                &before,
                &after,
                Some(&esc),
                Some(&EventOutcome::Continue),
                &buf
            ),
            vec!["INV-5"]
        );
    }

    #[test]
    fn inv5_quit_other_than_ctrl_c() {
        let mut s = Snapshot::blank(View::CreateThread);
        s.unsaved = true;
        let q = key(KeyCode::Char('q'));
        let quit = Some(&EventOutcome::Quit);
        assert_eq!(run(&s, &s, Some(&q), quit, &screen(&[])), vec!["INV-5"]);
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            run(&s, &s, Some(&ctrl_c), quit, &screen(&[])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn inv6_selection_out_of_range() {
        let mut s = list_ok();
        s.list_selected = Some(3);
        let buf = screen(&[(0, " [q]quit")]);
        assert_eq!(run(&s, &s, None, None, &buf), vec!["INV-6"]);
    }

    #[test]
    fn inv7_area_outside_frame() {
        let mut s = list_ok();
        s.rects.list_table = Some(Rect::new(70, 20, 20, 10));
        let buf = screen(&[(0, " [q]quit")]);
        assert_eq!(run(&s, &s, None, None, &buf), vec!["INV-7"]);
    }

    #[test]
    fn inv8_key_on_error_flash_acts() {
        let mut before = list_ok();
        before.mode = Mode::ErrorFlash;
        before.error_flash = true;
        let mut after = list_ok();
        after.list_selected = Some(1);
        let buf = screen(&[(0, " [q]quit")]);
        let j = key(KeyCode::Char('j'));
        assert_eq!(
            run(
                &before,
                &after,
                Some(&j),
                Some(&EventOutcome::Continue),
                &buf
            ),
            vec!["INV-8"]
        );
    }

    #[test]
    fn inv10_label_missing() {
        let mut s = Snapshot::blank(View::ThreadDetail("t".into()));
        s.mode = Mode::ThreadDetail;
        s.rects.help_line = Some(Rect::new(1, 0, 11, 1));
        let buf = screen(&[(0, " [esc]close   [esc/q]back")]);
        assert_eq!(run(&s, &s, None, None, &buf), vec!["INV-10"]);
    }

    #[test]
    fn inv10_area_not_in_table_m() {
        // A body editor has no choice list.
        let mut s = Snapshot::blank(View::EditThreadBody);
        s.rects.dropdown = Some(Rect::new(40, 2, 20, 5));
        let buf = screen(&[(0, " [ctrl+s]done  [esc]back")]);
        let found = check_step(&Step {
            before: &s,
            after: &s,
            event: None,
            outcome: None,
            screen: &buf,
            first: &buf,
        });
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].inv, "INV-10");
        assert!(found[0].detail.contains("table M has no row"), "{found:?}");
    }

    #[test]
    fn inv14_screen_changes_when_drawn_again() {
        let s = list_ok();
        let buf = screen(&[(0, " [q]quit"), (2, "ID  CREATED   UPDATED")]);
        let again = screen(&[(0, " [q]quit"), (2, "ID  CREATE UPDATED")]);
        let found = check_step(&Step {
            before: &s,
            after: &s,
            event: None,
            outcome: None,
            screen: &again,
            first: &buf,
        });
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].inv, "INV-14");
        assert!(found[0].detail.contains("row 2 changed"), "{found:?}");
    }

    #[test]
    fn inv11_click_on_confirmation_reaches_form() {
        let mut before = Snapshot::blank(View::CreateThread);
        before.mode = Mode::ConfirmDiscard;
        before.confirm_discard = true;
        before.unsaved = true;
        let mut after = before.clone();
        after.forms = "submitted".into();
        let buf = screen(&[(0, "Press y to discard, any other key to cancel")]);
        let c = click(5, 7);
        assert_eq!(
            run(
                &before,
                &after,
                Some(&c),
                Some(&EventOutcome::Continue),
                &buf
            ),
            vec!["INV-11"]
        );
    }
}
