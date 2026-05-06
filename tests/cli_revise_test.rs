mod support;

use std::process::Command;

use git_forum::internal::thread;

use support::cli::make_thread_via_cli;
use support::forum::setup;

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
