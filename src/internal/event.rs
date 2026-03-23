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
    Dec,
    Task,
}

impl ThreadKind {
    /// Initial state for a new thread of this kind.
    pub fn initial_status(self) -> &'static str {
        match self {
            Self::Issue => "open",
            Self::Rfc => "draft",
            Self::Dec => "proposed",
            Self::Task => "open",
        }
    }

    /// Display ID prefix (e.g. "ISSUE", "RFC").
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Issue => "ISSUE",
            Self::Rfc => "RFC",
            Self::Dec => "DEC",
            Self::Task => "TASK",
        }
    }
}

impl std::fmt::Display for ThreadKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue => write!(f, "issue"),
            Self::Rfc => write!(f, "rfc"),
            Self::Dec => write!(f, "dec"),
            Self::Task => write!(f, "task"),
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
    State,
    Scope,
    Resolve,
    Reopen,
    Verify,
    Merge,
    #[serde(rename = "revise-body")]
    ReviseBody,
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Create => "create",
            Self::Edit => "edit",
            Self::Retract => "retract",
            Self::Say => "say",
            Self::Link => "link",
            Self::State => "state",
            Self::Scope => "scope",
            Self::Resolve => "resolve",
            Self::Reopen => "reopen",
            Self::Verify => "verify",
            Self::Merge => "merge",
            Self::ReviseBody => "revise-body",
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
    Evidence,
    Summary,
    Action,
    Risk,
    Review,
    Alternative,
    Assumption,
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Claim => "claim",
            Self::Question => "question",
            Self::Objection => "objection",
            Self::Evidence => "evidence",
            Self::Summary => "summary",
            Self::Action => "action",
            Self::Risk => "risk",
            Self::Review => "review",
            Self::Alternative => "alternative",
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
            "evidence" => Ok(Self::Evidence),
            "summary" => Ok(Self::Summary),
            "action" => Ok(Self::Action),
            "risk" => Ok(Self::Risk),
            "review" => Ok(Self::Review),
            "alternative" => Ok(Self::Alternative),
            "assumption" => Ok(Self::Assumption),
            _ => Err(format!("unknown node type '{s}'; valid types: claim, question, objection, evidence, summary, action, risk, review, alternative, assumption")),
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
    /// Branch scope recorded by Create/Scope events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Node IDs incorporated into a body revision (ReviseBody events).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incorporated_node_ids: Vec<String>,
    /// Node ID this node is replying to (Say events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

impl Event {
    /// Create an Event with required fields filled and all optional fields set to defaults.
    pub fn base(
        thread_id: &str,
        event_type: EventType,
        actor: &str,
        clock: &dyn super::clock::Clock,
    ) -> Self {
        Self {
            event_id: String::new(),
            thread_id: thread_id.to_string(),
            event_type,
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
            evidence: None,
            link_rel: None,
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
        }
    }

    pub fn with_body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }

    pub fn with_node_type(mut self, node_type: NodeType) -> Self {
        self.node_type = Some(node_type);
        self
    }

    pub fn with_target_node_id(mut self, node_id: &str) -> Self {
        self.target_node_id = Some(node_id.to_string());
        self
    }

    pub fn with_reply_to(mut self, reply_to: Option<&str>) -> Self {
        self.reply_to = reply_to.map(str::to_string);
        self
    }

    pub fn with_new_state(mut self, state: &str) -> Self {
        self.new_state = Some(state.to_string());
        self
    }

    pub fn with_approvals(mut self, approvals: Vec<Approval>) -> Self {
        self.approvals = approvals;
        self
    }

    pub fn with_title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    pub fn with_kind(mut self, kind: ThreadKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn with_branch(mut self, branch: Option<&str>) -> Self {
        self.branch = branch.map(str::to_string);
        self
    }

    pub fn with_incorporated_node_ids(mut self, ids: Vec<String>) -> Self {
        self.incorporated_node_ids = ids;
        self
    }

    pub fn with_evidence(mut self, evidence: super::evidence::Evidence) -> Self {
        self.evidence = Some(evidence);
        self
    }

    pub fn with_link_rel(mut self, rel: &str) -> Self {
        self.link_rel = Some(rel.to_string());
        self
    }

    pub fn with_created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = created_at;
        self
    }

    /// Validate event field sizes and content.
    ///
    /// Returns an error if any field exceeds safety limits. Called at both
    /// write-time and read-time to prevent DoS via oversized fields.
    pub fn validate(&self) -> ForumResult<()> {
        const MAX_ACTOR: usize = 200;
        const MAX_TITLE: usize = 500;
        const MAX_BODY: usize = 10 * 1024 * 1024; // 10 MB
        const MAX_THREAD_ID: usize = 100;

        if self.actor.len() > MAX_ACTOR {
            return Err(super::error::ForumError::Config(format!(
                "actor too long ({} chars, max {MAX_ACTOR})",
                self.actor.len()
            )));
        }
        if self.thread_id.len() > MAX_THREAD_ID {
            return Err(super::error::ForumError::Config(format!(
                "thread_id too long ({} chars, max {MAX_THREAD_ID})",
                self.thread_id.len()
            )));
        }
        if let Some(ref title) = self.title {
            if title.len() > MAX_TITLE {
                return Err(super::error::ForumError::Config(format!(
                    "title too long ({} chars, max {MAX_TITLE})",
                    title.len()
                )));
            }
        }
        if let Some(ref body) = self.body {
            if body.len() > MAX_BODY {
                return Err(super::error::ForumError::Config(format!(
                    "body too long ({} bytes, max {MAX_BODY})",
                    body.len()
                )));
            }
        }
        Ok(())
    }
}

/// Write an event as a Git commit and update the thread ref.
///
/// Returns the new commit SHA.
pub fn write_event(git: &GitOps, event: &Event) -> ForumResult<String> {
    event.validate()?;
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

    // Use compare-and-swap to detect concurrent writes
    match &parent_sha {
        Some(old_sha) => git.update_ref_cas(&ref_name, &commit_sha, old_sha)?,
        None if event.event_type == EventType::Create => git.create_ref(&ref_name, &commit_sha)?,
        None => {
            return Err(super::error::ForumError::Repo(format!(
                "thread {} does not exist (no ref at {})",
                event.thread_id, ref_name
            )));
        }
    }
    Ok(commit_sha)
}

/// Read an event from a commit SHA.
pub fn read_event(git: &GitOps, commit_sha: &str) -> ForumResult<Event> {
    let json = git.show_file(commit_sha, "event.json")?;
    let mut event: Event = serde_json::from_str(&json)?;
    event.event_id = commit_sha.to_string();
    event.validate()?;
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
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
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
        assert!(!json.contains("branch"));
    }

    #[test]
    fn thread_kind_initial_status() {
        assert_eq!(ThreadKind::Issue.initial_status(), "open");
        assert_eq!(ThreadKind::Rfc.initial_status(), "draft");
        assert_eq!(ThreadKind::Dec.initial_status(), "proposed");
        assert_eq!(ThreadKind::Task.initial_status(), "open");
    }

    #[test]
    fn validate_accepts_normal_event() {
        let event = sample_create_event();
        assert!(event.validate().is_ok());
    }

    #[test]
    fn validate_rejects_oversized_actor() {
        let mut event = sample_create_event();
        event.actor = "x".repeat(201);
        assert!(event.validate().is_err());
    }

    #[test]
    fn validate_rejects_oversized_title() {
        let mut event = sample_create_event();
        event.title = Some("x".repeat(501));
        assert!(event.validate().is_err());
    }

    #[test]
    fn validate_rejects_oversized_body() {
        let mut event = sample_create_event();
        event.body = Some("x".repeat(10 * 1024 * 1024 + 1));
        assert!(event.validate().is_err());
    }

    #[test]
    fn validate_accepts_max_size_fields() {
        let mut event = sample_create_event();
        event.actor = "x".repeat(200);
        event.title = Some("x".repeat(500));
        event.body = Some("x".repeat(10 * 1024 * 1024));
        assert!(event.validate().is_ok());
    }
}
