//! Module integration tests for `src/internal/show.rs` rendering
//! (test-policy.md category 1). show-with-nodes, DEC/TASK kind
//! display, and Track G's `--tree` advisory tests land here in later
//! splits.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::show;
use git_forum::internal::thread;
use git_forum::internal::write_ops;

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

fn make_rfc(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Rfc,
        "Test RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn make_dec(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Dec,
        "Test DEC",
        Some(
            "## Context\nSome context\n## Decision\nUse Redis\n## Rationale\nFast\n## Impact\nNone",
        ),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

fn make_task(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Task,
        "Test TASK",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
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

// ---- show with nodes ----

#[test]
fn show_includes_open_objections_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "Concern about performance.",
        "human/bob",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains("**open objections:** 1"));
    assert!(out.contains("Concern about performance."));
}

#[test]
fn show_includes_latest_summary_section() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Summary,
        "This is the consensus.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains("latest summary:"));
    assert!(out.contains("This is the consensus."));
}

#[test]
fn show_no_extra_sections_when_no_nodes() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(!out.contains("open objections:"));
    assert!(!out.contains("open actions:"));
    assert!(!out.contains("latest summary:"));
}

#[test]
fn show_timeline_includes_say_events() {
    let (_repo, git, _paths) = setup();
    let thread_id = make_rfc(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "This is important.",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());

    assert!(out.contains(&node_id[..node_id.len().min(16)]));
    // SPEC-2.0 §2.5 / §9.3: legacy `claim` writes are canonicalized to
    // `comment`. Authors who want to preserve a rhetorical distinction
    // should encode it in the body (e.g. "Claim:" prefix).
    assert!(out.contains("comment"));
    assert!(out.contains("This is important."));
}

// ---- DEC / TASK rendering ----

#[test]
fn show_dec_includes_kind() {
    let (_repo, git, _paths) = setup();
    let id = make_dec(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**kind:**     dec"));
    assert!(output.contains("**status:**   open"));
}

#[test]
fn show_task_includes_kind() {
    let (_repo, git, _paths) = setup();
    let id = make_task(&git);
    let state = thread::replay_thread(&git, &id).unwrap();
    let output = show::render_show(&state, &show::ShowOptions::default());
    assert!(output.contains("**kind:**     task"));
    assert!(output.contains("**status:**   open"));
}
