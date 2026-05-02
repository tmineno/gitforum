//! Module integration tests for `src/internal/index.rs` and
//! `src/internal/reindex.rs` (test-policy.md category 1). Also covers
//! the TUI startup path that triggers a reindex via
//! `tui::load_threads`. Track G's reverse-link query coverage will
//! land here in a later split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{self, Event, EventType, NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::reindex;
use git_forum::internal::tui;
use git_forum::internal::write_ops;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn test_thread_id(kind: ThreadKind, seed: u8) -> String {
    id_alloc::alloc_thread_id_with_nonce(
        kind,
        "human/alice",
        "test",
        "2026-01-01T00:00:00Z",
        &[seed, seed, seed, seed, seed, seed, seed, seed],
    )
}

fn sample_create(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
    Event {
        event_id: "evt-0001".into(),
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        title: Some(title.into()),
        kind: Some(kind),
        ..Event::default()
    }
}

fn make_thread(git: &GitOps, kind: ThreadKind, title: &str) -> String {
    create::create_thread(git, kind, title, None, "human/alice", &fixed_clock()).unwrap()
}

fn add_node(git: &GitOps, thread_id: &str, node_type: NodeType, body: &str) {
    write_ops::say_node(
        git,
        thread_id,
        node_type,
        body,
        "human/alice",
        &FixedClock {
            instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        },
        None,
    )
    .unwrap();
}

// ---- Reindex ----

#[test]
fn reindex_empty_repo() {
    let (_repo, git, paths) = setup();
    let db_path = paths.git_forum.join("index.db");
    let report = reindex::run_reindex(&git, &db_path).unwrap();
    assert_eq!(report.threads_found, 0);
    assert!(report.errors.is_empty());
}

#[test]
fn reindex_replays_all_threads() {
    let (_repo, git, paths) = setup();
    let ask_id = test_thread_id(ThreadKind::Issue, 7);
    let rfc_id = test_thread_id(ThreadKind::Rfc, 8);
    event::write_event(&git, &sample_create(&ask_id, ThreadKind::Issue, "Bug")).unwrap();
    event::write_event(&git, &sample_create(&rfc_id, ThreadKind::Rfc, "Proposal")).unwrap();

    let db_path = paths.git_forum.join("index.db");
    let report = reindex::run_reindex(&git, &db_path).unwrap();
    assert_eq!(report.threads_found, 2);
    assert_eq!(report.threads_replayed.len(), 2);
    assert!(report.errors.is_empty());
}

#[test]
fn reindex_populates_index() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "My RFC");
    make_thread(&git, ThreadKind::Issue, "My Bug");

    let db_path = paths.git_forum.join("index.db");
    let report = reindex::run_reindex(&git, &db_path).unwrap();
    assert_eq!(report.threads_found, 2);
    assert_eq!(report.threads_replayed.len(), 2);
    assert!(report.errors.is_empty());

    let conn = index::open_db(&db_path).unwrap();
    let rows = index::list_threads(&conn).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.kind == "rfc"));
    assert!(rows.iter().any(|r| r.kind == "issue"));
}

#[test]
fn reindex_is_idempotent() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "My RFC");

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let rows = index::list_threads(&conn).unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn reindex_stores_correct_status() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "Draft RFC");

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let rows = index::list_threads(&conn).unwrap();
    assert_eq!(rows[0].status, "draft");
    assert_eq!(rows[0].kind, "rfc");
}

// ---- Index unit ----

#[test]
fn open_db_creates_file_and_schema() {
    let (_repo, _git, paths) = setup();
    let db_path = paths.git_forum.join("index.db");
    let conn = index::open_db(&db_path).unwrap();
    let rows = index::list_threads(&conn).unwrap();
    assert!(rows.is_empty());
    assert!(db_path.exists());
}

// ---- Search ----

#[test]
fn search_finds_by_title() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "Unique Title Here");
    make_thread(&git, ThreadKind::Issue, "Other Issue");

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let results = index::search_threads(&conn, "Unique").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].thread.title, "Unique Title Here");
}

#[test]
fn search_finds_by_kind() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "RFC A");
    make_thread(&git, ThreadKind::Issue, "Issue B");

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let results = index::search_threads(&conn, "rfc").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].thread.kind, "rfc");
}

#[test]
fn search_finds_by_current_node_body_and_reports_hit() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let thread_id = make_thread(&git, ThreadKind::Rfc, "RFC A");
    add_node(
        &git,
        &thread_id,
        NodeType::Question,
        "Where is the migration plan?",
    );

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let results = index::search_threads(&conn, "migration").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].thread.id, thread_id);
    assert_eq!(results[0].node_hits.len(), 1);
    // SPEC-2.0 §2.5: `question` is canonicalized to `comment` on write.
    assert_eq!(results[0].node_hits[0].node_type, "comment");
    assert!(results[0].node_hits[0].body.contains("migration plan"));
}

#[test]
fn search_empty_returns_no_match_message_via_render() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "RFC");

    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();

    let conn = index::open_db(&db_path).unwrap();
    let results = index::search_threads(&conn, "zzznomatch").unwrap();
    let out = git_forum::internal::ls::render_search_results(&results);
    assert_eq!(out, "no threads found\n");
}

// ---- TUI startup reindex ----

#[test]
fn tui_load_threads_reindexes_on_startup() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    make_thread(&git, ThreadKind::Rfc, "TUI Visible RFC");

    let db_path = paths.git_forum.join("index.db");
    let rows = tui::load_threads(&git, &db_path).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "TUI Visible RFC");
}
