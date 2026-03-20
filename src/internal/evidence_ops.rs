use super::clock::Clock;
use super::error::ForumResult;
use super::event::{Event, EventType};
use super::evidence::{Evidence, EvidenceKind, Locator};
use super::git_ops::GitOps;

/// Add an evidence item to a thread via a Link event.
///
/// Preconditions: git is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Link event carrying the evidence is written to the thread.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn add_evidence(
    git: &GitOps,
    thread_id: &str,
    kind: EvidenceKind,
    ref_target: &str,
    locator: Option<Locator>,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let ref_target = canonicalize_evidence_ref(git, &kind, ref_target)?;
    let ev = Event::base(thread_id, EventType::Link, actor, clock).with_evidence(Evidence {
        evidence_id: String::new(),
        kind,
        ref_target,
        locator,
    });
    super::event::write_event(git, &ev)
}

fn canonicalize_evidence_ref(
    git: &GitOps,
    kind: &EvidenceKind,
    ref_target: &str,
) -> ForumResult<String> {
    match kind {
        EvidenceKind::Commit => git.resolve_commit(ref_target),
        _ => Ok(ref_target.to_string()),
    }
}

/// Add a link between two threads via a Link event.
///
/// Preconditions: git is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Link event with target and rel is written to the thread.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn add_thread_link(
    git: &GitOps,
    thread_id: &str,
    target_thread_id: &str,
    rel: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let ev = Event::base(thread_id, EventType::Link, actor, clock)
        .with_target_node_id(target_thread_id)
        .with_link_rel(rel);
    super::event::write_event(git, &ev)
}
