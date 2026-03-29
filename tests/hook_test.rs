mod support;

use std::fs;
use std::process::Command;

use git_forum::internal::config::RepoPaths;
use git_forum::internal::init;

fn git_forum_cmd(repo_path: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_git-forum"));
    cmd.current_dir(repo_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

fn init_repo(repo: &support::repo::TestRepo) {
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
}

fn create_issue(repo: &support::repo::TestRepo, title: &str) {
    let output = git_forum_cmd(repo.path())
        .args(["issue", "new", title])
        .output()
        .expect("failed to create issue");
    assert!(
        output.status.success(),
        "issue creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_msg_file(repo: &support::repo::TestRepo, content: &str) -> std::path::PathBuf {
    let msg_path = repo.path().join("COMMIT_MSG_TEST");
    fs::write(&msg_path, content).unwrap();
    msg_path
}

// --- check-commit-msg tests ---

#[test]
fn check_commit_msg_no_refs_warns_and_exits_0() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let msg_path = write_msg_file(&repo, "fix typo in README");

    let output = git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no thread ID referenced"));
}

#[test]
fn check_commit_msg_valid_ref_exits_0() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    create_issue(&repo, "Test issue");
    let msg_path = write_msg_file(&repo, "fix ASK-0001 bug");

    let output = git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("non-existent"));
}

#[test]
fn check_commit_msg_missing_ref_exits_1() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let msg_path = write_msg_file(&repo, "fix ASK-9999 bug");

    let output = git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non-existent thread"));
    assert!(stderr.contains("ASK-9999"));
}

#[test]
fn check_commit_msg_mixed_refs() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    create_issue(&repo, "Real issue");
    let msg_path = write_msg_file(&repo, "fix ASK-0001 and ASK-9999");

    let output = git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ASK-9999"));
    assert!(!stderr.contains("ASK-0001"));
}

#[test]
fn check_commit_msg_strips_comments() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let msg_path = write_msg_file(&repo, "fix typo\n# ASK-9999 is in a comment");

    let output = git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg");

    // ISSUE-9999 is in a comment line, so it should be stripped.
    // No thread IDs remain, so we get the "no thread ID" warning + exit 0.
    assert!(output.status.success());
}

// --- hook install/uninstall tests ---

#[test]
fn hook_install_creates_executable_file() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    let output = git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .expect("failed to run hook install");

    assert!(output.status.success());

    let hook_path = repo.path().join(".git/hooks/commit-msg");
    assert!(hook_path.exists());

    let content = fs::read_to_string(&hook_path).unwrap();
    assert!(content.contains("git-forum"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&hook_path).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0, "hook should be executable");
    }
}

#[test]
fn hook_install_idempotent() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    // First install
    let output = git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Second install (no --force)
    let output = git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already installed"));
}

#[test]
fn hook_install_refuses_foreign_hook_without_force() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    // Write a foreign hook
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("commit-msg"), "#!/bin/sh\necho foreign\n").unwrap();

    let output = git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--force"));
}

#[test]
fn hook_install_force_overwrites() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    // Write a foreign hook
    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("commit-msg"), "#!/bin/sh\necho foreign\n").unwrap();

    let output = git_forum_cmd(repo.path())
        .args(["hook", "install", "--force"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let content = fs::read_to_string(hooks_dir.join("commit-msg")).unwrap();
    assert!(content.contains("git-forum"));
}

#[test]
fn hook_uninstall_removes_hook() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    // Install first
    git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .unwrap();

    let hook_path = repo.path().join(".git/hooks/commit-msg");
    assert!(hook_path.exists());

    // Uninstall
    let output = git_forum_cmd(repo.path())
        .args(["hook", "uninstall"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!hook_path.exists());
}

#[test]
fn hook_uninstall_refuses_foreign_hook() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    let hooks_dir = repo.path().join(".git/hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("commit-msg"), "#!/bin/sh\necho foreign\n").unwrap();

    let output = git_forum_cmd(repo.path())
        .args(["hook", "uninstall"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not installed by git-forum"));
}

#[test]
fn init_also_installs_hook() {
    let repo = support::repo::TestRepo::new();

    let output = git_forum_cmd(repo.path())
        .arg("init")
        .output()
        .expect("failed to run init");
    assert!(output.status.success());

    let hook_path = repo.path().join(".git/hooks/commit-msg");
    assert!(
        hook_path.exists(),
        "init should install the commit-msg hook"
    );
}
