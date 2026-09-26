//! Fixture repositories for the TUI UX suites (doc/spec/TUI-UX-TESTING.md).
//!
//! Built once per test through the TUI's own snapshot writers with a
//! StepClock, then copied for every case so cases never share state. The
//! thread list of each template is read once too: reading it costs a
//! `git` process per snapshot file, which made up most of the suites'
//! run time.

use std::path::{Path, PathBuf};

use chrono::TimeZone;
use tempfile::TempDir;

use crate::internal::clock::{Clock, StepClock};
use crate::internal::config::RepoPaths;
use crate::internal::git_ops::GitOps;
use crate::internal::id::TEST_NONCE;
use crate::internal::node::{NodeKind, NodeStatus};
use crate::internal::refs::{PUBLISHED_PREFIX, THREADS_PREFIX};
use crate::internal::snapshot::list::ThreadRow;
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
    full_listing: Listing,
    empty_listing: Listing,
}

impl Templates {
    pub(super) fn build() -> Self {
        let full = super::tests::setup_repo();
        build_full_fixture(&full.1);
        let empty = super::tests::setup_repo();
        let full_listing = Listing::read(&full.1);
        let empty_listing = Listing::read(&empty.1);
        Self {
            full,
            empty,
            full_listing,
            empty_listing,
        }
    }

    pub(super) fn path(&self, fixture: Fixture) -> &Path {
        match fixture {
            Fixture::Full => self.full.0.path(),
            Fixture::Empty => self.empty.0.path(),
        }
    }

    /// The thread list of `fixture`, as `list_threads` reads it on a
    /// fresh copy.
    pub(super) fn listing(&self, fixture: Fixture) -> &Listing {
        match fixture {
            Fixture::Full => &self.full_listing,
            Fixture::Empty => &self.empty_listing,
        }
    }
}

/// `list_threads` of a template, with the thread refs it was read at.
#[derive(Clone)]
pub(super) struct Listing {
    tips: Vec<(String, String)>,
    rows: Vec<ThreadRow>,
}

impl Listing {
    fn read(git: &GitOps) -> Self {
        Self {
            tips: thread_tips(git),
            rows: snapshot_list::list_threads(git).unwrap(),
        }
    }

    /// The rows `list_threads` returns on a fresh copy of the template.
    /// A copy has the template's refs and objects, so it lists the same.
    pub(super) fn rows(&self) -> Vec<ThreadRow> {
        self.rows.clone()
    }

    /// `list_threads` on a copy of the template. While every thread ref
    /// still points where it did in the template, the rows are the
    /// template's, since `list_threads` reads nothing but the snapshots
    /// at those tips; otherwise the snapshots are read again.
    pub(super) fn list_threads(&self, git: &GitOps) -> Vec<ThreadRow> {
        if thread_tips(git) == self.tips {
            self.rows()
        } else {
            snapshot_list::list_threads(git).unwrap()
        }
    }
}

/// Every ref `list_threads` reads, with its tip.
fn thread_tips(git: &GitOps) -> Vec<(String, String)> {
    let mut tips = git.list_refs_with_shas(THREADS_PREFIX).unwrap();
    tips.extend(git.list_refs_with_shas(PUBLISHED_PREFIX).unwrap());
    tips
}

/// Threads in every lifecycle and several statuses, every node kind with
/// nested replies, links, a body longer than the viewport, a Markdown
/// table, CJK and emoji. Created through the TUI's own snapshot writers
/// with a StepClock and a counting id nonce, so ids and timestamps are the
/// same on every run.
fn build_full_fixture(git: &GitOps) {
    struct FixedIds;
    impl Drop for FixedIds {
        fn drop(&mut self) {
            TEST_NONCE.with(|c| c.set(None));
        }
    }
    TEST_NONCE.with(|c| c.set(Some(1)));
    let _fixed_ids = FixedIds;

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

/// Write a fixture repository for the PTY driver `scripts/tui-ux/drive.py`,
/// so its runs see the same ids and dates as the suites. Ignored in normal
/// runs; the driver runs it with `--ignored`, `TUI_UX_EXPORT_FIXTURE`
/// (`full` or `empty`) and `TUI_UX_EXPORT_DIR` (a path that must not exist).
#[test]
#[ignore = "run by scripts/tui-ux/drive.py"]
fn export_fixture() {
    let fixture = match std::env::var("TUI_UX_EXPORT_FIXTURE").as_deref() {
        Ok("full") => Fixture::Full,
        Ok("empty") => Fixture::Empty,
        other => panic!("TUI_UX_EXPORT_FIXTURE must be full or empty, got {other:?}"),
    };
    let dir = PathBuf::from(std::env::var("TUI_UX_EXPORT_DIR").expect("TUI_UX_EXPORT_DIR"));
    assert!(!dir.exists(), "{} already exists", dir.display());
    copy_tree(Templates::build().path(fixture), &dir);
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

/// The template's listing is what `list_threads` reads on a copy, both
/// while the copy is unchanged and after a thread is created on it.
#[test]
fn listing_matches_list_threads_on_a_copy() {
    let templates = Templates::build();
    let dir = TempDir::new().unwrap();
    copy_tree(templates.path(Fixture::Full), dir.path());
    let git = GitOps::new(dir.path().to_path_buf());
    let listing = templates.listing(Fixture::Full);
    let read = |git: &GitOps| format!("{:?}", snapshot_list::list_threads(git).unwrap());
    assert_eq!(format!("{:?}", listing.rows()), read(&git));
    assert_eq!(format!("{:?}", listing.list_threads(&git)), read(&git));

    let start = chrono::Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let clock = StepClock::new(start, chrono::Duration::minutes(1));
    snapshot_create_thread(
        &git,
        "Made on the copy",
        None,
        "execution",
        &[],
        ACTOR,
        &clock,
    )
    .unwrap();
    let after = listing.list_threads(&git);
    assert!(after.iter().any(|t| t.title == "Made on the copy"));
    assert_eq!(format!("{after:?}"), read(&git));
}
