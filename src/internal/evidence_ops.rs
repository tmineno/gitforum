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
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Link,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: Some(Evidence {
            evidence_id: String::new(),
            kind,
            ref_target: ref_target.to_string(),
            locator,
        }),
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)
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
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Link,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: Some(target_thread_id.to_string()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: Some(rel.to_string()),
        run_label: None,
    };
    super::event::write_event(git, &ev)
}
