mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

/// Create a mock editor script that writes known content to the file.
fn create_mock_editor(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
    let script = dir.join("mock-editor.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ncat > \"$1\" << 'MOCK_EOF'\n{content}\nMOCK_EOF\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// Create a mock editor script that does nothing (leaves template unchanged).
fn create_noop_editor(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("noop-editor.sh");
    fs::write(&script, "#!/bin/sh\ntrue\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn edit_flag_creates_thread() {
    let (repo, git, _paths) = setup();
    let editor = create_mock_editor(repo.path(), "Body from editor");

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["new", "issue", "Edit test", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "edit thread creation failed: stdout={stdout}, stderr={stderr}"
    );

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.body.as_deref(), Some("Body from editor"));
}

#[test]
fn edit_flag_creates_node() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("body"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let editor = create_mock_editor(repo.path(), "Claim from editor");
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["claim", "ASK-0001", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "edit claim failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Added claim"));

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.nodes.len(), 1);
    assert_eq!(state.nodes[0].body, "Claim from editor");
}

#[test]
fn edit_flag_revises_body() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("original"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let editor = create_mock_editor(repo.path(), "Revised from editor");
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["revise", "ASK-0001", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "edit revise failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Body revised"));

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.body.as_deref(), Some("Revised from editor"));
}

#[test]
fn edit_conflicts_with_body_flag() {
    let (repo, _git, _paths) = setup();
    let editor = create_mock_editor(repo.path(), "unused");

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["new", "issue", "Test", "--edit", "--body", "direct"])
        .output()
        .expect("failed to run");
    assert!(
        !output.status.success(),
        "should fail when --edit and --body are both provided"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "expected conflict error, got: {stderr}"
    );
}

#[test]
fn edit_conflicts_with_body_file() {
    let (repo, _git, _paths) = setup();
    let editor = create_mock_editor(repo.path(), "unused");
    let body_file = repo.path().join("body.md");
    fs::write(&body_file, "file body").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args([
            "new",
            "issue",
            "Test",
            "--edit",
            "--body-file",
            body_file.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run");
    assert!(
        !output.status.success(),
        "should fail when --edit and --body-file are both provided"
    );
}

#[test]
fn edit_empty_body_aborts() {
    let (repo, _git, _paths) = setup();
    let editor = create_noop_editor(repo.path());

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["new", "issue", "Empty test", "--edit"])
        .output()
        .expect("failed to run");
    assert!(
        !output.status.success(),
        "should abort when editor content is empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty body from editor"),
        "expected empty body error, got: {stderr}"
    );
}

#[test]
fn edit_strips_comment_lines() {
    let (repo, git, _paths) = setup();
    let editor = create_mock_editor(
        repo.path(),
        "# This is a comment\nReal content here\n# Another comment\nMore real content",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["new", "issue", "Comment test", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "edit with comments failed: stdout={stdout}, stderr={stderr}"
    );

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    let body = state.body.unwrap();
    assert!(!body.contains("# This is a comment"));
    assert!(!body.contains("# Another comment"));
    assert!(body.contains("Real content here"));
    assert!(body.contains("More real content"));
}

#[test]
fn edit_uses_visual_env_var() {
    let (repo, git, _paths) = setup();
    let visual_editor = create_mock_editor(repo.path(), "Body from VISUAL");

    // Set VISUAL, set EDITOR to something different to prove VISUAL takes precedence
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("VISUAL", &visual_editor)
        .env("EDITOR", "false") // would fail if used
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["new", "issue", "Visual test", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "VISUAL editor failed: stdout={stdout}, stderr={stderr}"
    );

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.body.as_deref(), Some("Body from VISUAL"));
}

#[test]
fn edit_flag_with_revise_body_subcommand() {
    let (repo, git, _paths) = setup();
    let clock = fixed_clock();
    create::create_thread(
        &git,
        ThreadKind::Issue,
        "Test issue",
        Some("original"),
        "human/alice",
        &clock,
    )
    .unwrap();

    let editor = create_mock_editor(repo.path(), "Revised via subcommand");
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", &editor)
        .env_remove("VISUAL")
        .env("GIT_FORUM_EDITOR_FORCE", "1")
        .args(["revise", "body", "ASK-0001", "--edit"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "edit revise body subcommand failed: stdout={stdout}, stderr={stderr}"
    );

    let state = thread::replay_thread(&git, "ASK-0001").unwrap();
    assert_eq!(state.body.as_deref(), Some("Revised via subcommand"));
}

#[test]
fn edit_rejects_non_interactive_stdin() {
    let (repo, _git, _paths) = setup();

    // Do NOT set GIT_FORUM_EDITOR_FORCE — let the TTY check fire.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .env("EDITOR", "false")
        .env_remove("VISUAL")
        .env_remove("GIT_FORUM_EDITOR_FORCE")
        .args(["new", "issue", "Non-interactive test", "--edit"])
        .output()
        .expect("failed to run");

    assert!(
        !output.status.success(),
        "should fail in non-interactive context"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--edit requires an interactive terminal"),
        "expected actionable error, got: {stderr}"
    );
}
