mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

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
fn thread_new_can_bind_branch_scope() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(&repo, "feat/parser-rewrite");

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "issue",
            "new",
            "Parser fails",
            "--branch",
            "feat/parser-rewrite",
        ])
        .output()
        .expect("failed to create issue");
    assert!(output.status.success());
    let thread_id = extract_created_id(&output);

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.branch.as_deref(), Some("feat/parser-rewrite"));
}

#[test]
fn branch_bind_and_clear_update_thread_scope() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(&repo, "feat/solver");

    let create = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Implement solver"])
        .output()
        .expect("failed to create issue");
    assert!(create.status.success());
    let thread_id = extract_created_id(&create);

    let bind = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["branch", "bind", &thread_id, "feat/solver"])
        .output()
        .expect("failed to bind branch");
    assert!(bind.status.success());

    let show = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["show", &thread_id])
        .output()
        .expect("failed to show issue");
    assert!(show.status.success());
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("branch:   feat/solver"));

    let clear = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["branch", "clear", &thread_id])
        .output()
        .expect("failed to clear branch");
    assert!(clear.status.success());

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.branch, None);
}

#[test]
fn issue_ls_can_filter_by_branch() {
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
    let id_a = extract_created_id(&issue_a);

    let issue_b = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "new", "Refactor parser"])
        .output()
        .expect("failed to create issue B");
    assert!(issue_b.status.success());
    let id_b = extract_created_id(&issue_b);

    let ls = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["issue", "ls", "--branch", "v0.1.0"])
        .output()
        .expect("failed to list issues by branch");
    assert!(ls.status.success());
    let stdout = String::from_utf8(ls.stdout).unwrap();
    assert!(stdout.contains("BRANCH"));
    assert!(stdout.contains(&id_a));
    assert!(stdout.contains("v0.1.0"));
    assert!(!stdout.contains(&id_b));
}
