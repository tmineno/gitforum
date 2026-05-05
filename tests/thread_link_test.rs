//! Module integration tests for the thread-link surface in
//! `src/internal/evidence.rs` — `add_thread_link` and how links
//! surface in `show` and `node show` output (test-policy.md
//! category 1). The evidence-side of the same module lives in
//! `evidence_test.rs`.

mod support;

use git_forum::internal::commands::show;
use git_forum::internal::create;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::legacy::event::{NodeType, ThreadKind};
use git_forum::internal::thread;
use git_forum::internal::write_ops;

use support::forum::{
    fixed_clock, link_thread, make_rfc as make_thread, setup_no_init as setup_with_paths,
};

fn setup() -> (support::repo::TestRepo, GitOps) {
    let (repo, git, _paths) = setup_with_paths();
    (repo, git)
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

    link_thread(&git, &thread_id, &target_id, "implements");

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

    link_thread(&git, &thread_id, &target_id, "implements");

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());
    assert!(out.contains("links: 1"));
    assert!(out.contains(&target_id));
    assert!(out.contains("implements"));
    // (Phase 4 Step 1a: dropped `out.contains(format!("{target_id} (implements)"))`
    // — that matched the v2 timeline's `Link` event detail format. The
    // links section itself renders as `<target>  <rel>` (two spaces),
    // covered by the assertions above.)
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

    link_thread(&git, &thread_id, &target_id, "implements");

    let lookup = thread::find_node(&git, &node_id).unwrap();
    let out = show::render_node_show(&lookup, &show::ShowOptions::default());
    assert!(out.contains("### thread links (1)"));
    assert!(out.contains(&target_id));
    assert!(out.contains("implements"));
}
