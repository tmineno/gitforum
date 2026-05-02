//! Module integration tests for `src/internal/init.rs` and the
//! commit-identity surface in `src/internal/git_ops.rs`
//! (test-policy.md category 1). Commit identity lives here because it
//! is configured at init time and only meaningful when a forum exists.

mod support;

use git_forum::internal::config::CommitIdentity;
use git_forum::internal::event::{self, ThreadKind};
use git_forum::internal::init;

use support::forum::{
    sample_create_event as sample_create, setup_no_init as setup, test_thread_id,
};

fn commit_author_name(repo_path: &std::path::Path, sha: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%an", sha])
        .current_dir(repo_path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git log failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn commit_author_email(repo_path: &std::path::Path, sha: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ae", sha])
        .current_dir(repo_path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git log failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// ---- Init ----

#[test]
fn init_creates_forum_structure() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();

    assert!(paths.dot_forum.join("policy.toml").exists());
    assert!(paths.dot_forum.join("actors.toml").exists());
    assert!(paths.dot_forum.join("templates").join("issue.md").exists());
    assert!(paths.dot_forum.join("templates").join("rfc.md").exists());
    assert!(paths.git_forum.join("logs").is_dir());
}

#[test]
fn init_policy_is_valid_toml() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let content = std::fs::read_to_string(paths.dot_forum.join("policy.toml")).unwrap();
    let _parsed: toml::Table = content.parse().expect("policy.toml should be valid TOML");
}

#[test]
fn init_is_idempotent() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();
    init::init_forum(&paths).unwrap();
    assert!(paths.dot_forum.join("policy.toml").exists());
}

// Regression test for @96u6zxmc / ADR-007. The committed rfc.md had
// drifted to the 1-line `# {title}` stub while the (now-removed) inline
// constant carried the multi-section scaffold. This test pins the seed
// content so the drift cannot silently return.
#[test]
fn init_writes_non_trivial_rfc_template() {
    let (_repo, _git, paths) = setup();
    init::init_forum(&paths).unwrap();

    let rfc = std::fs::read_to_string(paths.dot_forum.join("templates").join("rfc.md")).unwrap();
    for section in ["## Goal", "## Non-goals", "## Context", "## Proposal"] {
        assert!(
            rfc.contains(section),
            "rfc template missing `{section}` section; contents:\n{rfc}"
        );
    }
}

#[test]
fn init_local_only_skips_shared_forum_content() {
    let (_repo, _git, paths) = setup();
    init::init_forum_local(&paths).unwrap();

    assert!(paths.git_forum.join("logs").is_dir());
    assert!(!paths.dot_forum.join("policy.toml").exists());
    assert!(!paths.dot_forum.join("templates").join("rfc.md").exists());
}

// ---- Commit identity ----

#[test]
fn commit_uses_git_config_by_default() {
    let (repo, git, _paths) = setup();
    let tid = test_thread_id(ThreadKind::Rfc, 10);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Default identity");
    let sha = event::write_event(&git, &ev).unwrap();

    let name = commit_author_name(repo.path(), &sha);
    let email = commit_author_email(repo.path(), &sha);
    assert!(!name.is_empty(), "commit should have an author name");
    assert!(!email.is_empty(), "commit should have an author email");
}

#[test]
fn commit_identity_overrides_author_name_and_email() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: Some("Forum Bot".into()),
        email: Some("bot@forum.local".into()),
    });
    let tid = test_thread_id(ThreadKind::Rfc, 11);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Custom identity");
    let sha = event::write_event(&git, &ev).unwrap();

    assert_eq!(commit_author_name(repo.path(), &sha), "Forum Bot");
    assert_eq!(commit_author_email(repo.path(), &sha), "bot@forum.local");
}

#[test]
fn commit_identity_partial_override_name_only() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: Some("Pseudonym".into()),
        email: None,
    });
    let tid = test_thread_id(ThreadKind::Rfc, 12);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Name-only override");
    let sha = event::write_event(&git, &ev).unwrap();

    assert_eq!(commit_author_name(repo.path(), &sha), "Pseudonym");
    let email = commit_author_email(repo.path(), &sha);
    assert!(!email.is_empty(), "email should fall through to git config");
}

#[test]
fn commit_identity_partial_override_email_only() {
    let (repo, mut git, _paths) = setup();
    git.set_commit_identity(CommitIdentity {
        name: None,
        email: Some("private@example.com".into()),
    });
    let tid = test_thread_id(ThreadKind::Rfc, 13);
    let ev = sample_create(&tid, ThreadKind::Rfc, "Email-only override");
    let sha = event::write_event(&git, &ev).unwrap();

    let name = commit_author_name(repo.path(), &sha);
    assert!(!name.is_empty(), "name should fall through to git config");
    assert_eq!(
        commit_author_email(repo.path(), &sha),
        "private@example.com"
    );
}
