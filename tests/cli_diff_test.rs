mod support;

use std::process::Command;

use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::say;

use chrono::{TimeZone, Utc};

fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn run_diff(repo: &support::repo::TestRepo, args: &[&str]) -> std::process::Output {
    let mut full_args = vec!["diff"];
    full_args.extend_from_slice(args);
    Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(&full_args)
        .output()
        .expect("failed to run git-forum diff")
}

#[test]
fn diff_no_revisions_shows_message() {
    let (repo, git, _paths) = setup();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("original body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    let output = run_diff(&repo, &[&thread_id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "diff should succeed");
    assert!(
        stdout.contains("no body revisions to diff"),
        "should report no revisions: {stdout}"
    );
}

#[test]
fn diff_default_shows_latest_vs_previous() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("line one\nline two\n"),
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(
        &git,
        &thread_id,
        "line one\nline two\nline three\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();

    let output = run_diff(&repo, &[&thread_id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "diff failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("+line three"),
        "should show added line: {stdout}"
    );
    assert!(
        stdout.contains("a/rev0/"),
        "should have rev0 prefix: {stdout}"
    );
    assert!(
        stdout.contains("b/rev1/"),
        "should have rev1 prefix: {stdout}"
    );
}

#[test]
fn diff_rev_range() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("version zero\n"),
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(
        &git,
        &thread_id,
        "version one\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();
    say::revise_body(
        &git,
        &thread_id,
        "version two\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();

    // Diff rev 0..2
    let output = run_diff(&repo, &[&thread_id, "--rev", "0..2"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "diff 0..2 failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("-version zero"),
        "should show removed line: {stdout}"
    );
    assert!(
        stdout.contains("+version two"),
        "should show added line: {stdout}"
    );
    assert!(
        stdout.contains("a/rev0/"),
        "should have rev0 prefix: {stdout}"
    );
    assert!(
        stdout.contains("b/rev2/"),
        "should have rev2 prefix: {stdout}"
    );
}

#[test]
fn diff_rev_single() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("version zero\n"),
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(
        &git,
        &thread_id,
        "version one\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();
    say::revise_body(
        &git,
        &thread_id,
        "version two\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();

    // --rev 1 means diff rev 0 vs 1
    let output = run_diff(&repo, &[&thread_id, "--rev", "1"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "diff --rev 1 failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("-version zero"),
        "should show removed line: {stdout}"
    );
    assert!(
        stdout.contains("+version one"),
        "should show added line: {stdout}"
    );
}

#[test]
fn diff_empty_create_body_vs_first_revision() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        None, // no body at creation
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(
        &git,
        &thread_id,
        "first body content\n",
        &[],
        "human/alice",
        &clock,
    )
    .unwrap();

    let output = run_diff(&repo, &[&thread_id, "--rev", "0..1"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "diff 0..1 failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("+first body content"),
        "should show all content as added: {stdout}"
    );
}

#[test]
fn diff_headers_show_body_not_temp_paths() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("old\n"),
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(&git, &thread_id, "new\n", &[], "human/alice", &clock).unwrap();

    let output = run_diff(&repo, &[&thread_id]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    // Should not contain /tmp/ paths
    assert!(
        !stdout.contains("/tmp/"),
        "diff output should not contain temp file paths: {stdout}"
    );
    // Should contain "body" label
    assert!(
        stdout.contains("body"),
        "diff output should contain 'body' label: {stdout}"
    );
}

#[test]
fn diff_invalid_rev_fails() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("body\n"),
        "human/alice",
        &clock,
    )
    .unwrap();

    say::revise_body(&git, &thread_id, "new body\n", &[], "human/alice", &clock).unwrap();

    // Out of range
    let output = run_diff(&repo, &[&thread_id, "--rev", "5"]);
    assert!(!output.status.success(), "should fail for out-of-range rev");

    // Same rev
    let output = run_diff(&repo, &[&thread_id, "--rev", "1..1"]);
    assert!(!output.status.success(), "should fail for same rev");
}
