//! Fixture repositories for the TUI UX suites (doc/spec/TUI-UX-TESTING.md).
//!
//! Built once per test through the TUI's own snapshot writers with a
//! StepClock, then copied for every case so cases never share state.

use std::path::{Path, PathBuf};

use chrono::TimeZone;
use tempfile::TempDir;

use crate::internal::clock::{Clock, StepClock};
use crate::internal::config::RepoPaths;
use crate::internal::git_ops::GitOps;
use crate::internal::node::{NodeKind, NodeStatus};
use crate::internal::snapshot::{self, list as snapshot_list};

use super::state::{snapshot_append_link, snapshot_append_node, snapshot_create_thread};

pub(super) const ACTOR: &str = "human/alice";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fixture {
    Full,
    Empty,
}

/// Fixture repositories, built once per test and copied for every case.
pub(super) struct Templates {
    full: (TempDir, GitOps, RepoPaths, PathBuf),
    empty: (TempDir, GitOps, RepoPaths, PathBuf),
}

impl Templates {
    pub(super) fn build() -> Self {
        let full = super::tests::setup_repo();
        build_full_fixture(&full.1);
        let empty = super::tests::setup_repo();
        Self { full, empty }
    }

    pub(super) fn path(&self, fixture: Fixture) -> &Path {
        match fixture {
            Fixture::Full => self.full.0.path(),
            Fixture::Empty => self.empty.0.path(),
        }
    }
}

/// Threads in every lifecycle and several statuses, every node kind with
/// nested replies, links, a body longer than the viewport, a Markdown
/// table, CJK and emoji. Created through the TUI's own snapshot writers
/// with a StepClock, so ids and timestamps are the same on every run.
fn build_full_fixture(git: &GitOps) {
    let start = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let clock = StepClock::new(start, chrono::Duration::minutes(1));

    // More rows than an 80x24 list shows, so paging has somewhere to go.
    for i in 0..22 {
        snapshot_create_thread(
            git,
            &format!("Filler thread {i:02}"),
            None,
            "execution",
            &[],
            ACTOR,
            &clock,
        )
        .unwrap();
    }

    let long_body = (1..=80)
        .map(|i| format!("Line {i:02}: long enough that the body pane has to scroll."))
        .collect::<Vec<_>>()
        .join("\n");
    let table = "| 項目 | value |\n|---|---|\n| 日本語 | 🚀 emoji |\n| wide | 表のセルが長い場合 |";

    let record = snapshot_create_thread(
        git,
        "決定: ID は @ なしで表示する",
        Some("短い本文。"),
        "record",
        &[],
        ACTOR,
        &clock,
    )
    .unwrap();
    let execution = snapshot_create_thread(
        git,
        "Crash on narrow terminals",
        Some("Steps:\n1. Open the TUI\n2. Shrink the window"),
        "execution",
        &["bug".to_string()],
        ACTOR,
        &clock,
    )
    .unwrap();
    let proposal = snapshot_create_thread(
        git,
        "キャッシュ方針の提案 🚀",
        Some(&format!("## Goal\n\n{table}\n\n{long_body}")),
        "proposal",
        &[],
        ACTOR,
        &clock,
    )
    .unwrap();

    let node = |thread: &str, kind: NodeKind, body: &str| {
        snapshot_append_node(git, thread, kind, body, ACTOR, &clock).unwrap()
    };
    node(&execution, NodeKind::Action, &long_body);
    let objection = node(&proposal, NodeKind::Objection, "Benchmarks are missing.");
    let reply = node(&proposal, NodeKind::Comment, "ベンチマークを追加します。");
    let nested = node(&proposal, NodeKind::Comment, "Nested reply 🚀");
    let action = node(&proposal, NodeKind::Action, "Add a benchmark");
    let retracted = node(&proposal, NodeKind::Comment, "Withdrawn remark");
    node(&proposal, NodeKind::Approval, "LGTM");

    snapshot_append_link(git, &execution, &proposal, "implements", ACTOR, &clock).unwrap();
    snapshot_append_link(git, &record, &proposal, "relates-to", ACTOR, &clock).unwrap();

    // Statuses and reply nesting have no TUI writer; set them directly. Each
    // edit bumps updated_at, so the proposal (edited last, the thread with
    // nodes) is the first row of the default newest-first list.
    let set = |id: &str, edit: &dyn Fn(&mut snapshot::ThreadDocument)| {
        let mut doc = snapshot::read_snapshot(git, id).unwrap();
        edit(&mut doc);
        doc.snapshot.updated_at = clock.now();
        snapshot::store::write_snapshot(git, id, &doc, "fixture: statuses and replies").unwrap();
    };
    set(&record, &|doc| doc.snapshot.status = "done".into());
    set(&execution, &|doc| doc.snapshot.status = "working".into());
    set(&proposal, &|doc| {
        doc.snapshot.status = "review".into();
        for n in &mut doc.nodes {
            if n.record.id == reply {
                n.record.reply_to = Some(objection.clone());
            } else if n.record.id == nested {
                n.record.reply_to = Some(reply.clone());
            } else if n.record.id == action {
                n.record.status = NodeStatus::Resolved;
            } else if n.record.id == retracted {
                n.record.status = NodeStatus::Retracted;
            }
        }
    });
}

/// A list row whose thread no longer exists: another process removed it
/// after the TUI loaded its list, and the list has not been refreshed.
/// Opening it is the error a user can reach from a healthy repository.
/// Oldest `updated_at`, so it is the last row of the default list.
pub(super) fn stale_row() -> snapshot_list::ThreadRow {
    snapshot_list::ThreadRow {
        id: "zzzzzzzz".into(),
        kind: "task".into(),
        lifecycle: "execution".into(),
        lifecycle_explicit: true,
        tags: Vec::new(),
        status: "open".into(),
        title: "Removed by another process".into(),
        body: None,
        branch: None,
        created_at: "2000-01-01T00:00:00Z".into(),
        created_by: ACTOR.into(),
        updated_at: "2000-01-01T00:00:00Z".into(),
        visibility: crate::internal::thread::Visibility::Private,
        from_published: false,
    }
}

/// Copy a fixture repository (worktree and `.git`) into `to`.
pub(super) fn copy_tree(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry.unwrap();
        let dest = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}
