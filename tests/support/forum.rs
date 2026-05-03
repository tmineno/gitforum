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
use git_forum::internal::policy::{GuardEntry, GuardRule, Policy};
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

// ---- Policy builders ----

pub fn empty_policy() -> Policy {
    Policy {
        guards: vec![],
        ..Default::default()
    }
}

pub fn make_policy(guards: Vec<GuardEntry>) -> Policy {
    let mut p = Policy {
        guards,
        ..Default::default()
    };
    let emitter = git_forum::internal::lint_emit::LintEmitter::new_capturing(None);
    git_forum::internal::legacy::v1::rewrite_legacy_policy(
        &mut p,
        &emitter,
        std::path::Path::new("policy.toml"),
    );
    p
}

/// Default policy used by `state_change_test` / `verify_test` to
/// gate the proposal-lifecycle review→done edge with a no-objection
/// + one-human-approval requirement (ADR-006: `AtLeastOneSummary`
///   removed in 2.0).
pub fn policy_with_under_review_guards() -> Policy {
    make_policy(vec![GuardEntry {
        on: "under-review->accepted".into(),
        requires: vec![GuardRule::NoOpenObjections, GuardRule::OneHumanApproval],
        ..Default::default()
    }])
}

pub fn dec_guard_policy() -> Policy {
    make_policy(vec![GuardEntry {
        on: "proposed->accepted".into(),
        requires: vec![GuardRule::NoOpenObjections],
        ..Default::default()
    }])
}

pub fn task_guard_policy() -> Policy {
    make_policy(vec![GuardEntry {
        on: "reviewing->closed".into(),
        requires: vec![GuardRule::NoOpenActions],
        ..Default::default()
    }])
}
