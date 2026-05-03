//! Module integration tests for `src/internal/ls.rs` rendering
//! (test-policy.md category 1). Kind-filter tests for DEC/TASK land
//! here in the m6 split.

mod support;

use git_forum::internal::commands::ls;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::thread;

use support::forum::{fixed_clock, make_dec, make_task, setup};

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

// ---- Kind filter (DEC / TASK) ----

#[test]
fn ls_filters_by_dec_kind() {
    let (_repo, git, _paths) = setup();
    make_dec(&git);
    make_task(&git);
    let ids = thread::list_thread_ids(&git).unwrap();
    let all: Vec<_> = ids
        .iter()
        .map(|id| thread::replay_thread(&git, id).unwrap())
        .collect();
    let decs: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Dec).collect();
    let tasks: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Task).collect();
    assert_eq!(decs.len(), 1);
    assert_eq!(tasks.len(), 1);
}

#[test]
fn ls_filters_by_task_kind() {
    let (_repo, git, _paths) = setup();
    make_task(&git);
    make_task(&git);
    make_dec(&git);
    let ids = thread::list_thread_ids(&git).unwrap();
    let all: Vec<_> = ids
        .iter()
        .map(|id| thread::replay_thread(&git, id).unwrap())
        .collect();
    let tasks: Vec<_> = all.iter().filter(|s| s.kind == ThreadKind::Task).collect();
    assert_eq!(tasks.len(), 2);
}
