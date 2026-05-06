mod support;

use std::io::Write;
use std::process::{Command, Stdio};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::thread;

use support::cli::{extract_created_id, make_thread_via_cli};

/// Drive a fresh RFC to `accepted` via the CLI state arms (used by
/// the `--from-thread` regression that wants the source already
/// accepted).
fn drive_rfc_to_accepted(repo_path: &std::path::Path, thread_id: &str) {
    for state in &["proposed", "under-review", "accepted"] {
        let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
            .current_dir(repo_path)
            .args(["state", thread_id, state])
            .output()
            .expect("failed to run git-forum state");
        assert!(
            output.status.success(),
            "drive_rfc_to_accepted({state}) failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn thread_new_accepts_body_from_stdin() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Parser fails", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"Long body from stdin\nwith another line\n")
        .expect("failed to write stdin");

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(output.status.success());
    let thread_id = extract_created_id(&output);

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(
        state.body.as_deref(),
        Some("Long body from stdin\nwith another line\n")
    );
}

#[test]
fn thread_new_body_stdin_rejects_empty_input() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Empty body", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    // Close stdin immediately without writing anything
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(!output.status.success(), "empty stdin should cause failure");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty input"),
        "error should mention empty input: {stderr}"
    );
}

#[test]
fn thread_new_body_stdin_rejects_whitespace_only() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "Whitespace body", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run git-forum issue new");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"   \n  \n  ")
        .expect("failed to write whitespace");

    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(
        !output.status.success(),
        "whitespace-only stdin should cause failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("empty input"),
        "error should mention empty input: {stderr}"
    );
}

#[test]
fn thread_new_can_create_link_immediately() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let create_rfc = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "new",
            "rfc",
            "Switch backend",
            "--body",
            "## Goal\nSwitch to a new backend.",
        ])
        .output()
        .expect("failed to create rfc");
    assert!(create_rfc.status.success());
    let rfc_id = extract_created_id(&create_rfc);

    let create_issue = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "new",
            "issue",
            "Implement backend",
            "--link-to",
            &rfc_id,
            "--rel",
            "implements",
        ])
        .output()
        .expect("failed to create issue with link");
    assert!(create_issue.status.success());
    let issue_id = extract_created_id(&create_issue);

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &issue_id).unwrap();
    assert_eq!(state.links.len(), 1);
    assert_eq!(state.links[0].target_thread_id, rfc_id);
    assert_eq!(state.links[0].rel, "implements");
}

#[test]
fn from_thread_without_title_uses_default() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // Create source RFC and drive it to `accepted` via the CLI.
    let git = GitOps::new(repo.path().to_path_buf());
    let rfc_id = make_thread_via_cli(
        repo.path(),
        "rfc",
        "Original design",
        "Body of original RFC",
    );
    drive_rfc_to_accepted(repo.path(), &rfc_id);

    // Create new RFC from source without explicit title (regression for finding #2)
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "rfc", "--from-thread", &rfc_id])
        .output()
        .expect("failed to run git-forum rfc new --from-thread");
    assert!(
        output.status.success(),
        "from-thread without title should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let new_rfc_id = extract_created_id(&output);

    let state = thread::replay_thread(&git, &new_rfc_id).unwrap();
    assert_eq!(state.title, "v2: Original design");
    assert_eq!(state.body.as_deref(), Some("Body of original RFC"));
}

#[test]
fn from_thread_issue_to_issue_does_not_deprecate_source() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let git = GitOps::new(repo.path().to_path_buf());
    let source_id = make_thread_via_cli(
        repo.path(),
        "issue",
        "Original bug",
        "Body of original issue",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "--from-thread", &source_id])
        .output()
        .expect("failed to run git-forum issue new --from-thread");
    assert!(
        output.status.success(),
        "issue --from-thread issue should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let new_id = extract_created_id(&output);

    // New issue has links and copied content
    let new_state = thread::replay_thread(&git, &new_id).unwrap();
    assert_eq!(new_state.title, "v2: Original bug");
    assert_eq!(new_state.body.as_deref(), Some("Body of original issue"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, source_id);
    assert_eq!(new_state.links[0].rel, "supersedes");

    // Source issue is NOT deprecated — remains in its prior state
    let source = thread::replay_thread(&git, &source_id).unwrap();
    assert_eq!(source.status, "open");
    assert_eq!(source.links.len(), 1);
    assert_eq!(source.links[0].target_thread_id, new_id);
    assert_eq!(source.links[0].rel, "superseded-by");
}

#[test]
fn from_thread_issue_to_rfc_does_not_deprecate_source() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let git = GitOps::new(repo.path().to_path_buf());
    let source_id = make_thread_via_cli(
        repo.path(),
        "issue",
        "Feature request",
        "We need a better API",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "rfc", "--from-thread", &source_id])
        .output()
        .expect("failed to run git-forum rfc new --from-thread");
    assert!(
        output.status.success(),
        "rfc --from-thread issue should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let new_id = extract_created_id(&output);

    // New RFC has links and copied content
    let new_state = thread::replay_thread(&git, &new_id).unwrap();
    assert_eq!(new_state.title, "v2: Feature request");
    assert_eq!(new_state.body.as_deref(), Some("We need a better API"));
    assert_eq!(new_state.links.len(), 1);
    assert_eq!(new_state.links[0].target_thread_id, source_id);
    assert_eq!(new_state.links[0].rel, "supersedes");

    // Source issue is NOT deprecated
    let source = thread::replay_thread(&git, &source_id).unwrap();
    assert_eq!(source.status, "open");
    assert_eq!(source.links.len(), 1);
    assert_eq!(source.links[0].target_thread_id, new_id);
    assert_eq!(source.links[0].rel, "superseded-by");
}

#[test]
fn from_thread_rfc_to_issue_is_rejected() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let _git = GitOps::new(repo.path().to_path_buf());
    let rfc_id = make_thread_via_cli(repo.path(), "rfc", "Some RFC", "RFC body");

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["new", "issue", "--from-thread", &rfc_id])
        .output()
        .expect("failed to run git-forum issue new --from-thread");
    assert!(
        !output.status.success(),
        "issue --from-thread RFC should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Track D: 1.x §9.2 RFC→issue rule restated in lifecycle terms.
    assert!(
        stderr.contains("cannot create an execution thread --from-thread a proposal"),
        "error should explain why: {stderr}"
    );
}

/// Track D: canonical `git forum thread new --lifecycle X --tag Y` form
/// (SPEC-2.0 §9.1) — power-user / scripting interface that lets callers set
/// arbitrary lifecycle/tag combinations without going through the kind preset.
#[test]
fn canonical_thread_new_with_lifecycle_and_tag() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "thread",
            "new",
            "Tracker for migration sweep",
            "--lifecycle",
            "execution",
            "--tag",
            "migration",
            "--tag",
            "backend",
            "--body",
            "Sweep all callers.",
        ])
        .output()
        .expect("failed to run git-forum thread new");
    assert!(
        output.status.success(),
        "thread new failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let thread_id = extract_created_id(&output);

    let git = GitOps::new(repo.path().to_path_buf());
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(
        state.category, "task",
        "facet_set should persist execution lifecycle (category=task)"
    );
    assert!(
        !state.tags.iter().any(|t| t == "decision"),
        "execution should not carry the decision tag"
    );
    assert!(
        state.lifecycle_explicit,
        "explicit facet_set must flip lifecycle_explicit"
    );
    assert!(
        state.tags.contains(&"migration".to_string()),
        "expected `migration` tag on the thread, got {:?}",
        state.tags
    );
    assert!(
        state.tags.contains(&"backend".to_string()),
        "expected `backend` tag on the thread, got {:?}",
        state.tags
    );
}

/// Track D: rejecting unknown lifecycle values surfaces a usable error so
/// scripts can detect and adjust.
#[test]
fn canonical_thread_new_rejects_unknown_lifecycle() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args([
            "thread",
            "new",
            "Doomed thread",
            "--lifecycle",
            "bogus",
            "--body",
            "ignored",
        ])
        .output()
        .expect("failed to run git-forum thread new");
    assert!(!output.status.success(), "should reject unknown lifecycle");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown lifecycle 'bogus'"),
        "error should name the bad value: {stderr}",
    );
}

/// Pin the SPEC-2.0 §9.1 preset alias table: every alias resolves to a
/// concrete (lifecycle, tags) pair on the created thread. The user-visible
/// contract is the canonical 2.0 axis pair; the storage `kind` is a backing
/// detail (`Commands::New` derives it from lifecycle, not from the preset).
///
/// This test guards the alias data against silent divergence — adding,
/// renaming, or re-keying an alias without updating the data table will fail
/// here. It is intentionally placed before the `WorkflowSpec` consolidation
/// (thread `34ith16h`) so the structural change cannot regress the contract.
#[test]
fn preset_aliases_resolve_to_canonical_axes() {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();

    // (alias, expected category, expected tags)
    // task `1v400j3l`: lifecycle was a typed enum; now the canonical
    // SPEC-3.0 fingerprint is `category + tags` (the lifecycle label
    // is derived for display via `policy::lifecycle_label_for`).
    let cases: &[(&str, &str, &[&str])] = &[
        ("ask", "task", &["bug"]),
        ("bug", "task", &["bug"]),
        ("issue", "task", &["bug"]),
        ("job", "task", &["task"]),
        ("task", "task", &["task"]),
        ("rfc", "rfc", &["cross-cutting"]),
        // SPEC-3.0 §8.3: `dec`/`record` collapse onto category `task`
        // with the canonical `decision` tag.
        ("dec", "task", &["decision"]),
    ];

    let git = GitOps::new(repo.path().to_path_buf());
    for &(alias, expected_category, expected_tags) in cases {
        let title = format!("Preset alias {alias}");
        let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
            .current_dir(repo.path())
            // --body satisfies the default `required_body` creation rule on
            // proposal threads. Other aliases tolerate it (the rule is
            // lifecycle-keyed; passing a body never blocks).
            .args(["new", alias, &title, "--body", "fixture body"])
            .output()
            .unwrap_or_else(|e| panic!("failed to run new {alias}: {e}"));
        assert!(
            output.status.success(),
            "alias {alias} should succeed: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        let id = extract_created_id(&output);
        let state = thread::replay_thread(&git, &id).unwrap();
        assert_eq!(
            state.category, expected_category,
            "alias={alias} id={id} category"
        );
        let mut got = state.tags.clone();
        got.sort();
        let mut want: Vec<String> = expected_tags.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(got, want, "alias={alias} id={id} tags");
    }
}

/// Pin the legacy ID-prefix alias table (`ASK` ⇄ `ISSUE`, `JOB` ⇄ `TASK`).
/// Used by hook scanning, migrate, and id_alloc to recognise pre-2.0 IDs;
/// must keep working through any `WorkflowSpec` consolidation.
#[test]
fn id_prefix_aliases_resolve_to_canonical_kind() {
    use git_forum::internal::legacy::event::ThreadKind;
    assert_eq!(ThreadKind::from_id_prefix("ASK"), Some(ThreadKind::Issue));
    assert_eq!(ThreadKind::from_id_prefix("ISSUE"), Some(ThreadKind::Issue));
    assert_eq!(ThreadKind::from_id_prefix("JOB"), Some(ThreadKind::Task));
    assert_eq!(ThreadKind::from_id_prefix("TASK"), Some(ThreadKind::Task));
    assert_eq!(ThreadKind::from_id_prefix("RFC"), Some(ThreadKind::Rfc));
    assert_eq!(ThreadKind::from_id_prefix("DEC"), Some(ThreadKind::Dec));
    assert_eq!(ThreadKind::from_id_prefix("BOGUS"), None);
}

/// Track D / SPEC-2.0 §10.2: kind-prefixed subcommand groupings are removed
/// in 2.0 and produce a hard error with a redirect to the top-level form.
#[test]
fn kind_prefixed_subcommand_is_a_hard_error() {
    let repo = support::repo::TestRepo::new();

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["rfc", "new", "Whatever"])
        .output()
        .expect("failed to run git-forum rfc new");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("removed in 2.0"),
        "should advertise the 2.0 removal: {stderr}",
    );
    assert!(
        stderr.contains("git forum new"),
        "should redirect to top-level form: {stderr}",
    );
}
