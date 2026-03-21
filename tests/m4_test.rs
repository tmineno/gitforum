mod support;

use std::fs;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::say;
use git_forum::internal::show;
use git_forum::internal::thread;

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

fn make_real_commit(git: &GitOps) -> String {
    let path = git.root().join("fixture.txt");
    fs::write(&path, "fixture\n").unwrap();
    git.run(&["add", "fixture.txt"]).unwrap();
    git.run(&["commit", "-m", "fixture commit"]).unwrap();
    git.run(&["rev-parse", "--verify", "HEAD"]).unwrap()
}

// ---- Evidence tests ----

#[test]
fn add_evidence_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_evidence(
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

    let commit_sha = evidence_ops::add_evidence(
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

    let err = evidence_ops::add_evidence(
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

    evidence_ops::add_evidence(
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
    let out = show::render_show(&state, false);
    assert!(out.contains("evidence: 1"));
    assert!(out.contains("benchmark"));
    assert!(out.contains("bench/result.csv"));
    assert!(out.contains("link"));
    assert!(out.contains("benchmark bench/result.csv"));
}

// ---- Thread link tests ----

#[test]
fn add_thread_link_appears_in_thread_state() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_thread_link(
        &git,
        &thread_id,
        "ISSUE-0001",
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, "ISSUE-0001");
    assert_eq!(state.links[0].rel, "implements");
}

#[test]
fn show_includes_links_section() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    evidence_ops::add_thread_link(
        &git,
        &thread_id,
        "ISSUE-0001",
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    let out = show::render_show(&state, false);
    assert!(out.contains("links: 1"));
    assert!(out.contains("ISSUE-0001"));
    assert!(out.contains("implements"));
    assert!(out.contains("ISSUE-0001 (implements)"));
}

#[test]
fn node_show_includes_parent_thread_links() {
    let (_repo, git) = setup();
    let thread_id = make_thread(&git);

    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Question,
        "How is this tracked?",
        "human/alice",
        &fixed_clock(),
        None,
    )
    .unwrap();

    evidence_ops::add_thread_link(
        &git,
        &thread_id,
        "RFC-0001",
        "implements",
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let lookup = thread::find_node(&git, &node_id).unwrap();
    let out = show::render_node_show(&lookup);
    assert!(out.contains("thread links: 1"));
    assert!(out.contains("RFC-0001"));
    assert!(out.contains("implements"));
}
