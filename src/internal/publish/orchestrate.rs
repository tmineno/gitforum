//! High-level orchestration for `git forum push` (RFC `fls856j3`).
//!
//! This module owns the "build the local side of a publish" step:
//!
//! 1. Partition every authoritative thread under `refs/forum/threads/*`
//!    by visibility.
//! 2. For each *public* thread, run the §4 exclusion pipeline and the
//!    §4.4 pre-publish lint, then write a parentless snapshot into
//!    `refs/forum/published/<id>` if the recomputed tree differs from
//!    the current one (tree-equivalence skip per §2).
//! 3. Identify withdrawal candidates: published refs whose
//!    authoritative thread is now private or absent (§7).
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
use crate::internal::snapshot;
use crate::internal::thread::Visibility;

/// Per-thread record of what the publisher did locally and what
/// remote action (if any) is staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlan {
    pub thread_id: String,
    pub action: PlannedAction,
    pub warnings: Vec<LintWarning>,
}

/// Planned remote action per thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedAction {
    /// New `refs/forum/published/<id>` was created locally; will be
    /// pushed to the remote.
    Created { commit_sha: String },
    /// Existing `refs/forum/published/<id>` was force-updated
    /// locally; will be pushed.
    Updated { commit_sha: String },
    /// Tree-equivalence match: no local change, no remote push needed
    /// for this thread.
    Skipped,
    /// Authoritative thread is private or absent but
    /// `refs/forum/published/<id>` exists locally — stage a remote
    /// deletion. Local ref is preserved until the remote ack
    /// arrives (RFC §7 preserve-then-retry).
    Withdraw,
}

/// Aggregate plan returned by [`build_plan`]. The CLI consumes this
/// to drive the push and to print the summary line.
#[derive(Debug, Clone, Default)]
pub struct PublishPlan {
    pub threads: Vec<ThreadPlan>,
}

impl PublishPlan {
    pub fn refspecs(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.threads {
            let r = refs::published_ref(&t.thread_id);
            match &t.action {
                PlannedAction::Created { .. } | PlannedAction::Updated { .. } => {
                    out.push(format!("+{r}:{r}"));
                }
                PlannedAction::Withdraw => {
                    out.push(format!(":{r}"));
                }
                PlannedAction::Skipped => {}
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

/// Build the local publish plan.
///
/// Preconditions: `git` points at a forum-initialized repository.
/// Postconditions: every public authoritative thread has had its
/// exclusion + lint applied; `refs/forum/published/<id>` is locally
/// up-to-date for changed public threads. Withdrawal candidates are
/// listed in the returned plan but **not** deleted locally yet (RFC
/// §7 preserve-then-retry).
/// Failure modes: any I/O error from snapshot read or tree build.
/// Side effects: writes blob/tree/commit objects and updates local
/// `refs/forum/published/*` for changed public threads.
pub fn build_plan(git: &GitOps) -> ForumResult<PublishPlan> {
    // The publisher cares only about *authoritative* threads. We
    // bypass the §5 read-protocol fallback here: a published-only
    // orphan must surface as a withdrawal candidate, not as an
    // authoritative public thread we'd try to re-publish (which
    // would fail to find the thread for the visibility check).
    let auth_ids: Vec<String> = git
        .list_refs(refs::THREADS_PREFIX)?
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    let private_ids = compute_private_set(git, &auth_ids)?;

    let mut threads: Vec<ThreadPlan> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Pass 1: every authoritative public thread becomes a
    // create/update/skip plan entry.
    for thread_id in &auth_ids {
        seen.insert(thread_id.clone());
        let mut doc = snapshot::read_snapshot(git, thread_id)?;
        if doc.snapshot.visibility != Visibility::Public {
            // Private/absent-visibility — never published. Withdrawal
            // is handled in pass 2 via the published-ref walk.
            continue;
        }

        // Run exclusion BEFORE lint: the lint scans the materialized
        // body/node text, which is unchanged by exclusion (per §4.3
        // "published as-authored"), but the structured-side filter
        // is done before write.
        exclusion::apply(&mut doc, &private_ids);
        let warnings = lint::scan(&doc, &private_ids);

        let outcome = commit::write_published(git, thread_id, &doc)?;
        let action = match outcome {
            WriteOutcome::Created { commit_sha } => PlannedAction::Created { commit_sha },
            WriteOutcome::Updated { commit_sha } => PlannedAction::Updated { commit_sha },
            WriteOutcome::Skipped => PlannedAction::Skipped,
        };

        threads.push(ThreadPlan {
            thread_id: thread_id.clone(),
            action,
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
            // above) or it is private and we need to withdraw the
            // published copy.
            continue;
        }
        // No authoritative: pure orphan → withdraw.
        threads.push(ThreadPlan {
            thread_id: pid.to_string(),
            action: PlannedAction::Withdraw,
            warnings: Vec::new(),
        });
    }

    // Pass 3: among the threads we already saw, mark the ones whose
    // visibility is private and which still have a published ref
    // locally as withdrawal candidates. Pass 1 skipped them; we
    // re-classify here.
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

    // Stable order: thread id ascending.
    threads.sort_by(|a, b| a.thread_id.cmp(&b.thread_id));

    Ok(PublishPlan { threads })
}

/// Walk every authoritative thread snapshot and return the set of
/// ids whose visibility is `Private`. Threads that fail to read are
/// treated as private (default-deny) — the caller's snapshot read in
/// pass 1 will surface the underlying error if it persists.
pub(crate) fn compute_private_set(
    git: &GitOps,
    auth_ids: &[String],
) -> ForumResult<HashSet<String>> {
    let mut out = HashSet::new();
    for id in auth_ids {
        let doc = match snapshot::read_snapshot(git, id) {
            Ok(d) => d,
            Err(_) => {
                // Unreadable thread: classify as private so the
                // exclusion/lint partition stays default-deny. The
                // caller's per-thread pass will turn the read error
                // into a hard failure when the thread itself is
                // visited.
                out.insert(id.clone());
                continue;
            }
        };
        if doc.snapshot.visibility != Visibility::Public {
            out.insert(id.clone());
        }
    }
    Ok(out)
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
    fn second_run_skips_when_unchanged() {
        let (_dir, git) = fresh_forum();
        make_thread(&git, "pub00000", Visibility::Public, "first\n");

        build_plan(&git).unwrap();
        let plan2 = build_plan(&git).unwrap();
        assert_eq!(plan2.threads.len(), 1);
        assert_eq!(plan2.threads[0].action, PlannedAction::Skipped);
        assert!(plan2.refspecs().is_empty());
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
        // pub00000 should be Skipped (unchanged); pub11111 Withdraw.
        let refs = plan.refspecs();
        assert_eq!(refs, vec![":refs/forum/published/pub11111"]);
    }
}
