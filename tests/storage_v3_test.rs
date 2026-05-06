//! v3.0 storage-shape regression tests.
//!
//! Two test classes live in this file; do not conflate them:
//!
//! 1. **Direct snapshot-store shape tests** (RFC `7ymtc4b2`, task `qa8u71j9`).
//!    These run `internal::snapshot::store::write_snapshot` directly
//!    and assert the SPEC-3.0 §4.2 tree layout. They are unrelated to
//!    the CLI cutover and run NOW. They guard against regressions in
//!    the snapshot writer's tree-assembly contract.
//!
//! 2. **CLI-cutover storage-shape gates** (task `4w8hm98j`).
//!    These run `git forum <subcommand>` and assert the resulting tree
//!    matches the v3 invariant. They are `#[ignore]`-gated until each
//!    task `1hg98odf` cutover lands; the matching cutover commit removes the
//!    `#[ignore]` and the corresponding entry in
//!    `tests/storage_v2_test.rs`. See `doc/internal/main-rs-audit.md`
//!    for slot order and `doc/internal/cli-coverage-audit.md` for
//!    cutover discipline.

mod support;

use git_forum::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::snapshot::{write_snapshot, Link, Links, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;

use support::cli::{extract_created_id, fresh_repo as fresh_cli_repo, run_ok};
use support::git::{list_tree_paths, ls_thread_tip, read_thread_file};

// --------------------------------------------------------------------
// (1) Direct snapshot-store shape tests — task `qa8u71j9`, run NOW.
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
// (2) CLI-cutover storage-shape gates — task `1hg98odf`.
// --------------------------------------------------------------------

/// v3 invariant (SPEC-3.0 §4): `git forum new` writes a snapshot tree
/// containing `thread.toml` (and `links.toml` if links were specified)
/// at `refs/forum/threads/<id>`. Unblocked at task `1hg98odf`
/// (`thread_new` cutover, RFC `7ymtc4b2`); the v2 counterpart in
/// `tests/storage_v2_test.rs` is removed in the same commit.
#[test]
fn v3_cli_thread_new_writes_thread_toml() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "issue", "v3 shape probe"]));

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
        body.contains("schema_version = 3"),
        "v3 thread.toml must declare schema_version = 3; body was:\n{body}"
    );
    assert!(
        body.contains(&format!("id = \"{id}\"")),
        "v3 thread.toml must declare its own id; body was:\n{body}"
    );
    assert!(
        body.contains("category = "),
        "v3 thread.toml must declare a category; body was:\n{body}"
    );
}

/// v3 invariant (slot 2 / shorthand_say): `git forum comment` writes
/// `nodes/<id>.toml` and `nodes/<id>.md` to the snapshot tip.
#[test]
fn v3_cli_comment_writes_node_files() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "comment shape probe"],
    ));
    run_ok(repo.path(), &["comment", &id, "--body", "hello"]);

    let git = GitOps::new(repo.path().to_path_buf());
    let entries = ls_thread_tip(&git, &id);
    let has_node_toml = entries
        .iter()
        .any(|e| e.starts_with("nodes/") && e.ends_with(".toml"));
    let has_node_md = entries
        .iter()
        .any(|e| e.starts_with("nodes/") && e.ends_with(".md"));
    assert!(
        has_node_toml && has_node_md,
        "v3 snapshot must contain nodes/<id>.{{toml,md}} after `comment`; got {entries:?}"
    );
}

/// v3 invariant (slot 3 / state): `git forum state ... <NEW>` updates
/// `thread.toml.status` directly (no event commit appended). The
/// execution lifecycle's `closed` shorthand resolves to `done`.
#[test]
fn v3_cli_state_updates_thread_toml_status() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(repo.path(), &["new", "issue", "state shape probe"]));
    run_ok(repo.path(), &["state", &id, "closed"]);

    let git = GitOps::new(repo.path().to_path_buf());
    let body = read_thread_file(&git, &id, "thread.toml");
    assert!(
        body.contains("status = \"done\""),
        "v3 thread.toml must reflect the new status; got:\n{body}"
    );
}

/// v3 invariant (slot 4 / node_bulk): `git forum resolve` flips
/// `nodes/<id>.toml.status` from `open` to `resolved`.
#[test]
fn v3_cli_resolve_updates_node_status() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "resolve shape probe"],
    ));
    let comment = run_ok(repo.path(), &["comment", &id, "--body", "open question"]);
    let comment_stdout = String::from_utf8_lossy(&comment.stdout);
    let node_id = comment_stdout
        .lines()
        .find_map(|l| l.strip_prefix("Added comment "))
        .expect("node id in `Added comment` line")
        .trim()
        .to_string();

    run_ok(repo.path(), &["resolve", &id, &node_id]);

    let git = GitOps::new(repo.path().to_path_buf());
    let entries = ls_thread_tip(&git, &id);
    let toml = entries
        .iter()
        .find(|e| e.starts_with("nodes/") && e.ends_with(".toml"))
        .expect("nodes/<id>.toml present");
    let body = read_thread_file(&git, &id, toml);
    assert!(
        body.contains("status = \"resolved\""),
        "v3 nodes/*.toml must reflect resolved status; got:\n{body}"
    );
}

/// v3 invariant (slot 5 / revise): `git forum revise` overwrites
/// `body.md` with the new text.
#[test]
fn v3_cli_revise_overwrites_body_md() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "revise shape probe", "--body", "v1"],
    ));
    run_ok(repo.path(), &["revise", &id, "--body", "v2 final"]);

    let git = GitOps::new(repo.path().to_path_buf());
    let body_md = read_thread_file(&git, &id, "body.md");
    assert!(
        body_md.contains("v2 final"),
        "v3 body.md must contain the revised text; got:\n{body_md}"
    );
}

/// v3 invariant (slot 6 / branch): `git forum branch bind` writes
/// `branch = "<name>"` into `thread.toml`.
#[test]
fn v3_cli_branch_bind_writes_thread_toml_branch() {
    let repo = fresh_cli_repo();
    // The bind arm requires the branch ref to exist locally; create
    // an orphan branch on the test repo for the probe.
    let git = GitOps::new(repo.path().to_path_buf());
    git.run(&["checkout", "--orphan", "feat/storage-probe"])
        .unwrap();
    git.run(&["commit", "--allow-empty", "-m", "seed"]).unwrap();

    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "branch shape probe"],
    ));
    run_ok(repo.path(), &["branch", "bind", &id, "feat/storage-probe"]);

    let body = read_thread_file(&git, &id, "thread.toml");
    assert!(
        body.contains("branch = \"feat/storage-probe\""),
        "v3 thread.toml must record the bound branch; got:\n{body}"
    );
}

/// v3 invariant (slot 7g / retype): `git forum retype <ID> <NODE> action`
/// rewrites `nodes/<id>.toml.kind`.
#[test]
fn v3_cli_retype_updates_node_kind() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "retype shape probe"],
    ));
    let comment = run_ok(
        repo.path(),
        &["comment", &id, "--body", "let's call this an action"],
    );
    let comment_stdout = String::from_utf8_lossy(&comment.stdout);
    let node_id = comment_stdout
        .lines()
        .find_map(|l| l.strip_prefix("Added comment "))
        .expect("node id in `Added comment` line")
        .trim()
        .to_string();

    run_ok(repo.path(), &["retype", &id, &node_id, "--type", "action"]);

    let git = GitOps::new(repo.path().to_path_buf());
    let entries = ls_thread_tip(&git, &id);
    let toml = entries
        .iter()
        .find(|e| e.starts_with("nodes/") && e.ends_with(".toml"))
        .expect("nodes/<id>.toml present");
    let body = read_thread_file(&git, &id, toml);
    assert!(
        body.contains("type = \"action\""),
        "v3 nodes/*.toml must reflect retyped type; got:\n{body}"
    );
}

/// v3 invariant (slot 7j / evidence): `git forum evidence add` writes
/// `evidence.toml` with at least one entry.
#[test]
fn v3_cli_evidence_add_writes_evidence_toml() {
    let repo = fresh_cli_repo();
    let id = extract_created_id(&run_ok(
        repo.path(),
        &["new", "issue", "evidence shape probe"],
    ));
    // Need a real commit ref to satisfy commit-ref canonicalization.
    let git = GitOps::new(repo.path().to_path_buf());
    git.run(&["commit", "--allow-empty", "-m", "evidence target"])
        .unwrap();
    run_ok(
        repo.path(),
        &["evidence", "add", &id, "--kind", "commit", "--ref", "HEAD"],
    );

    let entries = ls_thread_tip(&git, &id);
    assert!(
        entries.iter().any(|e| e == "evidence.toml"),
        "v3 snapshot must contain evidence.toml after `evidence add`; got {entries:?}"
    );
    let body = read_thread_file(&git, &id, "evidence.toml");
    assert!(
        body.contains("kind = \"commit\""),
        "v3 evidence.toml must record the kind; got:\n{body}"
    );
}

/// v3 invariant (slot 7k / link): `git forum link` writes `links.toml`
/// with a row of (target, rel).
#[test]
fn v3_cli_link_writes_links_toml() {
    let repo = fresh_cli_repo();
    let src = extract_created_id(&run_ok(repo.path(), &["new", "issue", "link source"]));
    let dst = extract_created_id(&run_ok(
        repo.path(),
        &["new", "rfc", "link target", "--body", "## Goal\nv3 probe."],
    ));
    run_ok(repo.path(), &["link", &src, &dst, "--rel", "implements"]);

    let git = GitOps::new(repo.path().to_path_buf());
    let entries = ls_thread_tip(&git, &src);
    assert!(
        entries.iter().any(|e| e == "links.toml"),
        "v3 source snapshot must contain links.toml after `link`; got {entries:?}"
    );
    let body = read_thread_file(&git, &src, "links.toml");
    assert!(
        body.contains(&format!("target = \"{dst}\"")),
        "v3 links.toml must record the target id; got:\n{body}"
    );
    assert!(
        body.contains("rel = \"implements\""),
        "v3 links.toml must record the rel; got:\n{body}"
    );
}
