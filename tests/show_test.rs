//! Module integration tests for `src/internal/show.rs` rendering
//! (test-policy.md category 1). show-with-nodes, DEC/TASK kind
//! display, and Track G's `--tree` advisory tests land here in later
//! splits.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
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

#[test]
fn show_contains_all_required_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body.\nSecond line."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&id), "missing thread id");
    assert!(out.contains("Test RFC"), "missing title");
    assert!(out.contains("rfc"), "missing kind");
    assert!(out.contains("draft"), "missing status");
    assert!(out.contains("human/alice"), "missing actor");
    assert!(out.contains("---"), "missing body separator");
    assert!(out.contains("Initial thread body."), "missing body content");
    assert!(
        out.contains("Second line."),
        "missing multiline body content"
    );
    assert!(out.contains("2026-01-01T00:00:00Z"), "missing timestamp");
    assert!(out.contains("### timeline"), "missing timeline section");
    assert!(out.contains("date"), "missing timeline header");
    assert!(out.contains("node_id"), "missing node_id timeline header");
    assert!(out.contains("event_id"), "missing event_id timeline header");
    let event_id = &state.events[0].event_id;
    assert!(
        out.contains(&event_id[..event_id.len().min(16)]),
        "missing create event id in timeline"
    );
    assert!(
        out.contains("create"),
        "missing create event type in timeline"
    );
}

#[test]
fn show_replay_consistency() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state1 = thread::replay_thread(&git, &id).unwrap();
    let state2 = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(
        show::render_show(&state1, &show::ShowOptions::default()),
        show::render_show(&state2, &show::ShowOptions::default())
    );
}

#[test]
fn show_snapshot_contains_expected_fields() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Test RFC",
        Some("Initial thread body."),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&id));
    assert!(out.contains("Test RFC"));
    assert!(out.contains("rfc"));
    assert!(out.contains("draft"));
    assert!(out.contains("Initial thread body."));
    assert!(out.contains("human/alice"));
    assert!(out.contains("2026-01-01T00:00:00Z"));
    assert!(out.contains("create"));
}
