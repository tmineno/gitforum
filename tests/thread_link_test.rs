//! Module integration tests for the thread-link surface in
//! `src/internal/evidence.rs` — `add_thread_link` and how links
//! surface in `show` and `node show` output (test-policy.md
//! category 1). The evidence-side of the same module lives in
//! `evidence_test.rs`.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::show;
use git_forum::internal::thread;
use git_forum::internal::write_ops;

fn setup() -> (support::repo::TestRepo, GitOps) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    (repo, git)
}

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn make_thread(git: &GitOps) -> String {
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

#[test]
fn add_thread_link_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let target_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Target issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    evidence::add_thread_link(
        &git,
        &thread_id,
        &target_id,
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, target_id);
    assert_eq!(state.links[0].rel, "implements");
}

#[test]
fn show_includes_links_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let target_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Target issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    evidence::add_thread_link(
        &git,
        &thread_id,
        &target_id,
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());
    assert!(out.contains("links: 1"));
    assert!(out.contains(&target_id));
    assert!(out.contains("implements"));
    assert!(out.contains(&format!("{target_id} (implements)")));
}

#[test]
fn node_show_includes_parent_thread_links() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let node_id = write_ops::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "How is this tracked?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    let target_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Target RFC",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    evidence::add_thread_link(
        &git,
        &thread_id,
        &target_id,
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let lookup = thread::find_node(&git, &node_id).unwrap();
    let out = show::render_node_show(&lookup, &show::ShowOptions::default());
    assert!(out.contains("### thread links (1)"));
    assert!(out.contains(&target_id));
    assert!(out.contains("implements"));
}
