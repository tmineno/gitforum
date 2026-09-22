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

fn create_issue(repo: &support::repo::TestRepo, title: &str) -> String {
    let output = git_forum_cmd(repo.path())
        .args(["new", "issue", title])
        .output()
        .expect("failed to create issue");
    assert!(
        output.status.success(),
        "issue creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // CLI prints "Created <THREAD_ID>" — extract the ID.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .strip_prefix("Created ")
        .expect("expected 'Created <ID>' on stdout")
        .to_string()
}

fn write_msg_file(repo: &support::repo::TestRepo, content: &str) -> std::path::PathBuf {
    let msg_path = repo.path().join("COMMIT_MSG_TEST");
    fs::write(&msg_path, content).unwrap();
    msg_path
}

// --- check-commit-msg tests ---
//
// ADR-013 / SPEC-3.0 §2.5 (3): only `Refs:` trailers are read. Each test
// below pins one row of the measurement table in ADR-013.

fn check_msg(repo: &support::repo::TestRepo, content: &str) -> std::process::Output {
    let msg_path = write_msg_file(repo, content);
    git_forum_cmd(repo.path())
        .args(["hook", "check-commit-msg"])
        .arg(&msg_path)
        .output()
        .expect("failed to run check-commit-msg")
}

#[test]
fn check_commit_msg_no_refs_warns_and_exits_0() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let output = check_msg(&repo, "fix typo in README");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no thread ID referenced"));
    assert!(stderr.contains("Refs: <thread-id>"), "stderr: {stderr}");
    assert!(
        stderr.contains("git forum evidence add"),
        "stderr: {stderr}"
    );
}

#[test]
fn check_commit_msg_bare_trailer_exits_0_without_warning() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let tid = create_issue(&repo, "Test issue");
    let output = check_msg(&repo, &format!("fix bug\n\nRefs: {tid}"));

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn check_commit_msg_at_trailer_exits_0_without_warning() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let tid = create_issue(&repo, "Test issue");
    let output = check_msg(&repo, &format!("fix bug\n\nRefs: @{tid}"));

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn check_commit_msg_subject_tag_warns_with_suggestion() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let tid = create_issue(&repo, "Test issue");
    for subject in [format!("docs: foo [{tid}]"), format!("docs: foo [@{tid}]")] {
        let output = check_msg(&repo, &subject);
        assert!(output.status.success(), "{subject}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("no thread ID referenced"),
            "{subject}: {stderr}"
        );
        assert!(
            stderr.contains(&format!("Refs: {tid}\n")),
            "{subject}: {stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "git forum evidence add {tid} --kind commit --ref HEAD"
            )),
            "{subject}: {stderr}"
        );
    }
}

#[test]
fn check_commit_msg_missing_trailer_ref_exits_1() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    for msg in ["fix bug\n\nRefs: zzzzzzzz", "fix bug\n\nRefs: @zzzzzzzz"] {
        let output = check_msg(&repo, msg);
        assert!(!output.status.success(), "{msg}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("non-existent thread"), "{msg}: {stderr}");
        assert!(stderr.contains("zzzzzzzz"), "{msg}: {stderr}");
    }
}

#[test]
fn check_commit_msg_mixed_refs() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let tid = create_issue(&repo, "Real issue");
    let output = check_msg(&repo, &format!("fix bug\n\nRefs: {tid}, ASK-9999"));

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ASK-9999"));
    assert!(!stderr.contains(&tid));
}

#[test]
fn check_commit_msg_ignores_ids_outside_trailer() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    // An 8-letter mention and a legacy-looking ID in prose used to refuse
    // the commit; neither is a reference now.
    let output = check_msg(
        &repo,
        "fix typo\n\nThanks @reviewer for the catch; see ASK-9999.",
    );

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no thread ID referenced"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("non-existent"), "stderr: {stderr}");
}

#[test]
fn check_commit_msg_ignores_foreign_trailer_values() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let output = check_msg(&repo, "fix bug\n\nRefs: #123");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no thread ID referenced"),
        "stderr: {stderr}"
    );
}

#[test]
fn check_commit_msg_accepts_published_only_thread() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let tid = create_issue(&repo, "Public issue");
    // Simulate a public-only clone: the thread exists only under
    // refs/forum/published/.
    let thread_ref = format!("refs/forum/threads/{tid}");
    let published_ref = format!("refs/forum/published/{tid}");
    support::git::git(repo.path(), &["update-ref", &published_ref, &thread_ref]);
    support::git::git(repo.path(), &["update-ref", "-d", &thread_ref]);

    let output = check_msg(&repo, &format!("fix bug\n\nRefs: {tid}"));

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.is_empty(), "stderr: {stderr}");
}

#[test]
fn check_commit_msg_strips_comments() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);
    let output = check_msg(&repo, "fix typo\n\n# Refs: ASK-9999 is in a comment");

    // The trailer is in a comment line, so it is stripped before parsing.
    // No thread IDs remain, so we get the "no thread ID" warning + exit 0.
    assert!(output.status.success());
}

// --- hook install/uninstall tests ---

#[test]
fn hook_install_creates_executable_files() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    let output = git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .expect("failed to run hook install");

    assert!(output.status.success());

    // commit-msg hook
    let commit_msg_path = repo.path().join(".git/hooks/commit-msg");
    assert!(commit_msg_path.exists());
    let content = fs::read_to_string(&commit_msg_path).unwrap();
    assert!(content.contains("git-forum"));

    // post-checkout hook
    let post_checkout_path = repo.path().join(".git/hooks/post-checkout");
    assert!(post_checkout_path.exists());
    let pc_content = fs::read_to_string(&post_checkout_path).unwrap();
    assert!(pc_content.contains("worktree-init"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&commit_msg_path).unwrap().permissions();
        assert!(
            perms.mode() & 0o111 != 0,
            "commit-msg hook should be executable"
        );
        let perms = fs::metadata(&post_checkout_path).unwrap().permissions();
        assert!(
            perms.mode() & 0o111 != 0,
            "post-checkout hook should be executable"
        );
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
fn hook_uninstall_removes_hooks() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    // Install first
    git_forum_cmd(repo.path())
        .args(["hook", "install"])
        .output()
        .unwrap();

    let commit_msg_path = repo.path().join(".git/hooks/commit-msg");
    let post_checkout_path = repo.path().join(".git/hooks/post-checkout");
    assert!(commit_msg_path.exists());
    assert!(post_checkout_path.exists());

    // Uninstall
    let output = git_forum_cmd(repo.path())
        .args(["hook", "uninstall"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!commit_msg_path.exists());
    assert!(!post_checkout_path.exists());
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
fn init_also_installs_hooks() {
    let repo = support::repo::TestRepo::new();

    let output = git_forum_cmd(repo.path())
        .arg("init")
        .output()
        .expect("failed to run init");
    assert!(output.status.success());

    let commit_msg_path = repo.path().join(".git/hooks/commit-msg");
    assert!(
        commit_msg_path.exists(),
        "init should install the commit-msg hook"
    );

    let post_checkout_path = repo.path().join(".git/hooks/post-checkout");
    assert!(
        post_checkout_path.exists(),
        "init should install the post-checkout hook"
    );
}

#[test]
fn fix_index_subcommand_succeeds_on_clean_repo() {
    let repo = support::repo::TestRepo::new();
    init_repo(&repo);

    let output = git_forum_cmd(repo.path())
        .args(["hook", "fix-index"])
        .output()
        .expect("failed to run fix-index");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("all index blobs present"));
}
