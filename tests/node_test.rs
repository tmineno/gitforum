//! Module integration tests for `src/internal/node.rs` and the
//! `say_node*` write paths in `src/internal/write_ops.rs`
//! (test-policy.md category 1). Most of the say/objection/resolve/
//! retract/find_node coverage lands here in the m3 split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
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

#[test]
fn say_node_with_timestamp_uses_override() {
    let (_repo, git, _paths) = setup();
    let id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let custom_ts = Utc.with_ymd_and_hms(2020, 3, 10, 8, 0, 0).unwrap();
    write_ops::say_node_with_timestamp(
        &git,
        &id,
        NodeType::Claim,
        "Imported comment",
        "human/bob",
        &fixed_clock(),
        None,
        custom_ts,
    )
    .unwrap();
    let state = thread::replay_thread(&git, &id).unwrap();
    assert_eq!(state.events[1].created_at, custom_ts);
    assert_eq!(state.nodes[0].created_at, custom_ts);
}
