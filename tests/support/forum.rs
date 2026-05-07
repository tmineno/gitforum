//! Forum-aware fixture helpers shared across integration tests.
//!
//! task `913c4s9v`: rewritten to
//! construct test threads via the SPEC-3.0 snapshot writer
//! (`internal::snapshot::store::write_snapshot`) instead of the
//! deleted v2 `internal::create::create_thread` helper. The legacy
//! `state_change::change_state`, `reindex`, `index`, and v2-event
//! sample-event builders are gone with the modules that backed them.
//!
//! `#![allow(dead_code)]` is required because each `tests/*_test.rs`
//! compiles `support` independently — any helper not referenced by a
//! given binary would otherwise warn.

#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::node::{NodeKind, NodeRecord, NodeStatus};
use git_forum::internal::policy::{CategoryPolicy, GuardRule, Policy};
use git_forum::internal::snapshot::{self, write_snapshot, NodeWithBody, ThreadDocument};
use git_forum::internal::thread::ThreadSnapshot;

use super::repo::TestRepo;

// ---- Setup ----

/// Standard test setup: bare git repo + GitOps + RepoPaths with
/// `.forum/` initialized via `init::init_forum`. Use this unless the
/// test is asserting on init's own behavior.
pub fn setup() -> (TestRepo, GitOps, RepoPaths) {
    let repo = TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

/// Like `setup()` but skips `init_forum`. Use for `init_test.rs`.
pub fn setup_no_init() -> (TestRepo, GitOps, RepoPaths) {
    let repo = TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    (repo, git, paths)
}

// ---- Clock ----

/// FixedClock pinned to 2026-01-01T00:00:00Z. Most integration tests
/// want a single deterministic instant; tests that need time motion
/// build their own clock locally.
pub fn fixed_clock() -> FixedClock {
    FixedClock {
        instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
    }
}

// ---- Snapshot thread builders (task `913c4s9v`) ----

/// Build a deterministic 8-char id from a category + numeric seed.
/// Test threads use these to keep refs stable across runs.
pub fn test_thread_id(category: &str, seed: u8) -> String {
    let prefix = match category {
        "rfc" => "tstr",
        "dec" => "tstd",
        "task" => "tstt",
        _ => "tsti",
    };
    format!("{prefix}{seed:04x}")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// Initial status conventions matching what the v2 helpers produced
/// (proposal=draft, execution/record=open). Keeps existing test
/// assertions on `state.status` working against the new fixtures.
fn initial_status(category: &str) -> &'static str {
    match category {
        "rfc" => "draft",
        _ => "open",
    }
}

/// Construct a snapshot thread of the given category. Returns the
/// thread ID. Replaces the v2 `create::create_thread` fixture.
fn make_snapshot_thread(
    git: &GitOps,
    category: &str,
    title: &str,
    body: Option<&str>,
    seed: u8,
) -> String {
    let id = test_thread_id(category, seed);
    let doc = ThreadDocument {
        body: body.map(|s| s.to_string()),
        ..ThreadDocument::new(ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: id.clone(),
            title: title.to_string(),
            category: category.to_string(),
            status: initial_status(category).to_string(),
            tags: vec![],
            created_at: now(),
            created_by: "human/alice".into(),
            updated_at: now(),
            updated_by: "human/alice".into(),
            branch: None,
            supersedes: vec![],
            visibility: Default::default(),
        })
    };
    write_snapshot(git, &id, &doc, "create test thread").unwrap();
    id
}

/// Generic builder. `kind` is the v2 kind string ("rfc"/"dec"/"task"
/// /"issue") for callers that prefer the legacy verbiage; mapped to
/// the SPEC-3.0 category internally.
pub fn make_thread_with_seed(git: &GitOps, kind: &str, title: &str, seed: u8) -> String {
    let category = match kind {
        "rfc" => "rfc",
        "dec" => "dec",
        // SPEC-3.0 §8.3: issue/task collapse to category=task.
        _ => "task",
    };
    make_snapshot_thread(git, category, title, None, seed)
}

/// Convenience: same shape as the legacy `make_thread` (caller picks
/// kind), seeded with a fresh nonce so callers never collide.
pub fn make_thread(git: &GitOps, kind: &str, title: &str) -> String {
    make_thread_with_seed(git, kind, title, fresh_seed())
}

pub fn make_rfc(git: &GitOps) -> String {
    make_thread_with_seed(git, "rfc", "Test RFC", fresh_seed())
}

pub fn make_dec(git: &GitOps) -> String {
    // SPEC-3.0 §8.3: DEC collapses to category=task + decision tag. The
    // helper writes the snapshot directly so the lifecycle/tag panel
    // surfaces lifecycle=record (per `legacy_lifecycle_for_category`).
    let seed = fresh_seed();
    let id = test_thread_id("dec", seed);
    let n = now();
    let doc = ThreadDocument {
        body: Some(
            "## Context\nSome context\n## Decision\nUse Redis\n## Rationale\nFast\n## Impact\nNone"
                .into(),
        ),
        ..ThreadDocument::new(ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: id.clone(),
            title: "Test DEC".into(),
            category: "task".into(),
            status: "open".into(),
            tags: vec!["decision".into()],
            created_at: n,
            created_by: "human/alice".into(),
            updated_at: n,
            updated_by: "human/alice".into(),
            branch: None,
            supersedes: vec![],
            visibility: Default::default(),
        })
    };
    write_snapshot(git, &id, &doc, "create test thread").unwrap();
    id
}

pub fn make_task(git: &GitOps) -> String {
    make_thread_with_seed(git, "task", "Test TASK", fresh_seed())
}

/// Fresh-per-call seed so consecutive `make_*` calls don't collide
/// on the same id. Process-static counter is fine for test
/// determinism within a single run.
fn fresh_seed() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static COUNTER: AtomicU8 = AtomicU8::new(1);
    let s = COUNTER.fetch_add(1, Ordering::SeqCst);
    if s == 0 {
        // Avoid the 0 sentinel — bump again.
        COUNTER.fetch_add(1, Ordering::SeqCst)
    } else {
        s
    }
}

// ---- Snapshot mutators (task `913c4s9v` helpers) ----

/// Append a node to an existing snapshot thread. Returns the new node
/// id. Replaces v2 `write_ops::say_node` for tests that care about
/// node existence rather than v2 event-shape details.
pub fn append_snapshot_node(
    git: &GitOps,
    thread_id: &str,
    kind: NodeKind,
    body: &str,
    actor: &str,
) -> String {
    let mut doc = snapshot::read_snapshot(git, thread_id).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap();
    let id = git_forum::internal::id_alloc::alloc_bare_thread_id(actor, body, &now.to_rfc3339());
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

/// Convenience: link two threads via a `links.toml` write.
pub fn link_thread(git: &GitOps, from: &str, to: &str, rel: &str) {
    use git_forum::internal::snapshot::{Link, Links};
    let ts = Utc.with_ymd_and_hms(2026, 1, 1, 0, 2, 0).unwrap();
    let mut doc = snapshot::read_snapshot(git, from).unwrap();
    let mut entries = doc.links.entries.clone();
    entries.push(Link {
        target: to.into(),
        rel: rel.into(),
        created_at: ts,
        created_by: "human/alice".into(),
    });
    doc.links = Links { entries };
    doc.snapshot.updated_at = ts;
    write_snapshot(git, from, &doc, "add test link").unwrap();
}

// ---- Policy builders (SPEC-3.0 §3.2 category-keyed form) ----

pub fn empty_policy() -> Policy {
    Policy::default()
}

/// Build a single-category policy with the given guard rules at the
/// named transition.
pub fn make_category_guard_policy(
    category: &str,
    transition: &str,
    rules: Vec<GuardRule>,
) -> Policy {
    let mut policy = Policy::default();
    let mut cat = CategoryPolicy::default();
    cat.guards.insert(transition.to_string(), rules);
    policy.categories.insert(category.to_string(), cat);
    policy
}

/// Default policy used by `verify_test` to gate the rfc category's
/// review→done edge with `one_approval` + `no_open_objections`.
pub fn policy_with_under_review_guards() -> Policy {
    make_category_guard_policy(
        "rfc",
        "review->done",
        vec![GuardRule::NoOpenObjections, GuardRule::OneApproval],
    )
}

/// task category gate: review→done requires no open actions.
pub fn task_guard_policy() -> Policy {
    make_category_guard_policy("task", "review->done", vec![GuardRule::NoOpenActions])
}
