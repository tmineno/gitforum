mod support;

use std::process::Command;

use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

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

#[test]
fn revise_default_body_shorthand() {
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
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("original body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

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
    let thread_id = create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("original body"),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap();

    // `git forum issue revise ISSUE-0001 --body "..."` should default to body
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "revise", &thread_id, "--body", "via issue cmd"])
        .output()
        .expect("failed to run git-forum issue revise");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "issue revise shorthand failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Body revised"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.body.as_deref(), Some("via issue cmd"));
}
