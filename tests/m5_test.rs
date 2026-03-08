mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::SequentialIdGenerator;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::reindex;

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

fn make_thread(git: &GitOps, kind: ThreadKind, title: &str) -> String {
    create::create_thread(
        git,
        kind,
        title,
        None,
        "human/alice",
        &fixed_clock(),
        &SequentialIdGenerator::new("t"),
    )
    .unwrap()
}

// ---- Index unit tests ----

#[test]
fn open_db_creates_file_and_schema() {
    let (_repo, _git, paths) = setup();
    let db_path = paths.git_forum.join("index.db");
    let conn = index::open_db(&db_path).unwrap();
    // Schema must allow list_threads to run without error
    let rows = index::list_threads(&conn).unwrap();
    assert!(rows.is_empty());
    assert!(db_path.exists());
}

// ---- Reindex integration tests ----

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
    reindex::run_reindex(&git, &db_path).unwrap(); // second run replaces rows

    let conn = index::open_db(&db_path).unwrap();
    let rows = index::list_threads(&conn).unwrap();
    assert_eq!(rows.len(), 1); // no duplicates
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

// ---- Search tests ----

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
    assert_eq!(results[0].title, "Unique Title Here");
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
    assert_eq!(results[0].kind, "rfc");
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
    let out = git_forum::internal::show::render_ls_from_index(&results);
    assert_eq!(out, "no threads found\n");
}
