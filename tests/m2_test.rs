mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::SequentialIdGenerator;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::show;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn seq_ids(prefix: &str) -> SequentialIdGenerator {
    SequentialIdGenerator::new(prefix)
}

// ---- ID allocation ----

#[test]
fn alloc_first_issue_id() {
    let (_repo, git, _paths) = setup();
    let id = id_alloc::alloc_thread_id(&git, ThreadKind::Issue).unwrap();
    assert_eq!(id, "ISSUE-0001");
}

#[test]
fn alloc_second_rfc_id() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "First",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let id = id_alloc::alloc_thread_id(&git, ThreadKind::Rfc).unwrap();
    assert_eq!(id, "RFC-0002");
}

#[test]
fn alloc_ids_per_kind_are_independent() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    // RFC counter starts at 0001 regardless of Issue counter
    let rfc_id = id_alloc::alloc_thread_id(&git, ThreadKind::Rfc).unwrap();
    assert_eq!(rfc_id, "RFC-0001");
}

// ---- Thread creation ----

#[test]
fn create_issue_returns_id() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "First issue",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    assert_eq!(id, "ISSUE-0001");
}

#[test]
fn create_rfc_initial_status_is_draft() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "First RFC",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(state.status, "draft");
    assert_eq!(state.kind, ThreadKind::Rfc);
    assert_eq!(state.title, "First RFC");
}

#[test]
fn create_decision_initial_status_is_proposed() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Decision,
        "Choose algo",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state = thread::replay_thread(&git, "DEC-0001").unwrap();
    assert_eq!(state.status, "proposed");
}

#[test]
fn create_multiple_threads_of_same_kind() {
    let (_repo, git, _paths) = setup();
    let ids = seq_ids("e");
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC A",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC B",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "RFC C",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    let all = thread::list_thread_ids(&git).unwrap();
    assert_eq!(all, vec!["RFC-0001", "RFC-0002", "RFC-0003"]);
}

// ---- ls ----

#[test]
fn ls_shows_all_kinds() {
    let (_repo, git, _paths) = setup();
    let ids = seq_ids("e");
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    let all_ids = thread::list_thread_ids(&git).unwrap();
    let mut states = Vec::new();
    for id in &all_ids {
        states.push(thread::replay_thread(&git, id).unwrap());
    }
    let refs: Vec<&thread::ThreadState> = states.iter().collect();
    let out = show::render_ls(&refs);
    assert!(out.contains("ISSUE-0001"));
    assert!(out.contains("RFC-0001"));
    assert!(out.contains("Bug"));
    assert!(out.contains("Proposal"));
}

#[test]
fn ls_filtered_by_kind() {
    let (_repo, git, _paths) = setup();
    let ids = seq_ids("e");
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
        &ids,
    )
    .unwrap();
    let all_ids = thread::list_thread_ids(&git).unwrap();
    let mut rfc_states = Vec::new();
    for id in &all_ids {
        let s = thread::replay_thread(&git, id).unwrap();
        if s.kind == ThreadKind::Rfc {
            rfc_states.push(s);
        }
    }
    let refs: Vec<&thread::ThreadState> = rfc_states.iter().collect();
    let out = show::render_ls(&refs);
    assert!(!out.contains("ISSUE-0001"));
    assert!(out.contains("RFC-0001"));
}

// ---- show ----

#[test]
fn show_contains_all_required_fields() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body.\nSecond line."),
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    let out = show::render_show(&state);

    assert!(out.contains("RFC-0001"), "missing thread id");
    assert!(out.contains("Test RFC"), "missing title");
    assert!(out.contains("rfc"), "missing kind");
    assert!(out.contains("draft"), "missing status");
    assert!(out.contains("human/alice"), "missing actor");
    assert!(out.contains("body:"), "missing body section");
    assert!(out.contains("Initial thread body."), "missing body content");
    assert!(
        out.contains("Second line."),
        "missing multiline body content"
    );
    assert!(out.contains("2026-01-01T00:00:00Z"), "missing timestamp");
    assert!(out.contains("timeline:"), "missing timeline section");
    assert!(out.contains("date"), "missing timeline header");
    let event_id = &state.events[0].event_id;
    assert!(
        out.contains(event_id),
        "missing create event id in timeline"
    );
    assert!(
        out.contains("create"),
        "missing create event type in timeline"
    );
}

#[test]
fn show_replay_consistency() {
    // show output reflects replayed state, not raw event data
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state1 = thread::replay_thread(&git, "RFC-0001").unwrap();
    let state2 = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(show::render_show(&state1), show::render_show(&state2));
}

#[test]
fn show_snapshot_stable() {
    // Full deterministic snapshot with FixedClock
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body."),
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    let out = show::render_show(&state);
    let event_id = &state.events[0].event_id;
    let id_width = 18usize.max(event_id.len());

    let expected = format!(
        "\
RFC-0001     Test RFC
kind:     rfc
status:   draft
created:  2026-01-01T00:00:00Z
by:       human/alice
body:
  Initial thread body.

timeline:
  {:<20}  {:<id_width$}  {:<18}  {:<10}  {}
  {:<20}  {:<id_width$}  {:<18}  {:<10}  {}
",
        "date",
        "id",
        "author",
        "type",
        "body",
        "2026-01-01T00:00:00Z",
        event_id,
        "human/alice",
        "create",
        "Test RFC",
        id_width = id_width,
    );
    assert_eq!(out, expected);
}

#[test]
fn create_thread_body_roundtrips_in_replay() {
    let (_repo, git, _paths) = setup();
    create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Problem statement and context."),
        "human/alice",
        &fixed_clock(),
        &seq_ids("e"),
    )
    .unwrap();
    let state = thread::replay_thread(&git, "RFC-0001").unwrap();
    assert_eq!(
        state.body.as_deref(),
        Some("Problem statement and context.")
    );
}
