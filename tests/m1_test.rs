mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::doctor;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::reindex;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

fn sample_create(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
    Event {
        event_id: "evt-0001".into(),
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        base_rev: None,
        parents: vec![],
        title: Some(title.into()),
        kind: Some(kind),
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: None,
    }
}

fn sample_state(thread_id: &str, new_state: &str) -> Event {
    Event {
        event_id: "evt-0002".into(),
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/bob".into(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: Some(new_state.into()),
    }
}

// ---- Init tests ----

#[test]
fn init_creates_forum_structure() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();

    assert!(paths.dot_forum.join("policy.toml").exists());
    assert!(paths.dot_forum.join("actors.toml").exists());
    assert!(paths.dot_forum.join("templates").join("issue.md").exists());
    assert!(paths.dot_forum.join("templates").join("rfc.md").exists());
    assert!(paths
        .dot_forum
        .join("templates")
        .join("decision.md")
        .exists());
    assert!(paths.git_forum.join("logs").is_dir());
}

#[test]
fn init_policy_is_valid_toml() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let content = std::fs::read_to_string(paths.dot_forum.join("policy.toml")).unwrap();
    let _parsed: toml::Table = content.parse().expect("policy.toml should be valid TOML");
}

#[test]
fn init_is_idempotent() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();
    init::init_forum(&paths).unwrap(); // second call should not error
    assert!(paths.dot_forum.join("policy.toml").exists());
}

// ---- Event storage tests ----

#[test]
fn write_and_read_event_roundtrip() {
    let (_repo, git, _paths) = setup();
    let ev = sample_create("RFC-0001", ThreadKind::Rfc, "Test RFC");

    let commit_sha = event::write_event(&git, &ev).unwrap();
    assert!(!commit_sha.is_empty());

    let loaded = event::read_event(&git, &commit_sha).unwrap();
    assert_eq!(loaded.event_id, "evt-0001");
    assert_eq!(loaded.event_type, EventType::Create);
    assert_eq!(loaded.thread_id, "RFC-0001");
    assert_eq!(loaded.title.as_deref(), Some("Test RFC"));
    assert_eq!(loaded.kind, Some(ThreadKind::Rfc));
}

#[test]
fn write_two_events_creates_parent_chain() {
    let (_repo, git, _paths) = setup();
    let create = sample_create("RFC-0001", ThreadKind::Rfc, "Test RFC");
    let state = sample_state("RFC-0001", "proposed");

    let sha1 = event::write_event(&git, &create).unwrap();
    let sha2 = event::write_event(&git, &state).unwrap();
    assert_ne!(sha1, sha2);

    // rev-list from the ref should show both commits
    let shas = git.rev_list("refs/forum/threads/RFC-0001").unwrap();
    assert_eq!(shas.len(), 2);
    assert_eq!(shas[0], sha2); // newest first
    assert_eq!(shas[1], sha1);
}

#[test]
fn load_thread_events_returns_chronological() {
    let (_repo, git, _paths) = setup();
    let create = sample_create("RFC-0001", ThreadKind::Rfc, "Test RFC");
    let state = sample_state("RFC-0001", "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state).unwrap();

    let events = event::load_thread_events(&git, "RFC-0001").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, EventType::Create);
    assert_eq!(events[1].event_type, EventType::State);
}

// ---- Thread replay tests ----

#[test]
fn replay_thread_from_git() {
    let (_repo, git, _paths) = setup();
    let create = sample_create("RFC-0001", ThreadKind::Rfc, "Test RFC");
    let state_ev = sample_state("RFC-0001", "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state_ev).unwrap();

    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(state.id, "RFC-0001");
    assert_eq!(state.kind, ThreadKind::Rfc);
    assert_eq!(state.title, "Test RFC");
    assert_eq!(state.status, "proposed");
    assert_eq!(state.created_by, "human/alice");
    assert_eq!(state.events.len(), 2);
}

#[test]
fn list_thread_ids_finds_stored_threads() {
    let (_repo, git, _paths) = setup();
    event::write_event(&git, &sample_create("ISSUE-0001", ThreadKind::Issue, "Bug")).unwrap();
    event::write_event(
        &git,
        &sample_create("RFC-0001", ThreadKind::Rfc, "Proposal"),
    )
    .unwrap();

    let ids = thread::list_thread_ids(&git).unwrap();
    assert_eq!(ids, vec!["ISSUE-0001", "RFC-0001"]);
}

#[test]
fn list_thread_ids_empty_repo() {
    let (_repo, git, _paths) = setup();
    let ids = thread::list_thread_ids(&git).unwrap();
    assert!(ids.is_empty());
}

// ---- Doctor tests ----

#[test]
fn doctor_after_init_all_pass() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(
        report.all_passed(),
        "doctor should pass after init: {:?}",
        report
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| (&c.name, &c.detail))
            .collect::<Vec<_>>()
    );
}

#[test]
fn doctor_uninitialized_reports_failures() {
    let (_repo, git, paths) = setup();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    assert!(!report.all_passed());
}

// ---- Reindex tests ----

#[test]
fn reindex_empty_repo() {
    let (_repo, git, _paths) = setup();
    let report = reindex::run_reindex(&git).unwrap();
    assert_eq!(report.threads_found, 0);
    assert!(report.errors.is_empty());
}

#[test]
fn reindex_replays_all_threads() {
    let (_repo, git, _paths) = setup();
    event::write_event(&git, &sample_create("ISSUE-0001", ThreadKind::Issue, "Bug")).unwrap();
    event::write_event(
        &git,
        &sample_create("RFC-0001", ThreadKind::Rfc, "Proposal"),
    )
    .unwrap();

    let report = reindex::run_reindex(&git).unwrap();
    assert_eq!(report.threads_found, 2);
    assert_eq!(report.threads_replayed.len(), 2);
    assert!(report.errors.is_empty());
}
