mod support;

use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

use support::cli::extract_created_id;
use support::git::create_real_branch;

#[test]
fn thread_new_can_bind_branch_scope() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(repo.path(), "feat/parser-rewrite");

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "new",
            "issue",
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
    create_real_branch(repo.path(), "feat/solver");

    let create = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Implement solver"])
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
    // SPEC-2.0 classification: header labels gained 1 char of padding to align with `**lifecycle:**`.
    assert!(stdout.contains("**branch:**    feat/solver"));

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
    create_real_branch(repo.path(), "v0.1.0");

    let issue_a = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Setup CI", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue A");
    assert!(issue_a.status.success());
    let id_a = extract_created_id(&issue_a);

    let issue_b = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Refactor parser"])
        .output()
        .expect("failed to create issue B");
    assert!(issue_b.status.success());
    let id_b = extract_created_id(&issue_b);

    let ls = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["ls", "--kind", "issue", "--branch", "v0.1.0"])
        .output()
        .expect("failed to list issues by branch");
    assert!(ls.status.success());
    let stdout = String::from_utf8(ls.stdout).unwrap();
    assert!(stdout.contains("BRANCH"));
    assert!(stdout.contains(&id_a));
    assert!(stdout.contains("v0.1.0"));
    assert!(!stdout.contains(&id_b));
}
