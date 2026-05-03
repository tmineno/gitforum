//! Module integration tests for the evidence surface in
//! `src/internal/evidence.rs` (test-policy.md category 1). Thread-link
//! tests, which share the same module, live in `thread_link_test.rs`
//! because the link concept is distinct enough that bundling them
//! would obscure where to add new tests.

mod support;

use std::fs;

use git_forum::internal::commands::show;
use git_forum::internal::evidence;
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::thread;

use support::forum::{fixed_clock, make_rfc as make_thread, setup_no_init as setup_with_paths};

fn setup() -> (support::repo::TestRepo, GitOps) {
    let (repo, git, _paths) = setup_with_paths();
    (repo, git)
}

fn make_real_commit(git: &GitOps) -> String {
    let path = git.root().join("fixture.txt");
    fs::write(&path, "fixture\n").unwrap();
    git.run(&["add", "fixture.txt"]).unwrap();
    git.run(&["commit", "-m", "fixture commit"]).unwrap();
    git.run(&["rev-parse", "--verify", "HEAD"]).unwrap()
}

#[test]
fn add_evidence_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Benchmark,
        "bench/result.csv",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.evidence_items.len(), 1);
    assert_eq!(state.evidence_items[0].kind, EvidenceKind::Benchmark);
    assert_eq!(state.evidence_items[0].ref_target, "bench/result.csv");
    assert!(!state.evidence_items[0].evidence_id.is_empty());
}

#[test]
fn evidence_id_is_populated_from_commit_sha() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);
    let revision = make_real_commit(&git);

    let commit_sha = evidence::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Commit,
        &revision[..12],
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.evidence_items[0].evidence_id, commit_sha);
    assert_eq!(state.evidence_items[0].ref_target, revision);
}

#[test]
fn commit_evidence_rejects_invalid_revision() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let err = evidence::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Commit,
        "not-a-real-rev",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap_err();

    assert!(err
        .to_string()
        .contains("revision 'not-a-real-rev' does not resolve to a commit"));
}

#[test]
fn show_includes_evidence_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::Benchmark,
        "bench/result.csv",
        None,
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, &show::ShowOptions::default());
    assert!(out.contains("evidence: 1"));
    assert!(out.contains("benchmark"));
    assert!(out.contains("bench/result.csv"));
    assert!(out.contains("link"));
    assert!(out.contains("benchmark bench/result.csv"));
}
