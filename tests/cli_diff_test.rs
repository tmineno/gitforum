//! `git forum diff` CLI tests.
//!
//! task `913c4s9v`: switched from v2
//! `create::create_thread` + `write_ops::revise_body` setup to direct
//! snapshot writes via `internal::snapshot::write_snapshot`. The new
//! diff implementation derives revisions from snapshot commits whose
//! tree changed `body.md` (SPEC-3.0 §5.4), so the test setup must
//! produce real snapshot history.

mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::snapshot::{write_snapshot, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;

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

fn snapshot(id: &str) -> ThreadSnapshot {
    ThreadSnapshot {
        schema_version: 3,
        id: id.into(),
        title: "Test issue".into(),
        category: "issue".into(),
        status: "open".into(),
        tags: vec![],
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        created_by: "human/alice".into(),
        updated_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
    }
}

/// Create a thread snapshot with the given body. The first call creates
/// the ref; subsequent calls write an updated snapshot to record a body
/// revision (each call produces one commit that touches `body.md`).
fn write_revision(git: &GitOps, id: &str, body: Option<&str>, message: &str) {
    let doc = ThreadDocument {
        body: body.map(|s| s.to_string()),
        ..ThreadDocument::new(snapshot(id))
    };
    write_snapshot(git, id, &doc, message).unwrap();
}

#[test]
fn diff_no_revisions_shows_message() {
    let (repo, git, _paths) = setup();
    let id = "iss00001";
    write_revision(&git, id, Some("original body"), "create");

    let output = run_diff(&repo, &[id]);
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
    let id = "iss00002";
    write_revision(&git, id, Some("line one\nline two\n"), "create");
    write_revision(&git, id, Some("line one\nline two\nline three\n"), "revise");

    let output = run_diff(&repo, &[id]);
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
    let id = "iss00003";
    write_revision(&git, id, Some("version zero\n"), "create");
    write_revision(&git, id, Some("version one\n"), "revise 1");
    write_revision(&git, id, Some("version two\n"), "revise 2");

    let output = run_diff(&repo, &[id, "--rev", "0..2"]);
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
    let id = "iss00004";
    write_revision(&git, id, Some("version zero\n"), "create");
    write_revision(&git, id, Some("version one\n"), "revise 1");
    write_revision(&git, id, Some("version two\n"), "revise 2");

    // --rev 1 means diff rev 0 vs 1
    let output = run_diff(&repo, &[id, "--rev", "1"]);
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
    let id = "iss00005";
    // Create with no body — snapshot tree omits body.md entirely
    // (SPEC-3.0 §4.2: optional files MAY be absent when empty).
    write_revision(&git, id, None, "create empty");
    write_revision(&git, id, Some("first body content\n"), "add body");

    let output = run_diff(&repo, &[id, "--rev", "0..1"]);
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
    let id = "iss00006";
    write_revision(&git, id, Some("old\n"), "create");
    write_revision(&git, id, Some("new\n"), "revise");

    let output = run_diff(&repo, &[id]);
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
    let id = "iss00007";
    write_revision(&git, id, Some("body\n"), "create");
    write_revision(&git, id, Some("new body\n"), "revise");

    // Out of range
    let output = run_diff(&repo, &[id, "--rev", "5"]);
    assert!(!output.status.success(), "should fail for out-of-range rev");

    // Same rev
    let output = run_diff(&repo, &[id, "--rev", "1..1"]);
    assert!(!output.status.success(), "should fail for same rev");
}
