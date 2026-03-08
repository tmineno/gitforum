use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};

use super::error::ForumResult;
use super::git_ops::GitOps;
use super::index::{self, ThreadRow};
use super::show;
use super::thread;

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    List,
    Detail(String),
}

/// Application state for the TUI.
pub struct App {
    pub view: View,
    pub threads: Vec<ThreadRow>,
    pub table_state: TableState,
    pub kind_filter: Option<String>,
    pub detail_text: String,
    pub detail_scroll: u16,
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
            detail_text: String::new(),
            detail_scroll: 0,
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
            Some("rfc") => Some("decision".into()),
            _ => None,
        };
        let n = self.visible_threads().len();
        self.table_state.select(if n > 0 { Some(0) } else { None });
    }
}

/// Run the interactive TUI.
///
/// Preconditions: `db_path` points to a valid index (run `reindex` first, or index is auto-created).
/// Postconditions: terminal is restored on exit.
/// Failure modes: ForumError::Io on terminal I/O failure; ForumError::Repo on index/replay errors.
/// Side effects: modifies terminal state; restores on exit.
pub fn run(git: &GitOps, db_path: &Path, initial_thread_id: Option<&str>) -> ForumResult<()> {
    let conn = index::open_db(db_path)?;
    let threads = index::list_threads(&conn)?;

    let mut app = App::new(threads);
    if let Some(thread_id) = initial_thread_id {
        open_detail(&mut app, git, thread_id)?;
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app, git, &conn);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

pub(crate) fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    git: &GitOps,
    conn: &rusqlite::Connection,
) -> ForumResult<()> {
    loop {
        terminal.draw(|f| render(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(app, key, git, conn)? {
                    return Ok(());
                }
            }
        }
    }
}

fn handle_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    git: &GitOps,
    conn: &rusqlite::Connection,
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
            KeyCode::Char('r') => {
                let threads = index::list_threads(conn)?;
                let sel = app.table_state.selected().unwrap_or(0);
                app.threads = threads;
                let n = app.visible_threads().len();
                app.table_state
                    .select(if n > 0 { Some(sel.min(n - 1)) } else { None });
            }
            KeyCode::Enter => {
                if let Some(id) = app.selected_thread_id() {
                    open_detail(app, git, &id)?;
                }
            }
            _ => {}
        },
        View::Detail(_) => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                app.view = View::List;
                app.detail_text.clear();
                app.detail_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.detail_scroll = app.detail_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.detail_scroll = app.detail_scroll.saturating_sub(1);
            }
            KeyCode::Char('r') => {
                if let View::Detail(id) = app.view.clone() {
                    open_detail(app, git, &id)?;
                }
            }
            _ => {}
        },
    }
    Ok(false)
}

fn open_detail(app: &mut App, git: &GitOps, thread_id: &str) -> ForumResult<()> {
    let state = thread::replay_thread(git, thread_id)?;
    app.detail_text = show::render_show(&state);
    app.detail_scroll = 0;
    app.view = View::Detail(thread_id.to_string());
    Ok(())
}

/// Render the current app state into `frame`.
pub fn render(f: &mut Frame, app: &mut App) {
    match app.view {
        View::List => render_list(f, f.area(), app),
        View::Detail(_) => render_detail(f, f.area(), app),
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
        " [q]quit  [enter]detail  [r]refresh  [f]filter:{filter_label}  [j/k]navigate"
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

    f.render_stateful_widget(table, chunks[1], &mut app.table_state);

    f.render_widget(Paragraph::new(format!(" {count} threads")), chunks[2]);
}

pub(crate) fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(
        Paragraph::new(" [esc/q]back  [r]refresh  [j/k]scroll"),
        chunks[0],
    );

    let thread_id = if let View::Detail(ref id) = app.view {
        id.as_str()
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(app.detail_text.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {thread_id} ")),
            )
            .scroll((app.detail_scroll, 0)),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn make_row(id: &str, kind: &str, status: &str, title: &str) -> ThreadRow {
        ThreadRow {
            id: id.into(),
            kind: kind.into(),
            status: status.into(),
            title: title.into(),
            body: None,
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
        app.view = View::Detail("RFC-0001".into());
        app.detail_text = "RFC-0001 Test RFC\nkind: rfc\n".into();
        let out = render_to_string(&mut app, 80, 20);
        assert!(out.contains("RFC-0001"));
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
        assert_eq!(app.kind_filter.as_deref(), Some("decision"));
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
}
