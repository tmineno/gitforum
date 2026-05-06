//! `git forum evidence add <ID> --kind <KIND> --ref <REF>` orchestration.
//!
//! task `1hg98odf`: appends rows to `evidence.toml`
//! directly via `snapshot::store::write_snapshot`. The legacy
//! `internal::evidence::add_evidence` event-write path is no longer
//! invoked here.

use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::evidence::{EvidenceKind, EvidenceRecord};
use crate::internal::id_alloc;
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::snapshot::{self, store::write_snapshot};
use crate::internal::thread;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};

pub fn run_evidence_add(
    thread_id: &str,
    kind: EvidenceKind,
    ref_targets: &[String],
    as_actor: Option<String>,
    force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    if ref_targets.is_empty() {
        return Err(ForumError::Config("--ref is required".into()));
    }
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, thread_id)?;
    let actor = resolve_actor(as_actor, &git);
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let state = thread::replay_thread(&git, &thread_id)?;
    let category = crate::internal::policy::category_for_state(&state);
    let violations = operation_check::check_evidence(&policy, category, state.status.as_str());
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let mut doc = snapshot::read_snapshot(&git, &thread_id)?;
    let now = clock.now();

    let mut added: Vec<String> = Vec::new();
    for ref_target in ref_targets {
        // SPEC-3.0 / parity with legacy `evidence::add_evidence`:
        // commit refs are canonicalized via `git rev-parse` so the
        // stored value is a 40-char SHA, not the user-supplied
        // `HEAD`/branch name (which would not survive history rewrite).
        let canonical_ref = match kind {
            EvidenceKind::Commit => git.resolve_commit(ref_target)?,
            _ => ref_target.clone(),
        };
        let id = id_alloc::alloc_bare_thread_id(&actor, &canonical_ref, &now.to_rfc3339());
        doc.evidence.entries.push(EvidenceRecord {
            id: id.clone(),
            kind: kind.clone(),
            ref_target: canonical_ref,
            created_at: now,
            created_by: actor.clone(),
        });
        added.push(id);
    }
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();

    write_snapshot(
        &git,
        &thread_id,
        &doc,
        &format!("evidence add ({})", added.len()),
    )?;
    for id in &added {
        println!("Evidence added ({})", &id[..id.len().min(8)]);
    }
    Ok(())
}
