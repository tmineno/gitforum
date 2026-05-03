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
fn state_bulk_partial_apply_reports_failures() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    create_real_branch(&repo, "v0.1.0");

    let issue_a = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Setup CI", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue A");
    assert!(issue_a.status.success());
    let id_a = extract_created_id(&issue_a);

    let issue_b = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Build engine", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue B");
    assert!(issue_b.status.success());
    let id_b = extract_created_id(&issue_b);

    let action = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["action", &id_b, "Implement evaluator"])
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
    assert!(stdout.contains(&id_a));
    assert!(stdout.contains(&id_b));
    assert!(stdout.contains("no_open_actions"));

    let git = GitOps::new(repo.path().to_path_buf());
    let issue_a = thread::replay_thread(&git, &id_a).unwrap();
    let issue_b = thread::replay_thread(&git, &id_b).unwrap();
    assert_eq!(issue_a.status, "done");
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
        .args(["new", "issue", "Build engine", "--branch", "v0.1.0"])
        .output()
        .expect("failed to create issue");
    assert!(issue.status.success());
    let thread_id = extract_created_id(&issue);

    let action = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["action", &thread_id, "Implement evaluator"])
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
    assert!(stdout.contains(&thread_id));

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "done");
    assert_eq!(state.open_actions().len(), 0);
}

#[test]
fn state_self_loop_returns_zero_with_note() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let issue = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Self-loop CLI"])
        .output()
        .expect("failed to create issue");
    assert!(issue.status.success());
    let thread_id = extract_created_id(&issue);

    let git = GitOps::new(repo.path().to_path_buf());
    let events_before = thread::replay_thread(&git, &thread_id)
        .unwrap()
        .events
        .len();

    let out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["state", &thread_id, "open"])
        .output()
        .expect("failed to run state self-loop");
    assert!(
        out.status.success(),
        "self-loop must exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("already in state 'open'"),
        "stdout missing note: {stdout}"
    );
    assert!(
        stdout.contains("no transition recorded"),
        "stdout missing 'no transition recorded': {stdout}"
    );

    let events_after = thread::replay_thread(&git, &thread_id)
        .unwrap()
        .events
        .len();
    assert_eq!(
        events_after, events_before,
        "self-loop must not write any new events"
    );
}

#[test]
fn state_self_loop_with_comment_records_say_node() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let issue = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Self-loop with comment"])
        .output()
        .expect("failed to create issue");
    assert!(issue.status.success());
    let thread_id = extract_created_id(&issue);

    let out = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "state",
            &thread_id,
            "open",
            "--comment",
            "still investigating; leaving open",
        ])
        .output()
        .expect("failed to run state self-loop with comment");
    assert!(
        out.status.success(),
        "self-loop with comment must exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("already in state 'open'"));
    assert!(stdout.contains("comment attached as a standalone node"));

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
    // SPEC-3.0 §2.2 (Phase 2 slot 3): comments are nodes in
    // `nodes/<id>.{toml,md}`. The pre-Phase-2 assertion was on
    // `state.events` (v2 event-chain shape); the v3 surface is
    // `state.nodes`.
    let comment_nodes: Vec<_> = state
        .nodes
        .iter()
        .filter(|n| n.body == "still investigating; leaving open")
        .collect();
    assert_eq!(comment_nodes.len(), 1);
}
