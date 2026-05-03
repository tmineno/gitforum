//! v3.0 storage-shape regression tests.
//!
//! Two test classes live in this file; do not conflate them:
//!
//! 1. **Direct snapshot-store shape tests** (RFC `7ymtc4b2` Phase 1).
//!    These run `internal::snapshot::store::write_snapshot` directly
//!    and assert the SPEC-3.0 §4.2 tree layout. They are unrelated to
//!    the CLI cutover and run NOW. They guard against regressions in
//!    the snapshot writer's tree-assembly contract.
//!
//! 2. **CLI-cutover storage-shape gates** (task `4w8hm98j`).
//!    These run `git forum <subcommand>` and assert the resulting tree
//!    matches the v3 invariant. They are `#[ignore]`-gated until each
//!    Phase 2 slot lands; the matching cutover commit removes the
//!    `#[ignore]` and the corresponding entry in
//!    `tests/storage_v2_test.rs`. See `doc/internal/main-rs-audit.md`
//!    for slot order and `doc/internal/cli-coverage-audit.md` for
//!    cutover discipline.

mod support;

use std::process::{Command, Output};

use git_forum::internal::config::RepoPaths;
use git_forum::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{write_snapshot, Link, Links, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;

fn bin() -> String {
    env!("CARGO_BIN_EXE_git-forum").to_string()
}

fn run_ok(repo: &support::repo::TestRepo, args: &[&str]) -> Output {
    let out = Command::new(bin())
        .current_dir(repo.path())
        .args(args)
        .output()
        .expect("git-forum invocation failed");
    assert!(out.status.success());
    out
}

fn extract_created_id(out: &Output) -> String {
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .strip_prefix("Created ")
        .unwrap_or(s.trim())
        .split_whitespace()
        .next()
        .expect("no thread id in `Created …` line")
        .to_string()
}

fn fresh_cli_repo() -> support::repo::TestRepo {
    let repo = support::repo::TestRepo::new();
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    repo
}

fn list_tree_paths(git: &GitOps, refname: &str) -> Vec<String> {
    let tip = git.run(&["rev-parse", refname]).unwrap();
    let out = git
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .unwrap();
    let mut paths: Vec<String> = out.lines().map(|s| s.to_string()).collect();
    paths.sort();
    paths
}

// --------------------------------------------------------------------
// (1) Direct snapshot-store shape tests — Phase 1, run NOW.
// --------------------------------------------------------------------

fn sample_thread(id: &str) -> ThreadSnapshot {
    ThreadSnapshot {
        schema_version: 3,
        id: id.into(),
        title: "Storage-shape probe".into(),
        category: "rfc".into(),
        status: "draft".into(),
        tags: vec![],
        created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
        created_by: "human/alice".into(),
        updated_at: "2026-05-03T00:00:00Z".parse().unwrap(),
        updated_by: "human/alice".into(),
        branch: None,
        supersedes: vec![],
    }
}

/// SPEC-3.0 §4.2: a snapshot with non-empty body + node + link +
/// evidence writes all five optional files. The required `thread.toml`
/// is always present.
#[test]
fn v3_full_fixture_writes_every_optional_file() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());

    let doc = ThreadDocument {
        snapshot: sample_thread("FULL01"),
        body: Some("Body text.\n".into()),
        nodes: vec![NodeWithBody {
            record: NodeRecord {
                id: "node1".into(),
                kind: NodeKind::Comment,
                status: NodeStatus::Open,
                created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                created_by: "human/alice".into(),
                updated_at: None,
                updated_by: None,
                reply_to: None,
                legacy_label: None,
            },
            body: "Node body.\n".into(),
        }],
        links: Links {
            entries: vec![Link {
                target: "OTHER".into(),
                rel: "implements".into(),
                created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                created_by: "human/alice".into(),
            }],
        },
        evidence: EvidenceFile {
            entries: vec![EvidenceRecord {
                id: "ev1".into(),
                kind: EvidenceKind::Commit,
                ref_target: "HEAD".into(),
                created_at: "2026-05-03T00:00:00Z".parse().unwrap(),
                created_by: "human/alice".into(),
            }],
        },
    };
    write_snapshot(&git, "FULL01", &doc, "create FULL01").unwrap();

    let paths = list_tree_paths(&git, "refs/forum/threads/FULL01");
    assert_eq!(
        paths,
        vec![
            "body.md".to_string(),
            "evidence.toml".to_string(),
            "links.toml".to_string(),
            "nodes/node1.md".to_string(),
            "nodes/node1.toml".to_string(),
            "thread.toml".to_string(),
        ],
        "full fixture must write all five SPEC-3.0 §4.2 optional files plus the required thread.toml"
    );
}

/// SPEC-3.0 §4.2: an empty-everywhere snapshot writes ONLY the
/// required `thread.toml`. The optional `body.md`, `nodes/`,
/// `links.toml`, and `evidence.toml` files MUST be absent — not
/// present-but-empty.
#[test]
fn v3_empty_snapshot_omits_all_optional_files() {
    let repo = support::repo::TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());

    let doc = ThreadDocument::new(sample_thread("EMPTY1"));
    write_snapshot(&git, "EMPTY1", &doc, "create EMPTY1").unwrap();

    let paths = list_tree_paths(&git, "refs/forum/threads/EMPTY1");
    assert_eq!(
        paths,
        vec!["thread.toml".to_string()],
        "empty snapshot must omit body.md, nodes/, links.toml, evidence.toml per SPEC-3.0 §4.2"
    );
}

// --------------------------------------------------------------------
// (2) CLI-cutover storage-shape gates — `#[ignore]` until Phase 2.
// --------------------------------------------------------------------

/// v3 invariant (SPEC-3.0 §4): `git forum new` writes a snapshot tree
/// containing `thread.toml` (and `links.toml` if links were specified)
/// at `refs/forum/threads/<id>`. Unblocks at Phase 2 slot 1
/// (`thread_new` cutover); the v2 counterpart in
/// `tests/storage_v2_test.rs` is removed in the same commit.
#[test]
#[ignore = "unblocked at Phase 2 slot 1 (thread_new cutover) per RFC 7ymtc4b2"]
fn v3_cli_thread_new_writes_thread_toml() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(&repo, &["new", "issue", "v3 shape probe"]));

    let git = GitOps::new(repo.path().to_path_buf());
    let tip_ref = format!("refs/forum/threads/{id}");
    let tip = git.run(&["rev-parse", &tip_ref]).expect("rev-parse tip");

    let tree = git
        .run(&["ls-tree", "-r", "--name-only", tip.trim()])
        .expect("ls-tree tip");
    let entries: Vec<&str> = tree.lines().collect();
    assert!(
        entries.contains(&"thread.toml"),
        "v3 snapshot tree must contain thread.toml; got {entries:?}"
    );

    let body = git
        .run(&["cat-file", "-p", &format!("{}:thread.toml", tip.trim())])
        .expect("cat-file thread.toml");
    assert!(
        body.contains("[thread]"),
        "v3 thread.toml must contain [thread] section; body was:\n{body}"
    );
}
