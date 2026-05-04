//! Forum-aware fixture helpers shared across integration tests.
//!
//! Phase 1 of the milestone-test refactor (TEST-POLICY.md, @am3lp1kk)
//! split each `m?_test.rs` into per-module files but kept setup
//! helpers duplicated. Phase 4 lives here: every helper that appears
//! in 2+ files is consolidated below so adding a new test only
//! requires `use support::forum::{setup, fixed_clock, ...};`.
//!
//! `#![allow(dead_code)]` is required because each `tests/*_test.rs`
//! compiles `support` independently — any helper not referenced by a
//! given binary would otherwise warn.

#![allow(dead_code)]

use chrono::{TimeZone, Utc};
use git_forum::internal::clock::FixedClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::event::{self, Event, EventType, ThreadKind};
use git_forum::internal::evidence;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id_alloc;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::policy::{CategoryPolicy, GuardRule, Policy};
use git_forum::internal::reindex;
use git_forum::internal::state_change;
use git_forum::internal::thread;

use super::repo::TestRepo;

// ---- Setup ----

/// Standard test setup: bare git repo + GitOps + RepoPaths with
/// `.forum/` initialized via `init::init_forum`. Use this unless the
/// test is asserting on init's own behavior or works exclusively
/// with raw events.
pub fn setup() -> (TestRepo, GitOps, RepoPaths) {
    let repo = TestRepo::new();
    let git = GitOps::new(repo.path().to_path_buf());
    let paths = RepoPaths::from_repo_root(repo.path());
    init::init_forum(&paths).unwrap();
    (repo, git, paths)
}

/// Like `setup()` but skips `init_forum`. Use for `init_test.rs`
/// (which tests init itself) and for tests that operate on raw event
/// storage without going through the policy layer.
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

// ---- Thread builders ----

pub fn make_thread(git: &GitOps, kind: ThreadKind, title: &str) -> String {
    create::create_thread(git, kind, title, None, "human/alice", &fixed_clock()).unwrap()
}

pub fn make_rfc(git: &GitOps) -> String {
    make_thread(git, ThreadKind::Rfc, "Test RFC")
}

pub fn make_dec(git: &GitOps) -> String {
    create::create_thread(
        git,
        ThreadKind::Dec,
        "Test DEC",
        Some(
            "## Context\nSome context\n## Decision\nUse Redis\n## Rationale\nFast\n## Impact\nNone",
        ),
        "human/alice",
        &fixed_clock(),
    )
    .unwrap()
}

pub fn make_task(git: &GitOps) -> String {
    make_thread(git, ThreadKind::Task, "Test TASK")
}

// ---- Raw events (used by tests that bypass create::create_thread) ----

/// Generate a deterministic legacy KIND-XXXXXXXX thread ID. Used by
/// tests that pre-write raw events to exercise the 1.x-on-disk path.
pub fn test_thread_id(kind: ThreadKind, seed: u8) -> String {
    id_alloc::alloc_thread_id_with_nonce(
        kind,
        "human/alice",
        "test",
        "2026-01-01T00:00:00Z",
        &[seed, seed, seed, seed, seed, seed, seed, seed],
    )
}

pub fn sample_create_event(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
    Event {
        event_id: "evt-0001".into(),
        thread_id: thread_id.into(),
        event_type: EventType::Create,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        actor: "human/alice".into(),
        title: Some(title.into()),
        kind: Some(kind),
        ..Event::default()
    }
}

pub fn sample_state_event(thread_id: &str, new_state: &str) -> Event {
    Event {
        event_id: "evt-0002".into(),
        thread_id: thread_id.into(),
        event_type: EventType::State,
        created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
        actor: "human/bob".into(),
        new_state: Some(new_state.into()),
        ..Event::default()
    }
}

// ---- Links / transitions ----

pub fn link_thread(git: &GitOps, from: &str, to: &str, rel: &str) {
    evidence::add_thread_link(git, from, to, rel, "human/alice", &fixed_clock()).unwrap();
}

/// Drive an RFC from `draft` through `proposed` to `under-review`
/// with no policy guards. Used to set up the typical
/// review→accepted gating tests.
pub fn move_rfc_to_under_review(git: &GitOps, thread_id: &str) {
    state_change::change_state(
        git,
        thread_id,
        "proposed",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
    state_change::change_state(
        git,
        thread_id,
        "under-review",
        &[],
        "human/alice",
        &fixed_clock(),
        &empty_policy(),
        state_change::StateChangeOptions::default(),
    )
    .unwrap();
}

/// Walk a thread to its terminal `done` state along the shortest
/// valid path. Lifecycles vary in how many intermediate states stand
/// between the initial state and `done`; the state machine guards
/// reject multi-hop calls, so this helper steps one at a time.
pub fn drive_to_done(git: &GitOps, policy: &Policy, thread_id: &str) {
    loop {
        let state = thread::replay_thread(git, thread_id).unwrap();
        if state.status == event::ThreadStatus::Done {
            break;
        }
        let lifecycle = state.lifecycle;
        let path = event::find_path(lifecycle, state.status.as_str(), "done")
            .unwrap_or_else(|| panic!("no path to done from {} for {:?}", state.status, lifecycle));
        let next = path.first().expect("path is empty but state != done");
        state_change::change_state(
            git,
            thread_id,
            next,
            &[],
            "human/alice",
            &fixed_clock(),
            policy,
            state_change::StateChangeOptions::default(),
        )
        .unwrap();
    }
}

// ---- Index ----

pub fn build_index(git: &GitOps, paths: &RepoPaths) {
    let db_path = paths.git_forum.join("index.db");
    reindex::run_reindex(git, &db_path).unwrap();
}

pub fn open_index(paths: &RepoPaths) -> rusqlite::Connection {
    let db_path = paths.git_forum.join("index.db");
    index::open_db(&db_path).unwrap()
}

// ---- Policy builders (SPEC-3.0 §3.2 category-keyed form) ----

pub fn empty_policy() -> Policy {
    Policy::default()
}

/// Build a single-category policy with the given guard rules at the
/// named transition. Mirrors the §3.2 inline-table form
/// `[categories.<C>.guards] "from->to" = [rules...]`.
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

/// Default policy used by `state_change_test` / `verify_test` to gate
/// the rfc category's review→done edge with `one_approval` +
/// `no_open_objections` (SPEC-3.0 §3.2).
pub fn policy_with_under_review_guards() -> Policy {
    make_category_guard_policy(
        "rfc",
        "review->done",
        vec![GuardRule::NoOpenObjections, GuardRule::OneApproval],
    )
}

/// task category gate covering the dec-on-task lifecycle path:
/// `record`-lifecycle DEC threads fold into category=task per §8.3,
/// and the legacy `proposed->accepted` transition normalizes to
/// `open->done` after `event::normalize_state_name`. Both the
/// `open->done` and `working->done` edges are gated for tests that
/// drive a DEC through either path.
pub fn dec_guard_policy() -> Policy {
    let mut policy = Policy::default();
    let mut cat = CategoryPolicy::default();
    cat.guards
        .insert("open->done".into(), vec![GuardRule::NoOpenObjections]);
    cat.guards
        .insert("working->done".into(), vec![GuardRule::NoOpenObjections]);
    policy.categories.insert("task".into(), cat);
    policy
}

/// task category gate: review→done requires no open actions.
pub fn task_guard_policy() -> Policy {
    make_category_guard_policy("task", "review->done", vec![GuardRule::NoOpenActions])
}

// ---- Back-compat shims for v2-shaped test helpers ----
//
// Several legacy integration tests (`tests/state_change_test.rs`,
// `tests/verify_test.rs`) assemble policies via `make_policy(vec![GuardEntry
// { on, requires, .. }])`. Pre-flight P1 deletes those types from
// `internal::policy`, but rewriting every test verbatim is out of scope
// for the policy rewrite commit. This shim accepts the legacy shape and
// rewrites it into the SPEC-3.0 §3.2 category-table form so the same
// tests keep exercising the runtime against the new parser.
//
// The legacy `on` field is parsed as one of:
//   "from->to"                          → applies to BOTH categories
//   "lifecycle=proposal : from->to"     → category=rfc
//   "lifecycle=execution : from->to"    → category=task
//   "lifecycle=record : from->to"       → category=task
//   "kind:from->to" (rfc/issue/dec/task)→ §8.3 mapping
// State names are normalized to SPEC-3.0 canonical via
// `event::normalize_state_name` so 1.x verbs (under-review, accepted,
// closed, ...) line up with category transitions.

/// Legacy v2 guard-entry shape, kept as a test-side adapter only.
#[derive(Debug, Clone, Default)]
pub struct GuardEntry {
    pub on: String,
    pub requires: Vec<GuardRule>,
}

/// Back-compat wrapper for the v2 `make_policy(Vec<GuardEntry>)` helper
/// used across legacy state-change / verify tests. Builds the equivalent
/// SPEC-3.0 §3.2 category-keyed `Policy`.
pub fn make_policy(entries: Vec<GuardEntry>) -> Policy {
    let mut policy = Policy::default();
    for entry in entries {
        let (categories, transition) = parse_legacy_on(&entry.on);
        for category in categories {
            let cat = policy.categories.entry(category.to_string()).or_default();
            cat.guards
                .entry(transition.clone())
                .or_default()
                .extend(entry.requires.clone());
        }
    }
    policy
}

fn parse_legacy_on(on: &str) -> (Vec<&'static str>, String) {
    use git_forum::internal::event::normalize_state_name;
    let (scope, transition_part) = match on.split_once(':') {
        Some((s, t)) => (s.trim(), t.trim()),
        None => ("", on.trim()),
    };
    let categories: Vec<&'static str> = if scope.is_empty() {
        vec!["rfc", "task"]
    } else if let Some(rest) = scope.strip_prefix("lifecycle=") {
        match rest.trim() {
            "proposal" => vec!["rfc"],
            "execution" | "record" => vec!["task"],
            _ => vec!["rfc", "task"],
        }
    } else {
        // `kind:from->to` legacy form.
        match scope {
            "rfc" => vec!["rfc"],
            "issue" | "task" | "dec" => vec!["task"],
            _ => vec!["rfc", "task"],
        }
    };
    let transition = if let Some((from, to)) = transition_part.split_once("->") {
        format!(
            "{}->{}",
            normalize_state_name(from.trim()),
            normalize_state_name(to.trim())
        )
    } else {
        transition_part.to_string()
    };
    (categories, transition)
}
