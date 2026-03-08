use chrono::{DateTime, Utc};

use super::error::{ForumError, ForumResult};
use super::event::{self, Event, EventType, ThreadKind};
use super::git_ops::GitOps;
use super::refs;

/// Materialized state of a thread, derived from event replay.
#[derive(Debug, Clone)]
pub struct ThreadState {
    pub id: String,
    pub kind: ThreadKind,
    pub title: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub events: Vec<Event>,
}

/// Replay events to reconstruct thread state.
///
/// Precondition: `events` is in chronological order; first must be `Create`.
pub fn replay(events: &[Event]) -> ForumResult<ThreadState> {
    let first = events
        .first()
        .ok_or_else(|| ForumError::StateMachine("no events to replay".into()))?;

    if first.event_type != EventType::Create {
        return Err(ForumError::StateMachine(
            "first event must be 'create'".into(),
        ));
    }

    let kind = first
        .kind
        .ok_or_else(|| ForumError::StateMachine("create event missing 'kind'".into()))?;
    let title = first
        .title
        .clone()
        .ok_or_else(|| ForumError::StateMachine("create event missing 'title'".into()))?;

    let mut state = ThreadState {
        id: first.thread_id.clone(),
        kind,
        title,
        status: kind.initial_status().to_string(),
        created_at: first.created_at,
        created_by: first.actor.clone(),
        events: vec![first.clone()],
    };

    for ev in &events[1..] {
        apply_event(&mut state, ev)?;
    }
    Ok(state)
}

fn apply_event(state: &mut ThreadState, event: &Event) -> ForumResult<()> {
    state.events.push(event.clone());
    if event.event_type == EventType::State {
        if let Some(ref new_state) = event.new_state {
            state.status.clone_from(new_state);
        }
    }
    // Other event types will be handled in later milestones.
    Ok(())
}

/// Load events from Git and replay to get thread state.
pub fn replay_thread(git: &GitOps, thread_id: &str) -> ForumResult<ThreadState> {
    let events = event::load_thread_events(git, thread_id)?;
    replay(&events)
}

/// List all thread IDs from Git refs.
pub fn list_thread_ids(git: &GitOps) -> ForumResult<Vec<String>> {
    let ref_names = git.list_refs(refs::THREADS_PREFIX)?;
    let mut ids: Vec<String> = ref_names
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_create(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
        Event {
            event_id: "evt-0001".into(),
            thread_id: thread_id.into(),
            event_type: EventType::Create,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: Some(title.into()),
            kind: Some(kind),
            body: None,
            node_type: None,
            target_node_id: None,
            new_state: None,
        }
    }

    fn make_state(thread_id: &str, new_state: &str) -> Event {
        Event {
            event_id: "evt-0002".into(),
            thread_id: thread_id.into(),
            event_type: EventType::State,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: None,
            kind: None,
            body: None,
            node_type: None,
            target_node_id: None,
            new_state: Some(new_state.into()),
        }
    }

    #[test]
    fn replay_single_create() {
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "Test RFC")];
        let state = replay(&events).unwrap();
        assert_eq!(state.id, "RFC-0001");
        assert_eq!(state.kind, ThreadKind::Rfc);
        assert_eq!(state.title, "Test RFC");
        assert_eq!(state.status, "draft");
        assert_eq!(state.created_by, "human/alice");
        assert_eq!(state.events.len(), 1);
    }

    #[test]
    fn replay_create_then_state() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "Test RFC"),
            make_state("RFC-0001", "proposed"),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, "proposed");
        assert_eq!(state.events.len(), 2);
    }

    #[test]
    fn replay_empty_events_fails() {
        let result = replay(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn replay_non_create_first_fails() {
        let events = vec![make_state("RFC-0001", "proposed")];
        let result = replay(&events);
        assert!(result.is_err());
    }

    #[test]
    fn replay_issue_initial_status() {
        let events = vec![make_create("ISSUE-0001", ThreadKind::Issue, "Bug")];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, "open");
    }

    #[test]
    fn replay_decision_initial_status() {
        let events = vec![make_create("DEC-0001", ThreadKind::Decision, "Choice")];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, "proposed");
    }
}
