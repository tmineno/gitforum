use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{ForumError, ForumResult};
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::refs;
use super::workflow::SPEC;

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

// `ThreadKind` was relocated to `internal::thread` in Phase 4 Step 1g
// (RFC `7ymtc4b2`, task `913c4s9v`). KEEP files import via the new
// path; event.rs re-exports for back-compat with the legacy event
// loaders that still consume it (deleted in Step 2/3).
pub use super::thread::ThreadKind;

/// SPEC-2.0 §3.1 — the canonical 2.0 state set across every lifecycle.
///
/// Phase 2a (Finding 1 follow-up): the in-memory representation of a
/// thread's status. Storage (`Event.new_state: Option<String>`) stays
/// String-typed for compatibility with 1.x event chains and forward
/// flexibility; this enum is the read-side type after `parse_lenient`
/// has folded 1.x synonyms (`closed`, `proposed`, …) onto canonical
/// 2.0 names.
///
/// Per-lifecycle reachability is enforced by [`Lifecycle::allows_state`];
/// this enum is intentionally lifecycle-agnostic so legacy chains whose
/// state names predate the 2.0 split can still be replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThreadStatus {
    Draft,
    #[default]
    Open,
    Working,
    Review,
    Done,
    Rejected,
    Withdrawn,
    Deprecated,
}

impl ThreadStatus {
    /// Canonical 2.0 names only — does NOT accept 1.x synonyms.
    /// Use [`parse_lenient`](Self::parse_lenient) for inputs that may
    /// carry pre-2.0 names (`closed`, `proposed`, etc.).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "open" => Some(Self::Open),
            "working" => Some(Self::Working),
            "review" => Some(Self::Review),
            "done" => Some(Self::Done),
            "rejected" => Some(Self::Rejected),
            "withdrawn" => Some(Self::Withdrawn),
            "deprecated" => Some(Self::Deprecated),
            _ => None,
        }
    }

    /// Accepts canonical 2.0 names AND 1.x synonyms by routing through
    /// [`normalize_state_name`]. The lenient `apply_event` path uses this
    /// so legacy event chains keep replaying.
    pub fn parse_lenient(s: &str) -> Option<Self> {
        Self::parse(normalize_state_name(s))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Open => "open",
            Self::Working => "working",
            Self::Review => "review",
            Self::Done => "done",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
            Self::Deprecated => "deprecated",
        }
    }
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegate to &str's Display so format-spec padding (`{:<width$}`)
        // and precision rules behave identically to a plain string.
        std::fmt::Display::fmt(self.as_str(), f)
    }
}

// Ergonomic comparisons against string literals — keeps test assertions
// like `assert_eq!(state.status, "draft")` readable without forcing every
// test module to import `ThreadStatus`. The 1.x lenient mapping is
// intentionally NOT applied here: comparison is exact against the canonical
// 2.0 name. Callers that want lenient semantics use
// `ThreadStatus::parse_lenient(s) == Some(state.status)`.
impl PartialEq<&str> for ThreadStatus {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<ThreadStatus> for &str {
    fn eq(&self, other: &ThreadStatus) -> bool {
        *self == other.as_str()
    }
}
impl PartialEq<str> for ThreadStatus {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<ThreadStatus> for str {
    fn eq(&self, other: &ThreadStatus) -> bool {
        self == other.as_str()
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
        SPEC.initial_state(self)
    }

    /// SPEC-2.0 §3.1.1 — states reachable for this lifecycle.
    pub fn allowed_states(self) -> &'static [&'static str] {
        SPEC.allowed_states(self)
    }

    pub fn allows_state(self, state: &str) -> bool {
        SPEC.allows_state(self, state)
    }
}

/// SPEC-2.0 §3.1 — single unified transition graph.
///
/// Every edge any lifecycle might need; `Lifecycle::allowed_states` (§3.1.1)
/// filters reachability per thread. State names are 2.0 canonical.
///
/// Re-exported from [`workflow::WorkflowSpec::unified_transitions`] for
/// callers that iterate the raw edge list (state-diagram rendering,
/// policy lint).
pub fn unified_transitions() -> &'static [(&'static str, &'static str)] {
    SPEC.unified_transitions()
}

/// SPEC-2.0 §3.1.2 — pure text-level normalization of 1.x state names to 2.0.
///
/// Thin wrapper that re-exports [`super::legacy::v1::normalize_state_name`]
/// so existing call sites keep their `event::normalize_state_name`
/// import path. New domain code should call into [`super::legacy::v1`]
/// directly per RFC 915yuegd P1.
pub fn normalize_state_name(s: &str) -> &str {
    super::legacy::v1::normalize_state_name(s)
}

/// SPEC-2.0 §3.1.1 / §3.1.2 — kind-aware migration of a 1.x state name to a
/// 2.0 state in the lifecycle's allowed set.
///
/// Thin wrapper over [`super::legacy::v1::migrate_legacy_state`].
pub fn migrate_legacy_state(kind: ThreadKind, state: &str) -> &str {
    super::legacy::v1::migrate_legacy_state(kind, state)
}

/// Shortest path from `from` to `to` for `lifecycle`. Thin wrapper over
/// [`SPEC::find_path`](super::workflow::WorkflowSpec::find_path).
pub fn find_path(lifecycle: Lifecycle, from: &str, to: &str) -> Option<Vec<&'static str>> {
    SPEC.find_path(lifecycle, from, to)
}

/// Whether `from -> to` is a valid edge for the given lifecycle. Thin
/// wrapper over [`SPEC::is_valid_transition`](super::workflow::WorkflowSpec::is_valid_transition).
pub fn is_valid_transition(lifecycle: Lifecycle, from: &str, to: &str) -> bool {
    SPEC.is_valid_transition(lifecycle, from, to)
}

/// Destination states reachable in one step from `from` for `lifecycle`.
/// Thin wrapper over [`SPEC::valid_targets`](super::workflow::WorkflowSpec::valid_targets).
pub fn valid_targets(lifecycle: Lifecycle, from: &str) -> Vec<&'static str> {
    SPEC.valid_targets(lifecycle, from)
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

// `NodeType` was relocated to `internal::node` in Phase 4 Step 1f
// (RFC `7ymtc4b2`, task `913c4s9v`) so KEEP files no longer need to
// import from `internal::event` just for the type. Importers should
// switch to `crate::internal::node::NodeType`. event.rs back-imports
// here so internal references inside this file (and the legacy
// `Event` struct's `node_type` field) keep working until Step 2
// moves event.rs into `internal::legacy/`.
pub use super::node::NodeType;

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

// ============================================================================
// Stored / Domain split (P2 #wlqhi8xh, ADR-010)
//
// `Event` is the storage bag (~17 Optional fields shaped for serde
// round-trips and forward compat). `DomainEvent` is the typed sum
// projected from `Event` for replay / domain logic — each variant
// carries only the fields its `EventType` actually uses, so consumers
// stop unwrapping defensively. The bag is intentionally preserved
// (event.rs:519 cascading-edit shield) for storage and construction.
// ============================================================================

/// Common metadata every event variant carries. Carved out of
/// `DomainEvent` so each variant declares only its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventMeta {
    pub event_id: String,
    pub thread_id: String,
    /// Stored event_type. Useful for issue reporting (`MissingRequiredField`
    /// includes the offending event_type) and for the
    /// [`DomainEvent::Unknown`] case once Phase B (ADR-010) lands.
    pub event_type: EventType,
    pub created_at: DateTime<Utc>,
    pub actor: String,
    pub base_rev: Option<String>,
    pub parents: Vec<String>,
}

/// Two valid shapes for a `Link` event payload (SPEC-2.0 §2.6 / §2.6.1):
/// either it carries an `Evidence` bundle, or it carries
/// `target_thread_id` + relation kind.
#[derive(Debug, Clone)]
pub enum LinkPayload {
    Evidence(super::evidence::Evidence),
    Thread {
        target_thread_id: String,
        link_rel: String,
    },
}

/// Typed projection of a stored [`Event`].
///
/// Construct via [`Event::project`]. Replay matches on this enum
/// instead of unwrapping bag fields case-by-case; the compiler
/// enforces that every domain consumer handles every variant.
///
/// Adding a new domain-meaningful event variant requires four edits:
/// the [`EventType`] enum, this enum, the [`Event::project`] arm, and
/// the `apply_event` arm in `internal::thread`. No optional fields
/// cascade across the codebase.
///
/// The [`Self::Unknown`] variant is reserved per ADR-010 option (a):
/// today every `EventType` has an explicit projection arm so it is
/// never constructed; Phase B (ADR-010) will add
/// `EventType::Other(String)` and route it here.
#[derive(Debug, Clone)]
pub enum DomainEvent {
    Create {
        meta: EventMeta,
        title: String,
        kind: ThreadKind,
        body: Option<String>,
        branch: Option<String>,
    },
    Edit {
        meta: EventMeta,
        target_node_id: String,
        body: String,
    },
    Retract {
        meta: EventMeta,
        target_node_id: String,
    },
    Resolve {
        meta: EventMeta,
        target_node_id: String,
    },
    Reopen {
        meta: EventMeta,
        target_node_id: String,
    },
    Say {
        meta: EventMeta,
        node_type: NodeType,
        body: String,
        reply_to: Option<String>,
        legacy_subtype: Option<String>,
        /// Optional pre-allocated node id. When `Some`, the materialised
        /// node uses this id; when `None`, the node id is the event's
        /// commit SHA (the common case for native 2.0 writes).
        target_node_id: Option<String>,
    },
    Link {
        meta: EventMeta,
        payload: LinkPayload,
    },
    /// `new_state` is intentionally left as the raw stored string —
    /// `parse_lenient` (and the `InvalidStateValue` strict-issue path)
    /// runs at apply time so projection's failure mode stays narrow
    /// (only structural shape).
    State {
        meta: EventMeta,
        new_state: String,
        approvals: Vec<Approval>,
    },
    Scope {
        meta: EventMeta,
        branch: Option<String>,
    },
    Verify {
        meta: EventMeta,
    },
    Merge {
        meta: EventMeta,
    },
    ReviseBody {
        meta: EventMeta,
        body: String,
        incorporated_node_ids: Vec<String>,
    },
    Retype {
        meta: EventMeta,
        target_node_id: String,
        node_type: NodeType,
        old_node_type: Option<NodeType>,
    },
    /// `lifecycle` is left as the raw stored string for the same
    /// reason as `State::new_state` — `Lifecycle::parse` and the
    /// `InvalidLifecycleValue` issue path run at apply time.
    FacetSet {
        meta: EventMeta,
        lifecycle: Option<String>,
        tags_add: Vec<String>,
        tags_remove: Vec<String>,
    },
    /// Reserved per ADR-010 option (a). Unreachable today; Phase B
    /// will route `EventType::Other(String)` to this variant.
    #[allow(dead_code)]
    Unknown {
        meta: EventMeta,
        raw: Box<Event>,
    },
}

impl DomainEvent {
    /// Borrow the common event metadata regardless of variant.
    pub fn meta(&self) -> &EventMeta {
        match self {
            Self::Create { meta, .. }
            | Self::Edit { meta, .. }
            | Self::Retract { meta, .. }
            | Self::Resolve { meta, .. }
            | Self::Reopen { meta, .. }
            | Self::Say { meta, .. }
            | Self::Link { meta, .. }
            | Self::State { meta, .. }
            | Self::Scope { meta, .. }
            | Self::Verify { meta }
            | Self::Merge { meta }
            | Self::ReviseBody { meta, .. }
            | Self::Retype { meta, .. }
            | Self::FacetSet { meta, .. }
            | Self::Unknown { meta, .. } => meta,
        }
    }
}

/// Reason a stored [`Event`] failed to project to a [`DomainEvent`].
///
/// Mirrors [`super::validate::StrictReplayIssue::MissingRequiredField`]
/// — replay catches the error and turns it into the corresponding
/// strict issue while letting lenient replay continue with no state
/// change for that event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    MissingRequiredField { field: &'static str },
}

impl Event {
    /// Project this stored event into its typed [`DomainEvent`].
    ///
    /// Failure surfaces structural shape mismatches only (a required
    /// field is absent). Semantic validity — unparseable state names,
    /// out-of-set lifecycle values, unknown target node IDs — is
    /// left to `apply_event`, mirroring how lenient replay applies
    /// best-effort and strict replay collects issues.
    pub fn project(&self) -> Result<DomainEvent, ProjectionError> {
        let meta = EventMeta {
            event_id: self.event_id.clone(),
            thread_id: self.thread_id.clone(),
            event_type: self.event_type,
            created_at: self.created_at,
            actor: self.actor.clone(),
            base_rev: self.base_rev.clone(),
            parents: self.parents.clone(),
        };
        match self.event_type {
            EventType::Create => {
                let title = self
                    .title
                    .clone()
                    .ok_or(ProjectionError::MissingRequiredField { field: "title" })?;
                let kind = self
                    .kind
                    .ok_or(ProjectionError::MissingRequiredField { field: "kind" })?;
                Ok(DomainEvent::Create {
                    meta,
                    title,
                    kind,
                    body: self.body.clone(),
                    branch: self.branch.clone(),
                })
            }
            EventType::Edit => {
                let target_node_id =
                    self.target_node_id
                        .clone()
                        .ok_or(ProjectionError::MissingRequiredField {
                            field: "target_node_id",
                        })?;
                let body = self
                    .body
                    .clone()
                    .ok_or(ProjectionError::MissingRequiredField { field: "body" })?;
                Ok(DomainEvent::Edit {
                    meta,
                    target_node_id,
                    body,
                })
            }
            EventType::Retract => Ok(DomainEvent::Retract {
                target_node_id: self.target_node_id.clone().ok_or(
                    ProjectionError::MissingRequiredField {
                        field: "target_node_id",
                    },
                )?,
                meta,
            }),
            EventType::Resolve => Ok(DomainEvent::Resolve {
                target_node_id: self.target_node_id.clone().ok_or(
                    ProjectionError::MissingRequiredField {
                        field: "target_node_id",
                    },
                )?,
                meta,
            }),
            EventType::Reopen => Ok(DomainEvent::Reopen {
                target_node_id: self.target_node_id.clone().ok_or(
                    ProjectionError::MissingRequiredField {
                        field: "target_node_id",
                    },
                )?,
                meta,
            }),
            EventType::Say => {
                // Mirrors apply_event's prior arm-ordering: node_type
                // missing reports first, body second.
                let node_type = self
                    .node_type
                    .ok_or(ProjectionError::MissingRequiredField { field: "node_type" })?;
                let body = self
                    .body
                    .clone()
                    .ok_or(ProjectionError::MissingRequiredField { field: "body" })?;
                Ok(DomainEvent::Say {
                    meta,
                    node_type,
                    body,
                    reply_to: self.reply_to.clone(),
                    legacy_subtype: self.legacy_subtype.clone(),
                    target_node_id: self.target_node_id.clone(),
                })
            }
            EventType::Link => {
                let payload = if let Some(ev_data) = &self.evidence {
                    LinkPayload::Evidence(ev_data.clone())
                } else if let (Some(target), Some(rel)) = (&self.target_node_id, &self.link_rel) {
                    LinkPayload::Thread {
                        target_thread_id: target.clone(),
                        link_rel: rel.clone(),
                    }
                } else {
                    return Err(ProjectionError::MissingRequiredField {
                        field: "evidence_or_target_link",
                    });
                };
                Ok(DomainEvent::Link { meta, payload })
            }
            EventType::State => {
                let new_state = self
                    .new_state
                    .clone()
                    .ok_or(ProjectionError::MissingRequiredField { field: "new_state" })?;
                Ok(DomainEvent::State {
                    meta,
                    new_state,
                    approvals: self.approvals.clone(),
                })
            }
            EventType::Scope => Ok(DomainEvent::Scope {
                meta,
                branch: self.branch.clone(),
            }),
            EventType::Verify => Ok(DomainEvent::Verify { meta }),
            EventType::Merge => Ok(DomainEvent::Merge { meta }),
            EventType::ReviseBody => {
                let body = self
                    .body
                    .clone()
                    .ok_or(ProjectionError::MissingRequiredField { field: "body" })?;
                Ok(DomainEvent::ReviseBody {
                    meta,
                    body,
                    incorporated_node_ids: self.incorporated_node_ids.clone(),
                })
            }
            EventType::Retype => {
                let target_node_id =
                    self.target_node_id
                        .clone()
                        .ok_or(ProjectionError::MissingRequiredField {
                            field: "target_node_id",
                        })?;
                let node_type = self
                    .node_type
                    .ok_or(ProjectionError::MissingRequiredField { field: "node_type" })?;
                Ok(DomainEvent::Retype {
                    meta,
                    target_node_id,
                    node_type,
                    old_node_type: self.old_node_type,
                })
            }
            EventType::FacetSet => Ok(DomainEvent::FacetSet {
                meta,
                lifecycle: self.lifecycle.clone(),
                tags_add: self.tags_add.clone(),
                tags_remove: self.tags_remove.clone(),
            }),
        }
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
    load_thread_events_at(git, &ref_name)
}

/// Like [`load_thread_events`], but walks from a caller-supplied
/// rev (typically a captured commit OID) instead of resolving the
/// thread ref live. Used by migrate to pin the read against the
/// exact tip recorded for the eventual CAS write — so a concurrent
/// event landing between read and write fails the CAS instead of
/// silently dropping events from the archive (task `9635buy0`,
/// objection `e630f01f`).
pub fn load_thread_events_at(git: &GitOps, start_rev: &str) -> ForumResult<Vec<Event>> {
    let shas = git.rev_list(start_rev)?; // newest first
    let mut events = Vec::with_capacity(shas.len());
    for sha in &shas {
        events.push(read_event(git, sha)?);
    }
    events.reverse(); // chronological order
    Ok(events)
}

/// Like [`load_thread_events_at`], but stops walking at the first
/// ancestor that is a SPEC-3.0 snapshot commit (one whose tree
/// contains `thread.toml`). Returns the legacy event tail in
/// chronological order plus the OID of the snapshot ancestor when
/// one is present.
///
/// Used by migrate so a Phase-2 cutover ref (snapshot bottom +
/// event tail at tip) does not error trying to parse the snapshot
/// commit as `event.json`. `read_snapshot` only inspects the tip
/// tree, so an event-tip + snapshot-ancestor ref still routes to
/// migrate via [`crate::internal::error::ForumError::LegacyEventChain`]
/// — this loader handles that case (task `9635buy0`,
/// objection `bf678561`).
pub fn load_event_tail_at(
    git: &GitOps,
    start_rev: &str,
) -> ForumResult<(Vec<Event>, Option<String>)> {
    // `rev_list` is newest-first. Walk forward, stop at the first
    // snapshot ancestor — every commit *between* the start_rev and
    // that ancestor must be an event commit; we collect those.
    let shas = git.rev_list(start_rev)?;
    let mut events: Vec<Event> = Vec::new();
    let mut snapshot_ancestor: Option<String> = None;

    for sha in &shas {
        let listing = git.run(&["ls-tree", "--name-only", sha])?;
        let names: Vec<&str> = listing.lines().collect();
        if names.contains(&"thread.toml") {
            snapshot_ancestor = Some(sha.clone());
            break;
        }
        if names.contains(&"event.json") {
            events.push(read_event(git, sha)?);
        }
        // Unknown shapes (empty merge commits, etc.) are skipped —
        // same lenience as the mixed-chain replay walkers.
    }
    events.reverse(); // chronological
    Ok((events, snapshot_ancestor))
}

/// Returns `true` when the thread ref's bottom (oldest) commit cannot be
/// parsed as a valid `event.json`. Used by `doctor` and `prune-orphans` to
/// distinguish a structurally empty ref (manually-created Git ref under
/// `refs/forum/threads/`, or a history that lost its create event) from
/// mid-chain corruption that points at real damage to a once-valid thread.
///
/// An empty ref (no commits) is also reported as orphan.
pub fn is_orphan_ref(git: &GitOps, thread_id: &str) -> ForumResult<bool> {
    let ref_name = refs::thread_ref(thread_id);
    let shas = git.rev_list(&ref_name)?;
    let Some(oldest) = shas.last() else {
        return Ok(true);
    };
    Ok(read_event(git, oldest).is_err())
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

    // ---- ThreadStatus (Phase 2a: typed status) ----

    #[test]
    fn thread_status_parse_canonical_round_trip() {
        for s in [
            "draft",
            "open",
            "working",
            "review",
            "done",
            "rejected",
            "withdrawn",
            "deprecated",
        ] {
            let parsed = ThreadStatus::parse(s).expect(s);
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn thread_status_parse_strict_rejects_legacy() {
        // 1.x synonyms must NOT pass strict parse — that channel is
        // reserved for write-side rejection of unknown values.
        assert_eq!(ThreadStatus::parse("closed"), None);
        assert_eq!(ThreadStatus::parse("proposed"), None);
        assert_eq!(ThreadStatus::parse("under-review"), None);
    }

    #[test]
    fn thread_status_parse_lenient_folds_legacy() {
        assert_eq!(
            ThreadStatus::parse_lenient("closed"),
            Some(ThreadStatus::Done)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("accepted"),
            Some(ThreadStatus::Done)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("proposed"),
            Some(ThreadStatus::Open)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("under-review"),
            Some(ThreadStatus::Review)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("reviewing"),
            Some(ThreadStatus::Review)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("designing"),
            Some(ThreadStatus::Working)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("implementing"),
            Some(ThreadStatus::Working)
        );
        assert_eq!(
            ThreadStatus::parse_lenient("pending"),
            Some(ThreadStatus::Working)
        );
    }

    #[test]
    fn thread_status_parse_lenient_unknown_is_none() {
        assert_eq!(ThreadStatus::parse_lenient("garbage"), None);
        assert_eq!(ThreadStatus::parse_lenient(""), None);
    }

    #[test]
    fn thread_status_display_matches_as_str() {
        assert_eq!(format!("{}", ThreadStatus::Done), "done");
        assert_eq!(format!("{}", ThreadStatus::Withdrawn), "withdrawn");
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

    // ---- P2 #wlqhi8xh: Stored / Domain projection ----

    #[test]
    fn project_create_carries_payload_and_meta() {
        let event = sample_create_event();
        let dom = event.project().expect("create projects");
        match dom {
            DomainEvent::Create {
                meta,
                title,
                kind,
                body,
                branch,
            } => {
                assert_eq!(meta.event_id, event.event_id);
                assert_eq!(meta.thread_id, event.thread_id);
                assert_eq!(meta.event_type, EventType::Create);
                assert_eq!(meta.actor, event.actor);
                assert_eq!(title, "Test RFC");
                assert_eq!(kind, ThreadKind::Rfc);
                assert_eq!(body.as_deref(), Some("Thread body"));
                assert!(branch.is_none());
            }
            _ => panic!("expected Create variant"),
        }
    }

    #[test]
    fn project_say_requires_node_type_then_body() {
        // node_type missing comes first per the original apply_event arm
        // ordering; project must preserve that priority.
        let mut ev = sample_create_event();
        ev.event_type = EventType::Say;
        ev.title = None;
        ev.kind = None;
        ev.node_type = None;
        ev.body = None;
        let err = ev.project().expect_err("missing fields");
        assert_eq!(
            err,
            ProjectionError::MissingRequiredField { field: "node_type" }
        );
        ev.node_type = Some(NodeType::Comment);
        let err = ev.project().expect_err("missing body");
        assert_eq!(err, ProjectionError::MissingRequiredField { field: "body" });
        ev.body = Some("hi".into());
        let dom = ev.project().expect("now valid");
        assert!(matches!(dom, DomainEvent::Say { ref body, .. } if body == "hi"));
    }

    #[test]
    fn project_link_validates_either_evidence_or_target_pair() {
        let mut ev = sample_create_event();
        ev.event_type = EventType::Link;
        ev.title = None;
        ev.kind = None;
        ev.body = None;
        // neither evidence nor (target+rel)
        let err = ev.project().expect_err("link missing payload");
        assert_eq!(
            err,
            ProjectionError::MissingRequiredField {
                field: "evidence_or_target_link"
            }
        );
        // target without rel still fails (project mirrors apply_event's
        // both-or-neither rule).
        ev.target_node_id = Some("xyz".into());
        let err = ev.project().expect_err("link missing rel");
        assert_eq!(
            err,
            ProjectionError::MissingRequiredField {
                field: "evidence_or_target_link"
            }
        );
        ev.link_rel = Some("implements".into());
        let dom = ev.project().expect("link with target+rel");
        assert!(matches!(
            dom,
            DomainEvent::Link {
                payload: LinkPayload::Thread { .. },
                ..
            }
        ));
    }

    #[test]
    fn project_facetset_passes_through_unparsed_lifecycle() {
        // Projection keeps lifecycle as the raw stored string; apply_event
        // is the place that calls Lifecycle::parse and emits
        // InvalidLifecycleValue. Test that an unparseable string still
        // projects (so strict replay sees it).
        let mut ev = sample_create_event();
        ev.event_type = EventType::FacetSet;
        ev.title = None;
        ev.kind = None;
        ev.body = None;
        ev.lifecycle = Some("nonsense".into());
        let dom = ev.project().expect("facetset projects");
        match dom {
            DomainEvent::FacetSet { lifecycle, .. } => {
                assert_eq!(lifecycle.as_deref(), Some("nonsense"));
            }
            _ => panic!("expected FacetSet"),
        }
    }

    /// SPEC-2.0 §B.6 / ticket acceptance: projecting and re-serialising
    /// a stored Event must reproduce the original on-disk shape
    /// byte-for-byte. Confirms `project()` is non-destructive: it reads
    /// the bag without mutating it.
    #[test]
    fn stored_event_roundtrip_through_projection() {
        // Cover several representative variants that exercise different
        // optional-field combinations.
        let mut create = sample_create_event();
        create.branch = Some("main".into());
        let mut state_ev = sample_create_event();
        state_ev.event_type = EventType::State;
        state_ev.title = None;
        state_ev.kind = None;
        state_ev.body = None;
        state_ev.new_state = Some("done".into());
        state_ev.approvals = vec![Approval {
            actor_id: "human/alice".into(),
            approved_at: state_ev.created_at,
            mechanism: ApprovalMechanism::Recorded,
            key_id: None,
            proof_ref: None,
        }];
        let mut facet = sample_create_event();
        facet.event_type = EventType::FacetSet;
        facet.title = None;
        facet.kind = None;
        facet.body = None;
        facet.lifecycle = Some("execution".into());
        facet.tags_add = vec!["task".into()];
        facet.tags_remove = vec!["bug".into()];

        for original in [create, state_ev, facet] {
            let json_before = serde_json::to_string_pretty(&original).unwrap();
            // Round-trip through serde to drop the skip_serializing
            // event_id and pick up canonical field ordering.
            let reloaded: Event = serde_json::from_str(&json_before).unwrap();
            // Project once — must succeed and not mutate the bag.
            let _dom = reloaded.project().expect("projects");
            // Re-serialise the original (untouched) Event and compare
            // byte-for-byte.
            let json_after = serde_json::to_string_pretty(&reloaded).unwrap();
            assert_eq!(
                json_before, json_after,
                "projection mutated the stored event for {:?}",
                reloaded.event_type
            );
        }
    }
}
