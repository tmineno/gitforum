//! Module integration tests for `src/internal/index.rs` and
//! `src/internal/reindex.rs` (test-policy.md category 1). Search,
//! TUI startup reindex, and Track G reverse-link queries land here in
//! later splits.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::reindex;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
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
