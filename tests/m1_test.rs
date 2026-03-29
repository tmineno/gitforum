mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::{CommitIdentity, RepoPaths};
use git_forum::internal::doctor::{self, CheckLevel};
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::reindex;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

/// Generate a deterministic opaque thread ID for test fixtures.
/// Uses a nonce derived from `seed` so each call site gets a distinct but reproducible ID.
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
        base_rev: None,
        parents: vec![],
        title: Some(title.into()),
        kind: Some(kind),
        body: None,
        branch: None,
        node_type: None,
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        incorporated_node_ids: vec![],
        reply_to: None,
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
        branch: None,
        node_type: None,
        target_node_id: None,
        new_state: Some(new_state.into()),
        approvals: vec![],
        evidence: None,
        link_rel: None,
        incorporated_node_ids: vec![],
        reply_to: None,
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
    let tid = test_thread_id(ThreadKind::Rfc, 1);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Test RFC");

    let commit_sha = event::write_event(&git, &ev).unwrap();
    assert!(!commit_sha.is_empty());

    let loaded = event::read_event(&git, &commit_sha).unwrap();
    assert_eq!(loaded.event_id, commit_sha);
    assert_eq!(loaded.event_type, EventType::Create);
    assert_eq!(loaded.thread_id, tid);
    assert_eq!(loaded.title.as_deref(), Some("Test RFC"));
    assert_eq!(loaded.kind, Some(ThreadKind::Rfc));
}

#[test]
fn write_two_events_creates_parent_chain() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 2);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state = sample_state(&tid, "proposed");

    let sha1 = event::write_event(&git, &create).unwrap();
    let sha2 = event::write_event(&git, &state).unwrap();
    assert_ne!(sha1, sha2);

    // rev-list from the ref should show both commits
    let shas = git.rev_list(&format!("refs/forum/threads/{tid}")).unwrap();
    assert_eq!(shas.len(), 2);
    assert_eq!(shas[0], sha2); // newest first
    assert_eq!(shas[1], sha1);
}

#[test]
fn load_thread_events_returns_chronological() {
    let (_repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 3);
    let create = sample_create(&tid, ThreadKind::Rfc, "Test RFC");
    let state = sample_state(&tid, "proposed");
    event::write_event(&git, &create).unwrap();
    event::write_event(&git, &state).unwrap();

    let events = event::load_thread_events(&git, &tid).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, EventType::Create);
    assert_eq!(events[1].event_type, EventType::State);
}

// ---- Commit identity tests ----

/// Helper: read the author name from a git commit.
fn commit_author_name(repo_path: &std::path::Path, sha: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%an", sha])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git log failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Helper: read the author email from a git commit.
fn commit_author_email(repo_path: &std::path::Path, sha: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ae", sha])
        .current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git log failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn commit_uses_git_config_by_default() {
    let (repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 10);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Default identity");
    let sha = event::write_event(&git, &ev).unwrap();

    // Without a commit identity configured, the author comes from git config.
    // We verify it's non-empty (actual value depends on the host's git config
    // since GitOps doesn't isolate from global config in plumbing commands).
    let name = commit_author_name(repo.path(), &sha);
    let email = commit_author_email(repo.path(), &sha);
    assert!(!name.is_empty(), "commit should have an author name");
    assert!(!email.is_empty(), "commit should have an author email");
}

#[test]
fn commit_identity_overrides_author_name_and_email() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: Some("Forum Bot".into()),
        email: Some("bot@forum.local".into()),
    });
    let tid = test_thread_id(ThreadKind::Rfc, 11);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Custom identity");
    let sha = event::write_event(&git, &ev).unwrap();

    assert_eq!(commit_author_name(repo.path(), &sha), "Forum Bot");
    assert_eq!(commit_author_email(repo.path(), &sha), "bot@forum.local");
}

#[test]
fn commit_identity_partial_override_name_only() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: Some("Pseudonym".into()),
        email: None,
    });
    let tid = test_thread_id(ThreadKind::Rfc, 12);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Name-only override");
    let sha = event::write_event(&git, &ev).unwrap();

    assert_eq!(commit_author_name(repo.path(), &sha), "Pseudonym");
    // Email falls through to git config (not overridden)
    let email = commit_author_email(repo.path(), &sha);
    assert!(!email.is_empty(), "email should fall through to git config");
}

#[test]
fn commit_identity_partial_override_email_only() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: None,
        email: Some("private@example.com".into()),
    });
    let tid = test_thread_id(ThreadKind::Rfc, 13);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Email-only override");
    let sha = event::write_event(&git, &ev).unwrap();

    // Name falls through to git config (not overridden)
    let name = commit_author_name(repo.path(), &sha);
    assert!(!name.is_empty(), "name should fall through to git config");
    assert_eq!(
        commit_author_email(repo.path(), &sha),
        "private@example.com"
    );
}

// ---- Thread replay tests ----

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
    event::write_event(
        &git,
        &sample_create(&rfc_id, ThreadKind::Rfc, "Proposal"),
    )
    .unwrap();

    let ids = thread::list_thread_ids(&git).unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().any(|id| id == &ask_id), "should contain ASK thread");
    assert!(ids.iter().any(|id| id == &rfc_id), "should contain RFC thread");
    // All returned IDs should be valid opaque IDs
    for id in &ids {
        assert!(id_alloc::is_opaque_id(id), "expected opaque ID, got: {id}");
    }
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
            .filter(|c| !c.passed())
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

#[test]
fn doctor_reports_missing_template_files() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    // Remove one template file
    std::fs::remove_file(paths.dot_forum.join("templates/dec.md")).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let dec_check = report
        .checks
        .iter()
        .find(|c| c.name == "template dec.md")
        .expect("should have dec.md check");
    assert_eq!(dec_check.level, CheckLevel::Fail);
    // Other templates still pass
    let issue_check = report
        .checks
        .iter()
        .find(|c| c.name == "template issue.md")
        .expect("should have issue.md check");
    assert_eq!(issue_check.level, CheckLevel::Ok);
}

#[test]
fn doctor_reports_empty_template_file() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    // Truncate a template to empty
    std::fs::write(paths.dot_forum.join("templates/rfc.md"), "").unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let rfc_check = report
        .checks
        .iter()
        .find(|c| c.name == "template rfc.md")
        .expect("should have rfc.md check");
    assert_eq!(rfc_check.level, CheckLevel::Fail);
}

#[test]
fn doctor_warns_on_missing_index() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let idx_check = report
        .checks
        .iter()
        .find(|c| c.name == "index database")
        .expect("should have index database check");
    assert_eq!(idx_check.level, CheckLevel::Warn);
    // Warnings don't fail the report
    assert!(report.all_passed());
}

#[test]
fn doctor_passes_index_after_reindex() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(&git, &db_path).unwrap();
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let integrity_check = report
        .checks
        .iter()
        .find(|c| c.name == "index integrity")
        .expect("should have integrity check");
    assert_eq!(integrity_check.level, CheckLevel::Ok);
}

#[test]
fn doctor_warns_on_stale_index() {
    let (_repo, git, paths) = setup();
    init::init_forum(&paths).unwrap();
    let db_path = paths.git_forum.join("index.db");
    // Reindex with no threads
    reindex::run_reindex(&git, &db_path).unwrap();
    // Create a thread after reindexing
    let clock = git_forum::internal::clock::FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    };
    let created_id = git_forum::internal::create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    assert!(created_id.starts_with("ASK-"), "expected ASK- prefix, got: {created_id}");
    assert!(id_alloc::is_opaque_id(&created_id), "expected opaque ID, got: {created_id}");
    let report = doctor::run_doctor(&git, &paths).unwrap();
    let freshness_check = report
        .checks
        .iter()
        .find(|c| c.name == "index freshness")
        .expect("should have freshness check");
    assert_eq!(freshness_check.level, CheckLevel::Warn);
    // Stale index is a warning, not a failure
    assert!(report.all_passed());
}

// ---- Reindex tests ----

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
    event::write_event(
        &git,
        &sample_create(&rfc_id, ThreadKind::Rfc, "Proposal"),
    )
    .unwrap();

    let db_path = paths.git_forum.join("index.db");
    let report = reindex::run_reindex(&git, &db_path).unwrap();
    assert_eq!(report.threads_found, 2);
    assert_eq!(report.threads_replayed.len(), 2);
    assert!(report.errors.is_empty());
}
