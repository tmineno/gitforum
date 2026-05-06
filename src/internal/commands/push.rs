//! `git forum push` orchestration (RFC `fls856j3`).
//!
//! Drives the local publish plan from
//! [`super::super::publish::orchestrate`] and propagates the result
//! to a Git remote: creates/updates published refs and stages
//! deletions for withdrawn threads.
//!
//! Failure handling per RFC §7 ("preserve-then-retry"): when the
//! remote rejects a withdrawal, the local `refs/forum/published/<id>`
//! is preserved so a retry reattempts the deletion. When the remote
//! rejects a create/update, the local force-update has already
//! landed; the next push retries the publish. The summary line
//! breaks out failures separately (`L`) and the exit code is
//! non-zero whenever any remote operation failed, regardless of
//! `--strict`.

use std::process::Command as ProcCommand;

use super::super::error::{ForumError, ForumResult};
use super::super::publish::commit::delete_published;
use super::super::publish::orchestrate::{self, PlannedAction, PublishPlan};
use super::context::Context;
use super::shared::discover_repo_with_init_warning;

/// CLI args for `git forum push` per RFC §6.
pub struct PushArgs {
    pub remote: Option<String>,
    pub strict: bool,
}

/// Counts that drive the summary line and the exit code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushSummary {
    pub published_total: usize,
    pub created: usize,
    pub updated: usize,
    pub withdrawn: usize,
    pub failed: usize,
    pub warnings: usize,
}

impl PushSummary {
    pub fn render(&self) -> String {
        format!(
            "Published {} threads ({} new, {} updated, {} withdrawn, {} failed)",
            self.published_total, self.created, self.updated, self.withdrawn, self.failed
        )
    }
}

/// Entry point per the `commands::<cmd>::run(args, &ctx)` convention.
pub fn run(args: PushArgs, _ctx: &Context) -> ForumResult<()> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let remote = args.remote.unwrap_or_else(|| "origin".to_string());

    // Phase 1: build the local plan. This applies exclusion + lint +
    // tree-equivalence write to every public thread and identifies
    // withdrawal candidates.
    let plan = orchestrate::build_plan(&git)?;

    // Phase 2: emit lint warnings to stderr in document-source order.
    for tp in &plan.threads {
        for w in &tp.warnings {
            eprintln!("warning: {}", w.render());
        }
    }

    let refspecs = plan.refspecs();
    let mut summary = summary_from_plan(&plan);

    // Nothing to push? Print the summary and return.
    if refspecs.is_empty() {
        if summary.warnings > 0 && args.strict {
            return Err(ForumError::Repo(format!(
                "{} pre-publish warning(s); --strict requested non-zero exit",
                summary.warnings
            )));
        }
        println!("{}", summary.render());
        return Ok(());
    }

    // Phase 3: push.
    let push_outcome = run_git_push(git.root(), &remote, &refspecs);
    match push_outcome {
        Ok(_) => {
            // Phase 4: on push success, delete local refs for any
            // withdrawal we staged (preserve-then-retry rule:
            // local delete only after remote ack).
            for wid in plan.withdrawal_ids() {
                delete_published(&git, wid)?;
            }
            if summary.warnings > 0 && args.strict {
                println!("{}", summary.render());
                return Err(ForumError::Repo(format!(
                    "{} pre-publish warning(s); --strict requested non-zero exit",
                    summary.warnings
                )));
            }
            println!("{}", summary.render());
        }
        Err(msg) => {
            // The remote refused at least one ref update. We do NOT
            // know which refs the server accepted from a single
            // string; conservatively treat the whole batch as
            // failed for the count, leave local state alone, and
            // exit non-zero.
            //
            // A future iteration can run `git push --porcelain` and
            // parse per-ref outcomes to be more precise.
            summary.failed = refspecs.len();
            // Reset the per-action counts so the summary doesn't
            // claim work the remote rejected.
            summary.created = 0;
            summary.updated = 0;
            summary.withdrawn = 0;
            println!("{}", summary.render());
            return Err(ForumError::Git(format!(
                "git push to {remote} failed: {msg}\n  hint: local published/* refs preserved; rerun once the remote accepts these refspecs"
            )));
        }
    }

    Ok(())
}

fn summary_from_plan(plan: &PublishPlan) -> PushSummary {
    let mut s = PushSummary::default();
    for t in &plan.threads {
        match &t.action {
            PlannedAction::Created { .. } => s.created += 1,
            PlannedAction::Updated { .. } => s.updated += 1,
            PlannedAction::Withdraw => s.withdrawn += 1,
            PlannedAction::Skipped => {}
        }
        s.warnings += t.warnings.len();
    }
    s.published_total = plan
        .threads
        .iter()
        .filter(|t| {
            matches!(
                t.action,
                PlannedAction::Created { .. } | PlannedAction::Updated { .. }
            )
        })
        .count();
    s
}

/// Run `git push <remote> <refspec>...` from `repo_root`. Returns
/// `Err(stderr)` on non-zero exit.
fn run_git_push(
    repo_root: &std::path::Path,
    remote: &str,
    refspecs: &[String],
) -> Result<(), String> {
    let mut cmd = ProcCommand::new("git");
    cmd.arg("push").arg(remote);
    for r in refspecs {
        cmd.arg(r);
    }
    // Strip git env vars that would override `current_dir` — same
    // posture as `GitOps::commit_tree`. Pre-commit-driven tests
    // export GIT_DIR pointing at the parent repo; without this,
    // the push retargets the wrong repo.
    let output = cmd
        .current_dir(repo_root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .output()
        .map_err(|e| format!("spawn git push: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_render_includes_all_counts() {
        let s = PushSummary {
            published_total: 3,
            created: 2,
            updated: 1,
            withdrawn: 1,
            failed: 0,
            warnings: 4,
        };
        assert_eq!(
            s.render(),
            "Published 3 threads (2 new, 1 updated, 1 withdrawn, 0 failed)"
        );
    }
}
