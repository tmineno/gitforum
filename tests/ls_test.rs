//! Module integration tests for `src/internal/ls.rs` rendering
//! (test-policy.md category 1). Kind-filter tests for DEC/TASK land
//! here in the m6 split.

mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::ls;
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
fn ls_shows_all_kinds() {
    let (_repo, git, _paths) = setup();
    let ask_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let rfc_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let all_ids = thread::list_thread_ids(&git).unwrap();
    let mut states = Vec::new();
    for id in &all_ids {
        states.push(thread::replay_thread(&git, id).unwrap());
    }
    let refs: Vec<&thread::ThreadState> = states.iter().collect();
    let out = ls::render_ls(&refs);
    assert!(out.contains(&ask_id));
    assert!(out.contains(&rfc_id));
    assert!(out.contains("Bug"));
    assert!(out.contains("Proposal"));
}

#[test]
fn ls_filtered_by_kind() {
    let (_repo, git, _paths) = setup();
    let _ask_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Bug",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();
    let rfc_id = create::create_thread(
        &git,
        ThreadKind::Rfc,
        "Proposal",
        None,
        "human/alice",
        &fixed_clock(),
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
    let out = ls::render_ls(&refs);
    assert!(out.contains(&rfc_id));
    assert!(out.contains("Proposal"));
}
