mod support;

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::github_export;
use git_forum::internal::github_import;
use git_forum::internal::init;
use git_forum::internal::say;

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

// ── Import dedup ────────────────────────────────────────────────────────

#[test]
fn find_existing_import_no_match() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    let result =
        github_import::find_existing_import(&git, "https://github.com/owner/repo/issues/1")
            .unwrap();
    assert!(result.is_none());
}

#[test]
fn find_existing_import_with_matching_evidence() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::External,
        "https://github.com/owner/repo/issues/42",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let result =
        github_import::find_existing_import(&git, "https://github.com/owner/repo/issues/42")
            .unwrap();
    assert_eq!(result, Some(thread_id));
}

#[test]
fn find_existing_import_no_match_different_url() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::External,
        "https://github.com/owner/repo/issues/42",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let result =
        github_import::find_existing_import(&git, "https://github.com/owner/repo/issues/99")
            .unwrap();
    assert!(result.is_none());
}

// ── Export dedup ─────────────────────────────────────────────────────────

#[test]
fn find_existing_export_no_evidence() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    let state = git_forum::internal::thread::replay_thread(&git, &thread_id).unwrap();
    assert!(github_export::find_existing_export(&state).is_none());
}

#[test]
fn find_existing_export_with_github_evidence() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::External,
        "https://github.com/owner/repo/issues/10",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let state = git_forum::internal::thread::replay_thread(&git, &thread_id).unwrap();
    let result = github_export::find_existing_export(&state);
    assert_eq!(
        result,
        Some("https://github.com/owner/repo/issues/10".to_string())
    );
}

#[test]
fn find_existing_export_ignores_non_github_evidence() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    evidence_ops::add_evidence(
        &git,
        &thread_id,
        EvidenceKind::External,
        "https://example.com/something",
        None,
        "human/alice",
        &clock,
    )
    .unwrap();
    let state = git_forum::internal::thread::replay_thread(&git, &thread_id).unwrap();
    assert!(github_export::find_existing_export(&state).is_none());
}

// ── Export formatting ───────────────────────────────────────────────────

#[test]
fn format_node_comment_includes_marker_and_type() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    say::say_node(
        &git,
        &thread_id,
        NodeType::Objection,
        "This is broken",
        "human/bob",
        &clock,
        None,
    )
    .unwrap();
    let state = git_forum::internal::thread::replay_thread(&git, &thread_id).unwrap();
    let node = &state.nodes[0];
    let comment = github_export::format_node_as_comment(node);
    assert!(comment.contains("<!-- git-forum:"));
    assert!(comment.contains("**[objection]** by human/bob"));
    assert!(comment.contains("This is broken"));
}

#[test]
fn extract_marker_from_formatted_comment() {
    let (_repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id =
        create::create_thread(&git, ThreadKind::Issue, "Bug", None, "human/alice", &clock).unwrap();
    let node_id = say::say_node(
        &git,
        &thread_id,
        NodeType::Claim,
        "test",
        "human/alice",
        &clock,
        None,
    )
    .unwrap();
    let state = git_forum::internal::thread::replay_thread(&git, &thread_id).unwrap();
    let node = &state.nodes[0];
    let comment = github_export::format_node_as_comment(node);
    let extracted = github_export::extract_marker(&comment);
    assert_eq!(extracted, Some(node_id.as_str()));
}

// ── URL parsing ─────────────────────────────────────────────────────────

#[test]
fn parse_github_issue_url() {
    let result = github_export::parse_github_issue_url("https://github.com/foo/bar/issues/123");
    assert_eq!(result, Some(("foo/bar".to_string(), 123)));
}

#[test]
fn parse_github_issue_url_invalid_path() {
    assert!(github_export::parse_github_issue_url("https://github.com/foo/bar/pulls/1").is_none());
}

#[test]
fn parse_github_issue_url_not_github() {
    assert!(github_export::parse_github_issue_url("https://gitlab.com/a/b/issues/1").is_none());
}
