//! Module integration tests for `src/internal/thread.rs` — replay and
//! thread enumeration (test-policy.md category 1). Thread creation,
//! resolution, and timestamp override tests are added in the m2 split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::thread;

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

fn sample_state(thread_id: &str, new_state: &str) -> Event {
    Event {
        event_id: "evt-0002".into(),
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/bob".into(),
        new_state: Some(new_state.into()),
        ..Event::default()
    }
}

#[test]
fn replay_thread_from_git() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 4);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state_ev = sample_state(&tid, "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state_ev).unwrap();

    let state = thread::replay_thread(&git, &tid).unwrap();
    assert_eq!(state.id, tid);
    assert_eq!(state.kind, ThreadKind::Rfc);
    assert_eq!(state.title, "Test RFC");
    assert_eq!(state.status, "proposed");
    assert_eq!(state.created_by, "human/alice");
    assert_eq!(state.events.len(), 2);
}

#[test]
fn list_thread_ids_finds_stored_threads() {
    let (_repo, git, _paths) = setup();
    let ask_id = test_thread_id(ThreadKind::Issue, 5);
    let rfc_id = test_thread_id(ThreadKind::Rfc, 6);
    event::write_event(&git, &sample_create(&ask_id, ThreadKind::Issue, "Bug")).unwrap();
    event::write_event(&git, &sample_create(&rfc_id, ThreadKind::Rfc, "Proposal")).unwrap();

    let ids = thread::list_thread_ids(&git).unwrap();
    assert_eq!(ids.len(), 2);
    assert!(
        ids.iter().any(|id| id == &ask_id),
        "should contain ASK thread"
    );
    assert!(
        ids.iter().any(|id| id == &rfc_id),
        "should contain RFC thread"
    );
    // The fixture's `test_thread_id` helper still uses the legacy
    // kind-prefixed form to exercise the 1.x-on-disk path that
    // list_thread_ids must keep reading; assert any valid form.
    for id in &ids {
        assert!(
            id_alloc::is_valid_thread_id(id),
            "expected valid thread id, got: {id}"
        );
    }
}

#[test]
fn list_thread_ids_empty_repo() {
    let (_repo, git, _paths) = setup();
    let ids = thread::list_thread_ids(&git).unwrap();
    assert!(ids.is_empty());
}
