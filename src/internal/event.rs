use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::approval::Approval;
use super::error::ForumResult;
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::refs;

/// Thread kinds supported by git-forum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadKind {
    Issue,
    Rfc,
    Decision,
}

impl ThreadKind {
    /// Initial state for a new thread of this kind.
    pub fn initial_status(self) -> &'static str {
        match self {
            Self::Issue => "open",
            Self::Rfc => "draft",
            Self::Decision => "proposed",
        }
    }

    /// Display ID prefix (e.g. "ISSUE", "RFC", "DEC").
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Issue => "ISSUE",
            Self::Rfc => "RFC",
            Self::Decision => "DEC",
        }
    }
}

impl std::fmt::Display for ThreadKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue => write!(f, "issue"),
            Self::Rfc => write!(f, "rfc"),
            Self::Decision => write!(f, "decision"),
        }
    }
}

/// Event types as defined in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Create,
    Edit,
    Retract,
    Say,
    Link,
    Unlink,
    State,
    Assign,
    Decision,
    Resolve,
    Reopen,
    Spawn,
    Result,
    Verify,
    Merge,
    Close,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Create => "create",
            Self::Edit => "edit",
            Self::Retract => "retract",
            Self::Say => "say",
            Self::Link => "link",
            Self::Unlink => "unlink",
            Self::State => "state",
            Self::Assign => "assign",
            Self::Decision => "decision",
            Self::Resolve => "resolve",
            Self::Reopen => "reopen",
            Self::Spawn => "spawn",
            Self::Result => "result",
            Self::Verify => "verify",
            Self::Merge => "merge",
            Self::Close => "close",
        };
        f.write_str(s)
    }
}

/// Node types for structured discussion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Claim,
    Question,
    Objection,
    Alternative,
    Evidence,
    Summary,
    Decision,
    Action,
    Risk,
    Assumption,
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Claim => "claim",
            Self::Question => "question",
            Self::Objection => "objection",
            Self::Alternative => "alternative",
            Self::Evidence => "evidence",
            Self::Summary => "summary",
            Self::Decision => "decision",
            Self::Action => "action",
            Self::Risk => "risk",
            Self::Assumption => "assumption",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for NodeType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claim" => Ok(Self::Claim),
            "question" => Ok(Self::Question),
            "objection" => Ok(Self::Objection),
            "alternative" => Ok(Self::Alternative),
            "evidence" => Ok(Self::Evidence),
            "summary" => Ok(Self::Summary),
            "decision" => Ok(Self::Decision),
            "action" => Ok(Self::Action),
            "risk" => Ok(Self::Risk),
            "assumption" => Ok(Self::Assumption),
            _ => Err(format!("unknown node type '{s}'; valid types: claim, question, objection, alternative, evidence, summary, decision, action, risk, assumption")),
        }
    }
}

/// An immutable event in a thread's history.
///
/// Stored as `event.json` inside each Git commit's tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default, skip_serializing)]
    pub event_id: String,
    pub thread_id: String,
    pub event_type: EventType,
    pub created_at: DateTime<Utc>,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_rev: Option<String>,
    #[serde(default)]
    pub parents: Vec<String>,
    // -- Conditional fields (presence depends on event_type) --
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ThreadKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_type: Option<NodeType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_state: Option<String>,
    /// Approvals attached to State events (recorded sign-offs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<Approval>,
    /// Evidence attached to Link events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    /// Relation type for Link events (e.g. `"implements"`, `"relates-to"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_rel: Option<String>,
    /// Run label recorded by Spawn events (e.g. `"RUN-0001"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_label: Option<String>,
}

/// Write an event as a Git commit and update the thread ref.
///
/// Returns the new commit SHA.
pub fn write_event(git: &GitOps, event: &Event) -> ForumResult<String> {
    let json = serde_json::to_string_pretty(event)?;

    // Create blob → tree → commit
    let blob_sha = git.hash_object(json.as_bytes())?;
    let tree_sha = git.mktree_single("event.json", &blob_sha)?;

    // Find parent from existing thread ref
    let ref_name = refs::thread_ref(&event.thread_id);
    let parent_sha = git.resolve_ref(&ref_name)?;
    let parents: Vec<&str> = parent_sha.iter().map(|s| s.as_str()).collect();

    let message = format!("[git-forum] {} {}", event.event_type, event.thread_id);
    let commit_sha = git.commit_tree(&tree_sha, &parents, &message)?;

    git.update_ref(&ref_name, &commit_sha)?;
    Ok(commit_sha)
}

/// Read an event from a commit SHA.
pub fn read_event(git: &GitOps, commit_sha: &str) -> ForumResult<Event> {
    let json = git.show_file(commit_sha, "event.json")?;
    let mut event: Event = serde_json::from_str(&json)?;
    event.event_id = commit_sha.to_string();
    Ok(event)
}

/// Load all events for a thread in chronological order (oldest first).
pub fn load_thread_events(git: &GitOps, thread_id: &str) -> ForumResult<Vec<Event>> {
    let ref_name = refs::thread_ref(thread_id);
    let shas = git.rev_list(&ref_name)?; // newest first
    let mut events = Vec::with_capacity(shas.len());
    for sha in &shas {
        events.push(read_event(git, sha)?);
    }
    events.reverse(); // chronological order
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_create_event() -> Event {
        Event {
            event_id: "evt-0001".into(),
            thread_id: "RFC-0001".into(),
            event_type: EventType::Create,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: Some("Test RFC".into()),
            kind: Some(ThreadKind::Rfc),
            body: Some("Thread body".into()),
            node_type: None,
            target_node_id: None,
            new_state: None,
            approvals: vec![],
            evidence: None,
            link_rel: None,
            run_label: None,
        }
    }

    #[test]
    fn event_serialize_roundtrip() {
        let event = sample_create_event();
        let json = serde_json::to_string_pretty(&event).unwrap();
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_id, "");
        assert_eq!(parsed.event_type, EventType::Create);
        assert_eq!(parsed.kind, Some(ThreadKind::Rfc));
        assert_eq!(parsed.title.as_deref(), Some("Test RFC"));
        assert_eq!(parsed.body.as_deref(), Some("Thread body"));
    }

    #[test]
    fn event_json_omits_none_fields() {
        let mut event = sample_create_event();
        event.body = None;
        let json = serde_json::to_string_pretty(&event).unwrap();
        assert!(!json.contains("event_id"));
        assert!(!json.contains("body"));
        assert!(!json.contains("node_type"));
        assert!(!json.contains("target_node_id"));
        assert!(!json.contains("new_state"));
    }

    #[test]
    fn thread_kind_initial_status() {
        assert_eq!(ThreadKind::Issue.initial_status(), "open");
        assert_eq!(ThreadKind::Rfc.initial_status(), "draft");
        assert_eq!(ThreadKind::Decision.initial_status(), "proposed");
    }
}
