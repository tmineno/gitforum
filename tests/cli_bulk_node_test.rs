mod support;

use std::process::Command;

use chrono::{TimeZone, Utc};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::init;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{self, store::write_snapshot, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::{self, ThreadSnapshot};

fn setup() -> (support::repo::TestRepo, GitOps, RepoPaths) {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

/// Snapshot fixture: write a fresh SPEC-3.0 thread with the given
/// kind shorthand and title. Replaces the legacy `create::create_thread`
/// fixture path now that task `1v400j3l` forbids non-migrate code
/// paths from consuming legacy event chains.
///
/// `kind` accepts the v2 vocabulary (`rfc`/`issue`/`task`/`dec`) and
/// projects to the SPEC-3.0 §8.3 category + canonical-tag pair so the
/// snapshot is a valid 3.0 thread that the category registry can route.
fn make_snapshot_thread(git: &GitOps, kind: &str, title: &str, seed: u8) -> String {
    let id = format!("blk{seed:02x}{seed:02x}{seed:02x}", seed = seed.max(1));
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let (category, tags, initial_status) = match kind {
        "rfc" => ("rfc", vec![], "draft"),
        "issue" => ("task", vec!["bug".to_string()], "open"),
        "dec" => ("task", vec!["decision".to_string()], "open"),
        _ => ("task", vec![], "open"),
    };
    let doc = ThreadDocument::new(ThreadSnapshot {
        schema_version: ThreadSnapshot::SCHEMA_VERSION,
        id: id.clone(),
        title: title.to_string(),
        category: category.to_string(),
        status: initial_status.to_string(),
        tags,
        created_at: now,
        created_by: "human/alice".into(),
        updated_at: now,
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
        visibility: Default::default(),
    });
    write_snapshot(git, &id, &doc, "create test thread").unwrap();
    id
}

/// Append a node to a snapshot thread; returns the new node id.
fn append_snapshot_node(
    git: &GitOps,
    thread_id: &str,
    kind: NodeKind,
    body: &str,
    actor: &str,
) -> String {
    let mut doc = snapshot::read_snapshot(git, thread_id).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap();
    let id = id_alloc::alloc_bare_thread_id(actor, body, &now.to_rfc3339());
    doc.nodes.push(NodeWithBody {
        record: NodeRecord {
            id: id.clone(),
            kind,
            status: NodeStatus::Open,
            created_at: now,
            created_by: actor.into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: body.into(),
    });
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();
    write_snapshot(git, thread_id, &doc, "append test node").unwrap();
    id
}

/// Mark a node as resolved on a snapshot thread.
fn resolve_snapshot_node(git: &GitOps, thread_id: &str, node_id: &str, actor: &str) {
    let mut doc = snapshot::read_snapshot(git, thread_id).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 2, 0).unwrap();
    if let Some(node) = doc.nodes.iter_mut().find(|n| n.record.id == node_id) {
        node.record.status = NodeStatus::Resolved;
        node.record.updated_at = Some(now);
        node.record.updated_by = Some(actor.into());
    }
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();
    write_snapshot(git, thread_id, &doc, "resolve test node").unwrap();
}

#[test]
fn retract_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "rfc", "Bulk retract test", 0x01);
    let n1 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Comment,
        "Summary 1",
        "human/alice",
    );
    let n2 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Comment,
        "Summary 2",
        "human/alice",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["retract", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "retract multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Retracted"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(
        state.nodes[0].record.status,
        git_forum::internal::node::NodeStatus::Retracted
    );
    assert_eq!(
        state.nodes[1].record.status,
        git_forum::internal::node::NodeStatus::Retracted
    );
}

#[test]
fn resolve_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "rfc", "Bulk resolve test", 0x02);
    let n1 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Objection,
        "Objection 1",
        "human/bob",
    );
    let n2 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Objection,
        "Objection 2",
        "human/bob",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["resolve", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "resolve multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Resolved"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert!(state.open_objections().is_empty());
}

#[test]
fn reopen_multiple_nodes() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "rfc", "Bulk reopen test", 0x03);
    let n1 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Objection,
        "Objection 1",
        "human/bob",
    );
    let n2 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Objection,
        "Objection 2",
        "human/bob",
    );

    // Resolve both first.
    resolve_snapshot_node(&git, &thread_id, &n1, "human/alice");
    resolve_snapshot_node(&git, &thread_id, &n2, "human/alice");

    // Reopen both.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["reopen", &thread_id, &n1, &n2])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "reopen multi failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(stdout.contains("Reopened"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.open_objections().len(), 2);
}

#[test]
fn single_node_id_still_works() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "issue", "Single node test", 0x04);
    let n1 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Action,
        "Do something",
        "human/alice",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["resolve", &thread_id, &n1])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "single resolve failed");
    assert!(stdout.contains("Resolved"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(
        state.nodes[0].record.status,
        git_forum::internal::node::NodeStatus::Resolved
    );
}

#[test]
fn bulk_retract_reports_failures_inline() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "rfc", "Failure test", 0x05);
    let n1 = append_snapshot_node(
        &git,
        &thread_id,
        NodeKind::Comment,
        "Good node",
        "human/alice",
    );

    // Use a valid node and a bogus one.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["retract", &thread_id, &n1, "bogus_node_id_does_not_exist"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should exit non-zero.
    assert!(!output.status.success());
    // The valid node should still be retracted.
    assert!(stdout.contains("Retracted"));
    // The bogus one should report an error.
    assert!(stderr.contains("error:"));

    // Verify the valid node was retracted despite the failure.
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(
        state.nodes[0].record.status,
        git_forum::internal::node::NodeStatus::Retracted
    );
}

#[test]
fn reopen_without_node_ids_is_rejected() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "issue", "Thread reopen test", 0x06);

    // Close the thread.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["close", &thread_id])
        .output()
        .expect("failed to run");
    assert!(output.status.success());

    // Reopen without node IDs should reopen the thread itself.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["reopen", &thread_id])
        .output()
        .expect("failed to run");
    assert!(
        output.status.success(),
        "reopen without node IDs should reopen the thread: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
}

#[test]
fn thread_reopen_via_issue_subcommand() {
    let (repo, git, _paths) = setup();
    let thread_id = make_snapshot_thread(&git, "issue", "Thread reopen test", 0x07);

    // Close the thread.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["close", &thread_id])
        .output()
        .expect("failed to run");
    assert!(output.status.success());

    // Reopen via top-level shorthand.
    let output = Command::new(env!("CARGO_BIN_EXE_git-forum"))
        .current_dir(repo.path())
        .args(["reopen", &thread_id])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "thread reopen via top-level shorthand failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("-> open"));

    let state = thread::replay_thread(&git, &thread_id).unwrap();
    assert_eq!(state.status, "open");
}
