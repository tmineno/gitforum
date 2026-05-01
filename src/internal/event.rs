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
    /// Initial state for a new thread of this kind, in 2.0 vocabulary.
    /// Delegates to the lifecycle's initial state per SPEC-2.0 §3.1.1
    /// (proposal=draft, execution=open, record=open).
    pub fn initial_status(self) -> &'static str {
        self.lifecycle().initial_state()
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

    /// SPEC-2.0 §2.3.3: each 1.x kind maps to a canonical lifecycle facet.
    /// Used to derive `lifecycle` for legacy threads with no `facet_set`
    /// event in their chain.
    pub fn lifecycle(self) -> Lifecycle {
        match self {
            Self::Issue | Self::Task => Lifecycle::Execution,
            Self::Rfc => Lifecycle::Proposal,
            Self::Dec => Lifecycle::Record,
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

/// SPEC-2.0 §2.3.1 — the sole required facet, gates the unified state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Proposal,
    #[default]
    Execution,
    Record,
}

/// SPEC-2.0 §2.3.5 tag grammar:
/// - ASCII lowercase only, `[a-z0-9-]`
/// - Starts with a letter `[a-z]`
/// - Length 2..=32
/// - Not a reserved literal (`all`, `none`, `any`, `untagged`)
pub fn validate_tag(tag: &str) -> Result<(), String> {
    const RESERVED: &[&str] = &["all", "none", "any", "untagged"];
    if tag.len() < 2 {
        return Err(format!(
            "{tag:?}: tag length must be 2–32 characters (got {})",
            tag.len()
        ));
    }
    if tag.len() > 32 {
        return Err(format!(
            "{tag:?}: tag length must be 2–32 characters (got {})",
            tag.len()
        ));
    }
    let first = tag.chars().next().expect("non-empty after length check");
    if !first.is_ascii_lowercase() {
        return Err(format!(
            "{tag:?}: tag must start with a lowercase letter `[a-z]`"
        ));
    }
    for c in tag.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(format!(
                "{tag:?}: invalid character {c:?} (allowed: `[a-z0-9-]`)"
            ));
        }
    }
    if RESERVED.contains(&tag) {
        return Err(format!(
            "{tag:?} is a reserved filter literal (one of {:?})",
            RESERVED
        ));
    }
    Ok(())
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Execution => "execution",
            Self::Record => "record",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "proposal" => Some(Self::Proposal),
            "execution" => Some(Self::Execution),
            "record" => Some(Self::Record),
            _ => None,
        }
    }

    /// SPEC-2.0 §3.1.1 — initial state per lifecycle.
    pub fn initial_state(self) -> &'static str {
        match self {
            Self::Proposal => "draft",
            Self::Execution | Self::Record => "open",
        }
    }

    /// SPEC-2.0 §3.1.1 — states reachable for this lifecycle.
    pub fn allowed_states(self) -> &'static [&'static str] {
        match self {
            Self::Proposal => &[
                "draft",
                "open",
                "review",
                "done",
                "rejected",
                "withdrawn",
                "deprecated",
            ],
            Self::Execution => &[
                "open",
                "working",
                "review",
                "done",
                "rejected",
                "deprecated",
            ],
            Self::Record => &["open", "done", "rejected", "deprecated"],
        }
    }

    pub fn allows_state(self, state: &str) -> bool {
        self.allowed_states().contains(&state)
    }
}

/// SPEC-2.0 §3.1 — single unified transition graph.
///
/// Every edge any lifecycle might need; `Lifecycle::allowed_states` (§3.1.1)
/// filters reachability per thread. State names are 2.0 canonical.
pub const UNIFIED_TRANSITIONS: &[(&str, &str)] = &[
    ("draft", "open"),
    ("draft", "withdrawn"),
    ("open", "working"),
    ("open", "review"),
    ("open", "done"),
    ("open", "rejected"),
    ("open", "withdrawn"),
    ("working", "review"),
    ("working", "done"),
    ("working", "rejected"),
    ("review", "done"),
    ("review", "working"),
    ("review", "rejected"),
    ("done", "open"),
    ("rejected", "open"),
    ("done", "deprecated"),
    ("rejected", "deprecated"),
];

/// SPEC-2.0 §3.1.2 — pure text-level normalization of 1.x state names to 2.0.
///
/// `designing` and `implementing` both fold to `working`; this is lossy on
/// the 1.x→2.0 direction and intentional per the spec. Withdrawn passes
/// through (it's a 2.0-valid state name); kind-aware adjustments for
/// withdrawn-execution / withdrawn-record live in [`migrate_legacy_state`].
pub fn normalize_state_name(s: &str) -> &str {
    match s {
        "proposed" => "open",
        "under-review" | "reviewing" => "review",
        "accepted" | "closed" => "done",
        "pending" | "designing" | "implementing" => "working",
        _ => s,
    }
}

/// SPEC-2.0 §3.1.1 / §3.1.2 — kind-aware migration of a 1.x state name to a
/// 2.0 state in the lifecycle's allowed set. Composes
/// [`normalize_state_name`] with one further per-lifecycle trim:
/// execution/record lifecycles do not allow `withdrawn`, so legacy
/// `withdrawn` Issue/Task/Dec threads remap to `rejected` (closest 2.0
/// semantic — work was abandoned without being deprecated).
pub fn migrate_legacy_state(kind: ThreadKind, state: &str) -> &str {
    let normalized = normalize_state_name(state);
    if normalized == "withdrawn" && !kind.lifecycle().allows_state("withdrawn") {
        "rejected"
    } else {
        normalized
    }
}

/// Find the shortest path from `from` to `to` via BFS over the unified
/// transition graph, restricted to states allowed for `lifecycle`.
///
/// Inputs may be 1.x state names; they are normalized internally so legacy
/// callers (CLI shorthands, replay of pre-2.0 events) keep working.
pub fn find_path(lifecycle: Lifecycle, from: &str, to: &str) -> Option<Vec<&'static str>> {
    use std::collections::VecDeque;
    let from = normalize_state_name(from);
    let to = normalize_state_name(to);
    if from == to {
        return Some(vec![]);
    }
    if !lifecycle.allows_state(to) {
        return None;
    }
    let mut queue: VecDeque<(&str, Vec<&'static str>)> = VecDeque::new();
    let mut visited: Vec<&str> = vec![from];

    for &(src, dst) in UNIFIED_TRANSITIONS {
        if src == from && lifecycle.allows_state(dst) {
            if dst == to {
                return Some(vec![dst]);
            }
            visited.push(dst);
            queue.push_back((dst, vec![dst]));
        }
    }

    while let Some((current, path)) = queue.pop_front() {
        for &(src, dst) in UNIFIED_TRANSITIONS {
            if src == current && lifecycle.allows_state(dst) && !visited.contains(&dst) {
                let mut new_path = path.clone();
                new_path.push(dst);
                if dst == to {
                    return Some(new_path);
                }
                visited.push(dst);
                queue.push_back((dst, new_path));
            }
        }
    }
    None
}

/// Whether `from -> to` is a valid edge for the given lifecycle.
///
/// Inputs may be 1.x state names; they are normalized internally. Both
/// endpoints must be in the lifecycle's allowed set (§3.1.1) and the edge
/// must exist in the unified §3.1 graph.
pub fn is_valid_transition(lifecycle: Lifecycle, from: &str, to: &str) -> bool {
    let from = normalize_state_name(from);
    let to = normalize_state_name(to);
    lifecycle.allows_state(from)
        && lifecycle.allows_state(to)
        && UNIFIED_TRANSITIONS
            .iter()
            .any(|&(s, d)| s == from && d == to)
}

/// Destination states reachable in one step from `from` for the given lifecycle.
///
/// Returns 2.0 state names. The input may be a 1.x state name; it is
/// normalized internally.
pub fn valid_targets(lifecycle: Lifecycle, from: &str) -> Vec<&'static str> {
    let from = normalize_state_name(from);
    UNIFIED_TRANSITIONS
        .iter()
        .filter_map(|&(s, d)| (s == from && lifecycle.allows_state(d)).then_some(d))
        .collect()
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    fn validate_tag_grammar() {
        // §2.3.5: ASCII lowercase, [a-z][a-z0-9-]{1,31}, not reserved.
        assert!(validate_tag("bug").is_ok());
        assert!(validate_tag("cross-cutting").is_ok());
        assert!(validate_tag("a1").is_ok());
        // Too short.
        assert!(validate_tag("a").is_err());
        // Too long.
        assert!(validate_tag(&"a".repeat(33)).is_err());
        // Bad first char.
        assert!(validate_tag("1bug").is_err());
        // Uppercase.
        assert!(validate_tag("Bug").is_err());
        // Disallowed characters.
        assert!(validate_tag("bug fix").is_err());
        assert!(validate_tag("bug/fix").is_err());
        assert!(validate_tag("bug@fix").is_err());
        // Reserved literals.
        assert!(validate_tag("all").is_err());
        assert!(validate_tag("none").is_err());
        assert!(validate_tag("any").is_err());
        assert!(validate_tag("untagged").is_err());
    }

    #[test]
    fn thread_kind_initial_status() {
        // SPEC-2.0 §3.1.1: per-lifecycle initial state (proposal=draft,
        // execution=open, record=open). DEC's 1.x `proposed` initial state
        // canonicalizes to `open` under the record lifecycle.
        assert_eq!(ThreadKind::Issue.initial_status(), "open");
        assert_eq!(ThreadKind::Rfc.initial_status(), "draft");
        assert_eq!(ThreadKind::Dec.initial_status(), "open");
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

    // ---- SPEC-2.0 §3.1 unified state machine ----

    #[test]
    fn unified_proposal_typical_path() {
        // SPEC-2.0 §3.1.1 typical path for proposal: draft → open → review → done
        assert!(is_valid_transition(Lifecycle::Proposal, "draft", "open"));
        assert!(is_valid_transition(Lifecycle::Proposal, "open", "review"));
        assert!(is_valid_transition(Lifecycle::Proposal, "review", "done"));
    }

    #[test]
    fn unified_execution_excludes_withdrawn() {
        // §3.1.1: execution allowed states do NOT include withdrawn.
        assert!(!Lifecycle::Execution.allows_state("withdrawn"));
        assert!(!is_valid_transition(
            Lifecycle::Execution,
            "open",
            "withdrawn"
        ));
    }

    #[test]
    fn unified_record_excludes_review_and_working() {
        assert!(!Lifecycle::Record.allows_state("working"));
        assert!(!Lifecycle::Record.allows_state("review"));
        assert!(!is_valid_transition(Lifecycle::Record, "open", "review"));
        assert!(is_valid_transition(Lifecycle::Record, "open", "done"));
    }

    #[test]
    fn unified_proposal_excludes_working() {
        // §3.1.1: proposals don't have a "doing the work" state.
        assert!(!Lifecycle::Proposal.allows_state("working"));
    }

    #[test]
    fn find_path_proposal_draft_to_done() {
        // BFS picks shortest: draft → open → done (skips review).
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "done"),
            Some(vec!["open", "done"])
        );
    }

    #[test]
    fn find_path_proposal_draft_to_review() {
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "review"),
            Some(vec!["open", "review"])
        );
    }

    #[test]
    fn find_path_execution_open_to_done_picks_shortest() {
        assert_eq!(
            find_path(Lifecycle::Execution, "open", "done"),
            Some(vec!["done"])
        );
    }

    #[test]
    fn find_path_same_state_returns_empty() {
        assert_eq!(
            find_path(Lifecycle::Proposal, "draft", "draft"),
            Some(vec![])
        );
    }

    #[test]
    fn find_path_unreachable_returns_none() {
        assert_eq!(find_path(Lifecycle::Proposal, "done", "draft"), None);
    }

    #[test]
    fn find_path_bogus_target_returns_none() {
        assert_eq!(find_path(Lifecycle::Proposal, "draft", "bogus"), None);
    }

    #[test]
    fn lifecycle_initial_states() {
        assert_eq!(Lifecycle::Proposal.initial_state(), "draft");
        assert_eq!(Lifecycle::Execution.initial_state(), "open");
        assert_eq!(Lifecycle::Record.initial_state(), "open");
    }

    #[test]
    fn valid_targets_proposal_from_draft() {
        // Only draft→open and draft→withdrawn exist in the unified graph.
        assert_eq!(
            valid_targets(Lifecycle::Proposal, "draft"),
            vec!["open", "withdrawn"]
        );
    }

    #[test]
    fn valid_targets_execution_from_open() {
        // open → working/review/done/rejected. withdrawn excluded by execution.
        assert_eq!(
            valid_targets(Lifecycle::Execution, "open"),
            vec!["working", "review", "done", "rejected"]
        );
    }

    // ---- 1.x state-name normalization at boundaries ----

    #[test]
    fn legacy_state_names_are_normalized_in_queries() {
        // Caller passes 1.x names; the query layer normalizes before checking.
        assert!(is_valid_transition(
            Lifecycle::Proposal,
            "under-review",
            "accepted"
        ));
        assert!(is_valid_transition(Lifecycle::Execution, "open", "closed"));
        assert!(is_valid_transition(
            Lifecycle::Execution,
            "designing",
            "reviewing"
        ));
    }

    #[test]
    fn find_path_accepts_legacy_state_names() {
        // proposed (= open) → accepted (= done) for proposal
        assert_eq!(
            find_path(Lifecycle::Proposal, "proposed", "accepted"),
            Some(vec!["done"])
        );
    }

    // ---- SPEC-2.0 §3.1.2 1.x→2.0 round-trip ----

    /// Every state reachable in any 1.x kind's transition table — the union
    /// of the four legacy state machines, exercised by the round-trip test.
    fn all_1x_states() -> &'static [(ThreadKind, &'static str)] {
        &[
            (ThreadKind::Issue, "open"),
            (ThreadKind::Issue, "pending"),
            (ThreadKind::Issue, "closed"),
            (ThreadKind::Issue, "rejected"),
            (ThreadKind::Issue, "withdrawn"),
            (ThreadKind::Rfc, "draft"),
            (ThreadKind::Rfc, "proposed"),
            (ThreadKind::Rfc, "under-review"),
            (ThreadKind::Rfc, "accepted"),
            (ThreadKind::Rfc, "rejected"),
            (ThreadKind::Rfc, "withdrawn"),
            (ThreadKind::Rfc, "deprecated"),
            (ThreadKind::Dec, "proposed"),
            (ThreadKind::Dec, "accepted"),
            (ThreadKind::Dec, "rejected"),
            (ThreadKind::Dec, "deprecated"),
            (ThreadKind::Dec, "withdrawn"),
            (ThreadKind::Task, "open"),
            (ThreadKind::Task, "designing"),
            (ThreadKind::Task, "implementing"),
            (ThreadKind::Task, "reviewing"),
            (ThreadKind::Task, "closed"),
            (ThreadKind::Task, "rejected"),
            (ThreadKind::Task, "withdrawn"),
        ]
    }

    #[test]
    fn round_trip_every_1x_kind_state_lands_in_lifecycle_allowed_set() {
        // Acceptance criterion (JOB-41f5guw8): every (1.x kind, 1.x state)
        // pair migrates to a valid 2.0 (lifecycle, state) pair.
        for &(kind, state) in all_1x_states() {
            let lifecycle = kind.lifecycle();
            let migrated = migrate_legacy_state(kind, state);
            assert!(
                lifecycle.allows_state(migrated),
                "kind={kind} state={state} migrated={migrated} \
                 lifecycle={lifecycle} allowed={:?}",
                lifecycle.allowed_states(),
            );
        }
    }

    #[test]
    fn migrate_drops_withdrawn_for_execution() {
        assert_eq!(
            migrate_legacy_state(ThreadKind::Issue, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Task, "withdrawn"),
            "rejected"
        );
        assert_eq!(
            migrate_legacy_state(ThreadKind::Rfc, "withdrawn"),
            "withdrawn"
        );
    }

    #[test]
    fn normalize_known_legacy_state_names() {
        assert_eq!(normalize_state_name("accepted"), "done");
        assert_eq!(normalize_state_name("closed"), "done");
        assert_eq!(normalize_state_name("under-review"), "review");
        assert_eq!(normalize_state_name("reviewing"), "review");
        assert_eq!(normalize_state_name("proposed"), "open");
        assert_eq!(normalize_state_name("pending"), "working");
        assert_eq!(normalize_state_name("designing"), "working");
        assert_eq!(normalize_state_name("implementing"), "working");
        assert_eq!(normalize_state_name("draft"), "draft");
        assert_eq!(normalize_state_name("open"), "open");
        assert_eq!(normalize_state_name("rejected"), "rejected");
        assert_eq!(normalize_state_name("withdrawn"), "withdrawn");
        assert_eq!(normalize_state_name("deprecated"), "deprecated");
    }
}
