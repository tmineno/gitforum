mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

fn extract_created_id(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .strip_prefix("Created ")
        .unwrap_or(stdout.trim())
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// Snapshot fixture: invoke `git forum new` to write a fresh SPEC-3.0
/// thread. Replaces the legacy `create::create_thread` fixture path
/// now that ADR-011 Decision 3 forbids non-migrate code paths from
/// consuming legacy event chains.
fn make_thread_via_cli(repo_path: &std::path::Path, kind: &str, title: &str, body: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo_path)
        .args(["new", kind, title, "--body", body])
        .output()
        .expect("failed to run");
    assert!(
        output.status.success(),
        "make_thread_via_cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    extract_created_id(&output)
}

#[test]
fn revise_default_body_shorthand() {
    let (repo, git, _paths) = setup();
    let thread_id = make_thread_via_cli(repo.path(), "issue", "Test issue", "original body");

    // `git forum revise ISSUE-0001 --body "updated body"` should work as body revision
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["revise", &thread_id, "--body", "updated body"])
        .output()
        .expect("failed to run git-forum revise");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "revise shorthand failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Body revised"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.body.as_deref(), Some("updated body"));
}

#[test]
fn revise_body_explicit_still_works() {
    let (repo, git, _paths) = setup();
    let thread_id = make_thread_via_cli(repo.path(), "issue", "Test issue", "original body");

    // `git forum revise body ISSUE-0001 --body "updated"` should still work
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["revise", "body", &thread_id, "--body", "explicit body"])
        .output()
        .expect("failed to run git-forum revise body");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "revise body explicit failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Body revised"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.body.as_deref(), Some("explicit body"));
}

#[test]
fn revise_via_issue_subcommand_shorthand() {
    let (repo, git, _paths) = setup();
    let thread_id = make_thread_via_cli(repo.path(), "issue", "Test issue", "original body");

    // `git forum revise <THREAD> --body "..."` is the 2.0 top-level form
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["revise", &thread_id, "--body", "via top-level revise"])
        .output()
        .expect("failed to run git-forum revise");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "top-level revise failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Body revised"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.body.as_deref(), Some("via top-level revise"));
}
