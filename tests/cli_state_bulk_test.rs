mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

fn git(repo: &support::repo::TestRepo, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo.path())
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_real_branch(repo: &support::repo::TestRepo, branch: &str) {
    git(repo, &["commit", "--allow-empty", "-m", "init"]);
    git(repo, &["branch", branch]);
}

#[test]
fn state_bulk_partial_apply_reports_failures() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(&repo, "v0.1.0");

    let issue_a = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Setup CI", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue A");
    assert!(issue_a.status.success());

    let issue_b = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Build engine", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue B");
    assert!(issue_b.status.success());

    let action = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["action", "ASK-0002", "Implement evaluator"])
        .output()
        .expect("failed to add action");
    assert!(action.status.success());

    let bulk = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["state", "bulk", "--to", "closed", "--branch", "v0.1.0"])
        .output()
        .expect("failed to run state bulk");
    assert!(!bulk.status.success());
    let stdout = String::from_utf8(bulk.stdout).unwrap();
    assert!(stdout.contains("OK"));
    assert!(stdout.contains("FAIL"));
    assert!(stdout.contains("ASK-0001"));
    assert!(stdout.contains("ASK-0002"));
    assert!(stdout.contains("no_open_actions"));

    let git = GitOps::new(repo.path().to_path_buf());
    let issue_a = thread::replay_thread(&git, "ASK-0001").unwrap();
    let issue_b = thread::replay_thread(&git, "ASK-0002").unwrap();
    assert_eq!(issue_a.status, "closed");
    assert_eq!(issue_b.status, "open");
}

#[test]
fn state_bulk_can_resolve_open_actions_before_close() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(&repo, "v0.1.0");

    let issue = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Build engine", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue");
    assert!(issue.status.success());

    let action = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["action", "ASK-0001", "Implement evaluator"])
        .output()
        .expect("failed to add action");
    assert!(action.status.success());

    let bulk = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "state",
            "bulk",
            "--to",
            "closed",
            "--branch",
            "v0.1.0",
            "--resolve-open-actions",
        ])
        .output()
        .expect("failed to run state bulk");
    assert!(bulk.status.success());
    let stdout = String::from_utf8(bulk.stdout).unwrap();
    assert!(stdout.contains("OK"));
    assert!(stdout.contains("ASK-0001"));

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.status, "closed");
    assert_eq!(state.open_actions().len(), 0);
}
