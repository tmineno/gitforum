//! High-level orchestration for `git forum push` (RFC `fls856j3`).
//!
//! Split into two phases so the CLI can fail-closed on `--strict`
//! before any local writes happen:
//!
//! 1. [`lint_pass`] — read-only. Partition every authoritative thread
//!    under `refs/forum/threads/*` by visibility, apply the §5.5
//!    exclusion pipeline in memory, run the §5.5 pre-publish lint,
//!    and identify withdrawal candidates (§5.6). No blob/tree/commit
//!    objects are written, no refs change. Returns a [`PublishPlan`]
//!    whose create/update entries carry post-exclusion documents
//!    instead of commit shas, plus the lint warnings.
//! 2. [`commit_plan`] — writes the parentless snapshot commits and
//!    advances `refs/forum/published/*` for plan entries whose tree
//!    differs from the current published tree (tree-equivalence skip
//!    per §5.5). Withdrawals stay as plan entries — local refs are
//!    only deleted by [`commands::push`] after the remote
//!    acknowledges the deletion.
//!
//! Network I/O — the actual `git push` to the remote — happens in the
//! caller (`commands::push`). Splitting the local plan from the
//! transport keeps the orchestration unit-testable against a bare
//! repo.

use std::collections::HashSet;

use crate::internal::error::ForumResult;
use crate::internal::git_ops::GitOps;
use crate::internal::publish::commit::{self, WriteOutcome};
use crate::internal::publish::exclusion;
use crate::internal::publish::lint::{self, LintWarning};
use crate::internal::refs;
use crate::internal::snapshot::{self, ThreadDocument};
use crate::internal::thread::Visibility;

/// Per-thread record of the publisher's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlan {
    pub thread_id: String,
    pub action: PlannedAction,
    pub warnings: Vec<LintWarning>,
}

/// Planned action per thread. After [`lint_pass`] the create/update
/// entries carry the post-exclusion document (`Publish { doc }`); after
/// [`commit_plan`] runs they are rewritten to the realised outcome
/// (`Created` / `Updated` / `Skipped`) so the CLI can build its summary
/// line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedAction {
    /// Pre-write: this public thread will be materialised into the
    /// published namespace from `doc`. After [`commit_plan`] runs,
    /// the entry is rewritten to `Created`, `Updated`, or `Skipped`.
    Publish { doc: Box<ThreadDocument> },
    /// `refs/forum/published/<id>` was newly created locally.
    Created { commit_sha: String },
    /// `refs/forum/published/<id>` was force-updated locally.
    Updated { commit_sha: String },
    /// Tree-equivalence skip: the recomputed tree matched the current
    /// published tree, so no new commit was written.
    Skipped,
    /// Authoritative thread is private or absent but
    /// `refs/forum/published/<id>` exists locally — stage a remote
    /// deletion. Local ref is preserved until the remote ack
    /// arrives (RFC §5.6 preserve-then-retry).
    Withdraw,
}

/// Aggregate plan returned by [`lint_pass`] / [`commit_plan`]. The CLI
/// consumes this to drive the push and to print the summary line.
#[derive(Debug, Clone, Default)]
pub struct PublishPlan {
    pub threads: Vec<ThreadPlan>,
}

impl PublishPlan {
    /// Push refspecs for every plan entry that touches a remote ref.
    ///
    /// Create/update/skipped entries all emit `+REF:REF` so a local
    /// ref that is locally up-to-date but failed to push on a prior
    /// run is retried — without this, [`commit_plan`] tree-equivalence
    /// skips would mark the entry `Skipped` and the next push would
    /// stage no refspec, leaving the remote behind forever (the bug
    /// the preserve-then-retry rule already addresses for
    /// withdrawals). Wire cost is ~zero when local and remote SHAs
    /// match: git push negotiates ref states and sends no objects.
    pub fn refspecs(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.threads {
            let r = refs::published_ref(&t.thread_id);
            match &t.action {
                PlannedAction::Created { .. }
                | PlannedAction::Updated { .. }
                | PlannedAction::Skipped => {
                    out.push(format!("+{r}:{r}"));
                }
                PlannedAction::Withdraw => {
                    out.push(format!(":{r}"));
                }
                PlannedAction::Publish { .. } => {
                    // Pre-write entry — refspecs are only meaningful
                    // after commit_plan has run.
                }
            }
        }
        out
    }

    /// Threads staged for remote-deletion. The push consumer needs
    /// this list separately so it can decide which local refs to
    /// delete after the remote acknowledges.
    pub fn withdrawal_ids(&self) -> Vec<&str> {
        self.threads
            .iter()
            .filter(|t| matches!(t.action, PlannedAction::Withdraw))
            .map(|t| t.thread_id.as_str())
            .collect()
    }

    pub fn total_warnings(&self) -> usize {
        self.threads.iter().map(|t| t.warnings.len()).sum()
    }
}

/// Phase 1: read-only lint pass.
///
/// Walks `refs/forum/threads/*`, applies the §5.5 exclusion pipeline
/// to every public thread in memory, runs the §5.5 pre-publish lint,
/// and identifies withdrawal candidates from `refs/forum/published/*`
/// orphans and from public→private flips. **Writes nothing** to the
/// repository — no blob/tree/commit objects, no ref updates.
///
/// Preconditions: `git` points at a forum-initialized repository.
/// Postconditions: returned plan entries are either `Publish { doc }`
/// (post-exclusion document ready for [`commit_plan`]) or `Withdraw`.
/// Failure modes: any I/O error from snapshot read.
/// Side effects: none — this phase is pure read.
pub fn lint_pass(git: &GitOps) -> ForumResult<PublishPlan> {
    // The publisher cares only about *authoritative* threads. We
    // bypass the §5.1 read-protocol fallback here: a published-only
    // orphan must surface as a withdrawal candidate, not as an
    // authoritative public thread we'd try to re-publish (which
    // would fail to find the thread for the visibility check).
    let auth_ids: Vec<String> = git
        .list_refs(refs::THREADS_PREFIX)?
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    let (public_ids, private_ids) = compute_visibility_partition(git, &auth_ids)?;

    let mut threads: Vec<ThreadPlan> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Pass 1: every authoritative public thread becomes a
    // Publish plan entry. Exclusion runs against `public_ids` so
    // that links/evidence pointing at non-public targets (private,
    // unknown, or absent) are dropped — RFC §5.5 calls for "drop
    // entries whose target is non-public," not just "drop entries
    // whose target is known-private."
    for thread_id in &auth_ids {
        seen.insert(thread_id.clone());
        let mut doc = snapshot::read_snapshot(git, thread_id)?;
        if doc.snapshot.visibility != Visibility::Public {
            // Private/absent-visibility — never published. Withdrawal
            // is handled in pass 2 via the published-ref walk.
            continue;
        }

        // Run exclusion BEFORE lint: the lint scans the materialized
        // body/node text, which is unchanged by exclusion (per §5.5
        // "published as-authored"), but the structured-side filter
        // happens first.
        exclusion::apply(&mut doc, &public_ids);
        let warnings = lint::scan(&doc, &private_ids);

        threads.push(ThreadPlan {
            thread_id: thread_id.clone(),
            action: PlannedAction::Publish { doc: Box::new(doc) },
            warnings,
        });
    }

    // Pass 2: orphan published refs — published exists locally but
    // authoritative is now absent. Withdraw them.
    let published_refnames = git.list_refs(refs::PUBLISHED_PREFIX)?;
    for refname in published_refnames {
        let pid = match refs::thread_id_from_published_ref(&refname) {
            Some(id) => id,
            None => continue,
        };
        if seen.contains(pid) {
            // Authoritative exists; either it was public (handled
            // above) or it is private and we re-classify in pass 3.
            continue;
        }
        threads.push(ThreadPlan {
            thread_id: pid.to_string(),
            action: PlannedAction::Withdraw,
            warnings: Vec::new(),
        });
    }

    // Pass 3: authoritative is private but the published ref still
    // exists locally — stage a withdrawal.
    let private_with_published: Vec<String> = auth_ids
        .iter()
        .filter(|id| private_ids.contains(*id))
        .filter(|id| {
            git.resolve_ref(&refs::published_ref(id))
                .ok()
                .flatten()
                .is_some()
        })
        .cloned()
        .collect();
    for pid in private_with_published {
        threads.push(ThreadPlan {
            thread_id: pid,
            action: PlannedAction::Withdraw,
            warnings: Vec::new(),
        });
    }

    threads.sort_by(|a, b| a.thread_id.cmp(&b.thread_id));
    Ok(PublishPlan { threads })
}

/// Phase 2: commit the local writes.
///
/// For every `Publish { doc }` entry in `plan`, write a parentless
/// snapshot commit and force-update `refs/forum/published/<id>` (or
/// skip when the recomputed tree already matches the current
/// published tree). The plan entry's action is rewritten in place
/// to `Created` / `Updated` / `Skipped`. `Withdraw` entries are
/// untouched — local published refs are only deleted in
/// `commands::push` after the remote acknowledges.
///
/// Preconditions: `plan` came from [`lint_pass`] (entries are
/// either `Publish` or `Withdraw`).
/// Postconditions: every `Publish` entry has been transformed into a
/// realised outcome and `refs/forum/published/*` is locally
/// up-to-date for changed public threads.
/// Failure modes: any I/O error from tree/commit/ref write.
/// Side effects: writes blob/tree/commit objects and updates local
/// `refs/forum/published/*` for changed public threads.
pub fn commit_plan(git: &GitOps, plan: &mut PublishPlan) -> ForumResult<()> {
    for entry in &mut plan.threads {
        let doc = match &entry.action {
            PlannedAction::Publish { doc } => doc.clone(),
            _ => continue,
        };
        let outcome = commit::write_published(git, &entry.thread_id, &doc)?;
        entry.action = match outcome {
            WriteOutcome::Created { commit_sha } => PlannedAction::Created { commit_sha },
            WriteOutcome::Updated { commit_sha } => PlannedAction::Updated { commit_sha },
            WriteOutcome::Skipped => PlannedAction::Skipped,
        };
    }
    Ok(())
}

/// Compose [`lint_pass`] + [`commit_plan`] into a single call. Kept
/// for callers that don't need to short-circuit on warnings (e.g.
/// most tests).
pub fn build_plan(git: &GitOps) -> ForumResult<PublishPlan> {
    let mut plan = lint_pass(git)?;
    commit_plan(git, &mut plan)?;
    Ok(plan)
}

/// Walk every authoritative thread snapshot and partition the ids by
/// visibility. Threads that fail to read are treated as private
/// (default-deny) — the caller's per-thread snapshot read in
/// [`lint_pass`] will surface the underlying error if it persists.
///
/// Returns `(public_ids, private_ids)`. The two sets are disjoint and
/// cover exactly `auth_ids`.
pub(crate) fn compute_visibility_partition(
    git: &GitOps,
    auth_ids: &[String],
) -> ForumResult<(HashSet<String>, HashSet<String>)> {
    let mut public = HashSet::new();
    let mut private = HashSet::new();
    for id in auth_ids {
        let doc = match snapshot::read_snapshot(git, id) {
            Ok(d) => d,
            Err(_) => {
                // Unreadable thread: default-deny so the exclusion
                // partition stays safe. The caller's per-thread pass
                // will turn the read error into a hard failure when
                // the thread itself is visited.
                private.insert(id.clone());
                continue;
            }
        };
        if doc.snapshot.visibility == Visibility::Public {
            public.insert(id.clone());
        } else {
            private.insert(id.clone());
        }
    }
    Ok((public, private))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    use crate::internal::config::RepoPaths;
    use crate::internal::git_ops::GitOps;
    use crate::internal::init;
    use crate::internal::snapshot::store::write_snapshot;
    use crate::internal::snapshot::ThreadDocument;
    use crate::internal::thread::{ThreadSnapshot, Visibility};

    fn fresh_forum() -> (TempDir, GitOps) {
        let dir = TempDir::new().unwrap();
        let git = GitOps::new(dir.path().to_path_buf());
        git.run(&["init", "-q"]).unwrap();
        git.run(&["config", "user.name", "tester"]).unwrap();
        git.run(&["config", "user.email", "t@example"]).unwrap();
        let paths = RepoPaths::from_repo_root(dir.path());
        init::init_forum(&paths).unwrap();
        (dir, git)
    }

    fn epoch() -> chrono::DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn make_thread(git: &GitOps, id: &str, visibility: Visibility, body: &str) {
        let doc = ThreadDocument::new(ThreadSnapshot {
            schema_version: 3,
            id: id.into(),
            title: "T".into(),
            category: "rfc".into(),
            status: "draft".into(),
            tags: vec![],
            created_at: epoch(),
            created_by: "human/alice".into(),
            updated_at: epoch(),
            updated_by: "human/alice".into(),
            branch: None,
            supersedes: vec![],
            visibility,
        });
        let mut doc = doc;
        doc.body = Some(body.into());
        write_snapshot(git, id, &doc, "create").unwrap();
    }

    #[test]
    fn first_run_creates_published_for_public_only() {
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");
        make_thread(&git, "priv0000", Visibility::Private, "secret\n");

        let plan = build_plan(&git).unwrap();
        let ids: Vec<&str> = plan.threads.iter().map(|t| t.thread_id.as_str()).collect();
        // priv0000 has no published ref and is private → not in the
        // plan. pub00000 is created.
        assert_eq!(ids, vec!["pub00000"]);
        assert!(matches!(
            plan.threads[0].action,
            PlannedAction::Created { .. }
        ));

        // Local published ref exists.
        assert!(git
            .resolve_ref("refs/forum/published/pub00000")
            .unwrap()
            .is_some());
        assert!(git
            .resolve_ref("refs/forum/published/priv0000")
            .unwrap()
            .is_none());
    }

    #[test]
    fn second_run_skips_locally_but_still_emits_refspec() {
        // Tree-equivalence skip means no new local commit, but the
        // refspec MUST still be present so a previously failed remote
        // push can be retried.
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");

        build_plan(&git).unwrap();
        let plan2 = build_plan(&git).unwrap();
        assert_eq!(plan2.threads.len(), 1);
        assert_eq!(plan2.threads[0].action, PlannedAction::Skipped);
        assert_eq!(
            plan2.refspecs(),
            vec!["+refs/forum/published/pub00000:refs/forum/published/pub00000"]
        );
    }

    #[test]
    fn flipping_public_to_private_yields_withdraw() {
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");
        build_plan(&git).unwrap();
        // Flip visibility to private and re-plan.
        crate::internal::commands::visibility::set_visibility(
            &git,
            "pub00000",
            Visibility::Private,
            true, // force (no TTY in tests)
            "tester",
            &crate::internal::clock::SystemClock,
        )
        .unwrap();
        let plan = build_plan(&git).unwrap();
        assert_eq!(plan.threads.len(), 1);
        assert_eq!(plan.threads[0].action, PlannedAction::Withdraw);
        // Local published ref still present (preserve-then-retry).
        assert!(git
            .resolve_ref("refs/forum/published/pub00000")
            .unwrap()
            .is_some());
        // Refspec is the deletion form.
        assert_eq!(plan.refspecs(), vec![":refs/forum/published/pub00000"]);
    }

    #[test]
    fn orphan_published_ref_is_withdrawal_candidate() {
        // Published ref exists with no authoritative counterpart.
        // Build the situation manually: write a public thread,
        // publish it, then delete the authoritative thread ref.
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");
        build_plan(&git).unwrap();
        git.delete_ref("refs/forum/threads/pub00000").unwrap();

        let plan = build_plan(&git).unwrap();
        assert_eq!(plan.threads.len(), 1);
        assert_eq!(plan.threads[0].thread_id, "pub00000");
        assert_eq!(plan.threads[0].action, PlannedAction::Withdraw);
    }

    #[test]
    fn lint_warnings_propagate_to_plan() {
        let (_dir, git) = fresh_forum();
        // Public thread mentions a private thread by id.
        make_thread(&git, "priv1234", Visibility::Private, "secret\n");
        make_thread(
            &git,
            "pub00000",
            Visibility::Public,
            "blocked by @priv1234 right now",
        );

        let plan = build_plan(&git).unwrap();
        // Only the public thread is in the plan (private has no
        // published ref).
        assert_eq!(plan.threads.len(), 1);
        assert_eq!(plan.threads[0].thread_id, "pub00000");
        assert_eq!(plan.threads[0].warnings.len(), 1);
        assert_eq!(plan.threads[0].warnings[0].matched_id, "priv1234");
    }

    #[test]
    fn refspec_set_combines_pushes_and_deletes() {
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "a");
        make_thread(&git, "pub11111", Visibility::Public, "b");
        // Publish both.
        build_plan(&git).unwrap();
        // Withdraw pub11111.
        crate::internal::commands::visibility::set_visibility(
            &git,
            "pub11111",
            Visibility::Private,
            true,
            "tester",
            &crate::internal::clock::SystemClock,
        )
        .unwrap();

        let plan = build_plan(&git).unwrap();
        // pub00000 is Skipped (unchanged tree) but its refspec is
        // still emitted so a stale remote can catch up; pub11111 is
        // a Withdraw deletion refspec.
        let refs = plan.refspecs();
        assert_eq!(
            refs,
            vec![
                "+refs/forum/published/pub00000:refs/forum/published/pub00000",
                ":refs/forum/published/pub11111",
            ]
        );
    }

    #[test]
    fn lint_pass_does_not_write() {
        // Phase 1 is read-only: no published refs, no commits.
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");

        let plan = lint_pass(&git).unwrap();
        assert_eq!(plan.threads.len(), 1);
        assert!(matches!(
            plan.threads[0].action,
            PlannedAction::Publish { .. }
        ));
        // No published ref was created.
        assert!(git
            .resolve_ref("refs/forum/published/pub00000")
            .unwrap()
            .is_none());
    }

    #[test]
    fn commit_plan_realises_publish_entries() {
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");

        let mut plan = lint_pass(&git).unwrap();
        commit_plan(&git, &mut plan).unwrap();
        assert!(matches!(
            plan.threads[0].action,
            PlannedAction::Created { .. }
        ));
        assert!(git
            .resolve_ref("refs/forum/published/pub00000")
            .unwrap()
            .is_some());
    }

    #[test]
    fn exclusion_drops_links_to_unknown_targets() {
        // RFC §5.5: drop links/evidence whose target is *non-public*,
        // not just known-private. An unknown id (no auth ref locally)
        // must not survive into the published tree.
        use crate::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
        use crate::internal::snapshot::link::{Link, Links};

        let (_dir, git) = fresh_forum();
        // Public thread with one link to a public target, one link
        // to a private target, and one link to an unknown id.
        make_thread(&git, "pub11111", Visibility::Public, "other public");
        make_thread(&git, "priv1234", Visibility::Private, "secret");
        let mut doc = ThreadDocument::new(ThreadSnapshot {
            schema_version: 3,
            id: "pub00000".into(),
            title: "T".into(),
            category: "rfc".into(),
            status: "draft".into(),
            tags: vec![],
            created_at: epoch(),
            created_by: "human/alice".into(),
            updated_at: epoch(),
            updated_by: "human/alice".into(),
            branch: None,
            supersedes: vec![],
            visibility: Visibility::Public,
        });
        doc.links = Links {
            entries: vec![
                Link {
                    target: "pub11111".into(),
                    rel: "depends-on".into(),
                    created_at: epoch(),
                    created_by: "human/alice".into(),
                },
                Link {
                    target: "priv1234".into(),
                    rel: "blocks".into(),
                    created_at: epoch(),
                    created_by: "human/alice".into(),
                },
                Link {
                    target: "unknown1".into(),
                    rel: "relates-to".into(),
                    created_at: epoch(),
                    created_by: "human/alice".into(),
                },
            ],
        };
        doc.evidence = EvidenceFile {
            entries: vec![EvidenceRecord {
                id: "ev1".into(),
                kind: EvidenceKind::Thread,
                ref_target: "unknown2".into(),
                created_at: epoch(),
                created_by: "human/alice".into(),
            }],
        };
        write_snapshot(&git, "pub00000", &doc, "create").unwrap();

        let plan = lint_pass(&git).unwrap();
        let entry = plan
            .threads
            .iter()
            .find(|t| t.thread_id == "pub00000")
            .expect("pub00000 in plan");
        let materialised = match &entry.action {
            PlannedAction::Publish { doc } => doc,
            other => panic!("expected Publish, got {other:?}"),
        };
        let surviving_link_targets: Vec<&str> = materialised
            .links
            .entries
            .iter()
            .map(|l| l.target.as_str())
            .collect();
        // Only the known-public target survives.
        assert_eq!(surviving_link_targets, vec!["pub11111"]);
        assert!(materialised.evidence.entries.is_empty());
    }
}
