use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{ForumError, ForumResult};
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::refs;

/// Approval mechanism (legacy SPEC.md §7.7).
///
/// 2.0 folds the standalone Approval concept into the node namespace
/// (SPEC-2.0 §2.8): an approval is just an `approval`-typed node. This type
/// remains so the legacy `Event.approvals` field can still be deserialized
/// from 1.x repos; native 2.0 writes never populate it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMechanism {
    Recorded,
}

/// An approval record formerly attached to State events (SPEC.md §2.7 / §7.7).
///
/// Retained for 1.x read compatibility only; 2.0 writes emit `Say(Approval)`
/// nodes instead (SPEC-2.0 §2.8). Replay synthesizes equivalent Approval
/// Nodes from this field when reading legacy State events so policy guards
/// see a single source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub actor_id: String,
    pub approved_at: DateTime<Utc>,
    pub mechanism: ApprovalMechanism,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_ref: Option<String>,
}

/// Thread kinds supported by git-forum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThreadKind {
    #[default]
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

    /// Display ID prefix (e.g. "ASK", "RFC").
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Issue => "ASK",
            Self::Rfc => "RFC",
            Self::Dec => "DEC",
            Self::Task => "JOB",
        }
    }

    /// Parse a thread kind from an ID prefix string.
    ///
    /// Accepts both current prefixes (ASK, JOB) and legacy prefixes (ISSUE, TASK)
    /// for backward compatibility.
    pub fn from_id_prefix(prefix: &str) -> Option<ThreadKind> {
        match prefix {
            "ASK" | "ISSUE" => Some(Self::Issue),
            "RFC" => Some(Self::Rfc),
            "DEC" => Some(Self::Dec),
            "JOB" | "TASK" => Some(Self::Task),
            _ => None,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    #[default]
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
    Retype,
    /// 2.0: change a thread's facet values (lifecycle / tags). See SPEC-2.0 §2.4.
    #[serde(rename = "facet-set")]
    FacetSet,
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
            Self::Retype => "retype",
            Self::FacetSet => "facet-set",
        };
        f.write_str(s)
    }
}

/// Node types for structured discussion.
///
/// 2.0 reduces the canonical taxonomy to four (`Comment`, `Approval`,
/// `Objection`, `Action`) cut by *protocol effect* per ADR-006. The seven
/// 1.x prose-only variants (`Claim`, `Question`, `Summary`, `Risk`,
/// `Review`, `Alternative`, `Assumption`) and the data-pointer variant
/// (`Evidence`) remain on this enum for backward-compatible reads of legacy
/// repos; new write paths SHOULD use the canonical four. The `canonical()`
/// method maps any variant to its 2.0 equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    // -- 2.0 canonical variants (protocol-effect cut) --
    /// 2.0 canonical: body-prose contribution. No protocol effect.
    /// Replaces 1.x `Claim` / `Question` / `Summary` / `Risk` / `Review` /
    /// `Alternative` / `Assumption`.
    #[default]
    Comment,
    /// 2.0 canonical: positive sign-off. Counts toward state-transition
    /// guards (e.g. `one_human_approval`). Folds in the 1.x standalone
    /// Approval concept (SPEC.md §2.7).
    Approval,
    // -- Variants retained from 1.x (canonical and legacy alike) --
    Objection,
    Action,
    // -- Legacy 1.x variants (kept for read compat; map to Comment) --
    Claim,
    Question,
    Evidence,
    Summary,
    Risk,
    Review,
    Alternative,
    Assumption,
}

impl NodeType {
    /// Map any variant to its 2.0 canonical form.
    ///
    /// - The seven 1.x prose-only variants collapse to `Comment`.
    /// - `Evidence` collapses to `Comment` (the evidence-pointer surface
    ///   moves out of the node namespace entirely; see `evidence add`).
    /// - `Comment`, `Approval`, `Objection`, `Action` are unchanged.
    pub fn canonical(self) -> Self {
        match self {
            Self::Comment | Self::Approval | Self::Objection | Self::Action => self,
            Self::Claim
            | Self::Question
            | Self::Evidence
            | Self::Summary
            | Self::Risk
            | Self::Review
            | Self::Alternative
            | Self::Assumption => Self::Comment,
        }
    }

    /// Returns true if this is a 2.0 canonical variant.
    pub fn is_canonical(self) -> bool {
        matches!(
            self,
            Self::Comment | Self::Approval | Self::Objection | Self::Action
        )
    }

    /// Legacy 1.x label for non-canonical variants, or `None` if already canonical.
    ///
    /// Used by 2.0 write paths to record the user's stated rhetorical type in
    /// `Event.legacy_subtype` while persisting the canonical `node_type` on
    /// the event (SPEC-2.0 §2.5 / §9.3 / §10.1).
    pub fn legacy_subtype_label(self) -> Option<&'static str> {
        match self {
            Self::Comment | Self::Approval | Self::Objection | Self::Action => None,
            Self::Claim => Some("claim"),
            Self::Question => Some("question"),
            Self::Evidence => Some("evidence"),
            Self::Summary => Some("summary"),
            Self::Risk => Some("risk"),
            Self::Review => Some("review"),
            Self::Alternative => Some("alternative"),
            Self::Assumption => Some("assumption"),
        }
    }
}

impl std::fmt::Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Comment => "comment",
            Self::Approval => "approval",
            Self::Objection => "objection",
            Self::Action => "action",
            Self::Claim => "claim",
            Self::Question => "question",
            Self::Evidence => "evidence",
            Self::Summary => "summary",
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
            "comment" => Ok(Self::Comment),
            "approval" => Ok(Self::Approval),
            "objection" => Ok(Self::Objection),
            "action" => Ok(Self::Action),
            "claim" => Ok(Self::Claim),
            "question" => Ok(Self::Question),
            "evidence" => Ok(Self::Evidence),
            "summary" => Ok(Self::Summary),
            "risk" => Ok(Self::Risk),
            "review" => Ok(Self::Review),
            "alternative" => Ok(Self::Alternative),
            "assumption" => Ok(Self::Assumption),
            _ => Err(format!("unknown node type '{s}'; canonical types (2.0): comment, approval, objection, action; legacy types accepted for reads: claim, question, evidence, summary, risk, review, alternative, assumption")),
        }
    }
}

/// An immutable event in a thread's history.
///
/// Stored as `event.json` inside each Git commit's tree.
///
/// `Default` is implemented so test sites and helpers can construct events
/// with `Event { thread_id: "...".into(), event_type: ..., ..Default::default() }`
/// rather than enumerating every field. This keeps adding new optional fields
/// from cascading edits across the codebase.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// Previous node type before a Retype event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_node_type: Option<NodeType>,
    /// 2.0: rhetorical-subtype label preserved alongside the canonical
    /// `node_type`. Set when:
    /// - the migration tool rewrites a 1.x non-canonical node (Track C), or
    /// - a native 2.0 write path canonicalized a user-supplied legacy type
    ///   (e.g. `git forum claim` records `node_type = comment`,
    ///   `legacy_subtype = "claim"`; SPEC-2.0 §2.5 / §9.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_subtype: Option<String>,
    /// 2.0: lifecycle facet value set on this `facet_set` event. Present only on
    /// the thread's first `facet_set` (immutable after creation per SPEC-2.0 §7.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// 2.0: tags added by this `facet_set` event. Replay: applied before `tags_remove`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags_add: Vec<String>,
    /// 2.0: tags removed by this `facet_set` event. Replay: applied after `tags_add`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags_remove: Vec<String>,
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
            thread_id: thread_id.to_string(),
            event_type,
            created_at: clock.now(),
            actor: actor.to_string(),
            ..Self::default()
        }
    }

    /// Builder: set the `lifecycle` facet on a `facet_set` event.
    pub fn with_lifecycle(mut self, lifecycle: &str) -> Self {
        self.lifecycle = Some(lifecycle.to_string());
        self
    }

    /// Builder: set tags added by this `facet_set` event.
    pub fn with_tags_add(mut self, tags: Vec<String>) -> Self {
        self.tags_add = tags;
        self
    }

    /// Builder: set tags removed by this `facet_set` event.
    pub fn with_tags_remove(mut self, tags: Vec<String>) -> Self {
        self.tags_remove = tags;
        self
    }

    pub fn with_old_node_type(mut self, old_type: NodeType) -> Self {
        self.old_node_type = Some(old_type);
        self
    }

    pub fn with_body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }

    pub fn with_node_type(mut self, node_type: NodeType) -> Self {
        self.node_type = Some(node_type);
        self
    }

    /// Builder: record a rhetorical-subtype label (see [`NodeType::legacy_subtype_label`]).
    pub fn with_legacy_subtype(mut self, label: &str) -> Self {
        self.legacy_subtype = Some(label.to_string());
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
    let json = git.show_file(commit_sha, "event.json").map_err(|e| {
        ForumError::Git(format!(
            "commit {commit_sha} has no event.json (corrupt thread history): {e}"
        ))
    })?;
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
            title: Some("Test RFC".into()),
            kind: Some(ThreadKind::Rfc),
            body: Some("Thread body".into()),
            ..Event::default()
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
        assert!(!json.contains("lifecycle"));
        assert!(!json.contains("tags_add"));
        assert!(!json.contains("tags_remove"));
    }

    #[test]
    fn facet_set_event_roundtrip() {
        let mut event = sample_create_event();
        event.event_type = EventType::FacetSet;
        event.title = None;
        event.kind = None;
        event.body = None;
        event.lifecycle = Some("proposal".into());
        event.tags_add = vec!["cross-cutting".into(), "task".into()];
        event.tags_remove = vec!["bug".into()];
        let json = serde_json::to_string_pretty(&event).unwrap();
        assert!(json.contains("\"event_type\": \"facet-set\""));
        assert!(json.contains("\"lifecycle\": \"proposal\""));
        assert!(json.contains("cross-cutting"));
        assert!(json.contains("bug"));
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, EventType::FacetSet);
        assert_eq!(parsed.lifecycle.as_deref(), Some("proposal"));
        assert_eq!(
            parsed.tags_add,
            vec!["cross-cutting".to_string(), "task".to_string()]
        );
        assert_eq!(parsed.tags_remove, vec!["bug".to_string()]);
    }

    #[test]
    fn node_type_canonical_collapses_legacy_to_comment() {
        // 2.0 canonical four pass through unchanged.
        for nt in [
            NodeType::Comment,
            NodeType::Approval,
            NodeType::Objection,
            NodeType::Action,
        ] {
            assert_eq!(nt.canonical(), nt);
            assert!(nt.is_canonical());
        }
        // Seven prose-only legacy variants + Evidence collapse to Comment.
        for nt in [
            NodeType::Claim,
            NodeType::Question,
            NodeType::Evidence,
            NodeType::Summary,
            NodeType::Risk,
            NodeType::Review,
            NodeType::Alternative,
            NodeType::Assumption,
        ] {
            assert_eq!(nt.canonical(), NodeType::Comment);
            assert!(!nt.is_canonical());
        }
    }

    #[test]
    fn node_type_legacy_subtype_label() {
        // Canonical types have no legacy label.
        for nt in [
            NodeType::Comment,
            NodeType::Approval,
            NodeType::Objection,
            NodeType::Action,
        ] {
            assert_eq!(nt.legacy_subtype_label(), None);
        }
        // Legacy types map back to their string form.
        assert_eq!(NodeType::Claim.legacy_subtype_label(), Some("claim"));
        assert_eq!(NodeType::Question.legacy_subtype_label(), Some("question"));
        assert_eq!(NodeType::Evidence.legacy_subtype_label(), Some("evidence"));
        assert_eq!(NodeType::Summary.legacy_subtype_label(), Some("summary"));
        assert_eq!(NodeType::Risk.legacy_subtype_label(), Some("risk"));
        assert_eq!(NodeType::Review.legacy_subtype_label(), Some("review"));
        assert_eq!(
            NodeType::Alternative.legacy_subtype_label(),
            Some("alternative")
        );
        assert_eq!(
            NodeType::Assumption.legacy_subtype_label(),
            Some("assumption")
        );
    }

    #[test]
    fn node_type_parses_canonical_and_legacy() {
        use std::str::FromStr;
        assert_eq!(NodeType::from_str("comment").unwrap(), NodeType::Comment);
        assert_eq!(NodeType::from_str("approval").unwrap(), NodeType::Approval);
        assert_eq!(
            NodeType::from_str("objection").unwrap(),
            NodeType::Objection
        );
        assert_eq!(NodeType::from_str("action").unwrap(), NodeType::Action);
        // Legacy types still parse so 1.x events can be read.
        assert_eq!(NodeType::from_str("claim").unwrap(), NodeType::Claim);
        assert_eq!(NodeType::from_str("summary").unwrap(), NodeType::Summary);
        assert!(NodeType::from_str("nonsense").is_err());
    }

    #[test]
    fn node_type_serialize_roundtrip_canonical() {
        // 2.0 canonical types serialize as lowercase strings and round-trip.
        let json = serde_json::to_string(&NodeType::Comment).unwrap();
        assert_eq!(json, "\"comment\"");
        let parsed: NodeType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, NodeType::Comment);

        let json = serde_json::to_string(&NodeType::Approval).unwrap();
        assert_eq!(json, "\"approval\"");
        let parsed: NodeType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, NodeType::Approval);
    }

    #[test]
    fn facet_set_event_empty_payload_serializes_minimally() {
        let mut event = sample_create_event();
        event.event_type = EventType::FacetSet;
        event.title = None;
        event.kind = None;
        event.body = None;
        let json = serde_json::to_string_pretty(&event).unwrap();
        // Empty facet_set is allowed (backfill / hook no-op per SPEC-2.0 §2.4.1).
        assert!(!json.contains("lifecycle"));
        assert!(!json.contains("tags_add"));
        assert!(!json.contains("tags_remove"));
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, EventType::FacetSet);
        assert!(parsed.lifecycle.is_none());
        assert!(parsed.tags_add.is_empty());
        assert!(parsed.tags_remove.is_empty());
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
