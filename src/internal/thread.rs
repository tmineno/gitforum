use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{ForumError, ForumResult};
use super::event::{
    self, DomainEvent, Event, EventMeta, EventType, Lifecycle, LinkPayload, NodeType,
    ProjectionError,
};
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::node::Node;
use super::refs;
use super::validate::StrictReplayIssue;

pub const MIN_NODE_ID_PREFIX_LEN: usize = 4;

// --------------------------------------------------------------------
// `ThreadKind` (4-variant v2 enum) was relocated here from `event.rs`
// in Phase 4 Step 1g (RFC `7ymtc4b2`, task `913c4s9v`). Co-locating
// it with the other thread-shaped types lets KEEP files reach for a
// kind label without importing `internal::event`. The 3.0-native
// successor is the snapshot's `category` string (SPEC-3.0 §3.1);
// ThreadKind survives until Phase 4 Step 5 deletes the v2 peer types.
// --------------------------------------------------------------------

/// Thread kinds supported by git-forum (v2 4-variant enum).
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
    ///
    /// Routes through `SPEC::kind_lifecycle`, which sources from the
    /// kind preset table.
    pub fn lifecycle(self) -> Lifecycle {
        super::workflow::SPEC.kind_lifecycle(self)
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

// --------------------------------------------------------------------
// `ThreadStatus` (8-variant v2 enum + lenient parser) was relocated
// here from `event.rs` in Phase 4 Step 1h (RFC `7ymtc4b2`, task
// `913c4s9v`). Co-located with `ThreadKind` and the rest of the
// thread-shaped types so KEEP files don't need to import
// `internal::event` for status parsing. The 3.0-native successor is
// the snapshot's `status` string field; ThreadStatus survives until
// Phase 4 Step 5 deletes the v2 peer types.
// --------------------------------------------------------------------

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
    /// [`event::normalize_state_name`]. The lenient `apply_event` path
    /// uses this so legacy event chains keep replaying.
    pub fn parse_lenient(s: &str) -> Option<Self> {
        Self::parse(super::policy::normalize_state_name(s))
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

/// A link between two threads.
#[derive(Debug, Clone)]
pub struct ThreadLink {
    pub target_thread_id: String,
    pub rel: String,
}

/// Materialized state of a thread, derived from event replay.
///
/// `Default` is derived so test fixtures and helpers can construct partial
/// states with `ThreadState { id: …, kind: …, ..Default::default() }`,
/// matching the pattern used on `Event` and `Node`.
#[derive(Debug, Clone, Default)]
pub struct ThreadState {
    pub id: String,
    pub kind: ThreadKind,
    pub title: String,
    pub body: Option<String>,
    pub branch: Option<String>,
    /// Phase 2a: typed status. Storage (`Event.new_state`) stays
    /// `Option<String>` for 1.x compatibility; this field is the parsed,
    /// 2.0-canonical view used by every read path.
    pub status: ThreadStatus,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub events: Vec<Event>,
    /// All discussion nodes (say/edit/retract/resolve/reopen applied).
    pub nodes: Vec<Node>,
    /// Evidence items attached to this thread via Link events.
    pub evidence_items: Vec<Evidence>,
    /// Links to other threads via Link events.
    pub links: Vec<ThreadLink>,
    /// Number of times the thread body has been revised.
    pub body_revision_count: usize,
    /// Node IDs that have been incorporated into the body.
    pub incorporated_node_ids: Vec<String>,
    /// SPEC-2.0 §2.3.4 / §7.3: the thread's effective lifecycle.
    ///
    /// Phase 2c (Finding 1 follow-up): typed and always populated. Initial
    /// value is derived from
    /// [`super::legacy::v1::lifecycle_for_legacy_kind`] (the §2.3.3
    /// legacy mapping) at replay start; the first `facet_set` event
    /// carrying `lifecycle` overrides it and sets
    /// [`lifecycle_explicit`](Self::lifecycle_explicit).
    /// Subsequent `facet_set` events carrying a different value are
    /// silently ignored at replay (write-side rejection with
    /// `FacetTransitionDisallowed` is Track B's responsibility; strict
    /// replay surfaces `LifecycleResetAttempted`).
    pub lifecycle: Lifecycle,
    /// `true` iff a `facet_set` event in the chain explicitly wrote the
    /// lifecycle. `false` means the lifecycle is the kind-derived default.
    /// Used by write-side first-wins guards (`write_ops`) and by display
    /// surfaces that distinguish "explicitly chosen" from "inferred".
    pub lifecycle_explicit: bool,
    /// SPEC-2.0 §2.3.5: derived tag set after replaying every `facet_set`
    /// event in chain order. `tags_add` is applied before `tags_remove`
    /// within each event.
    pub tags: Vec<String>,
}

/// Resolved view of a single node inside a thread.
#[derive(Debug, Clone)]
pub struct NodeLookup {
    pub thread_id: String,
    pub thread_title: String,
    /// Phase 2b: kept on the lookup struct for storage compatibility,
    /// but no longer surfaced as the primary display label by `node show`.
    pub thread_kind: ThreadKind,
    /// Phase 2b: the canonical 2.0 classification axis. Populated from
    /// the parent thread's [`ThreadState::lifecycle`].
    pub thread_lifecycle: Lifecycle,
    pub thread_tags: Vec<String>,
    pub node: Node,
    pub links: Vec<ThreadLink>,
    pub events: Vec<Event>,
}

impl ThreadState {
    /// Open (unresolved, not retracted) objection nodes.
    pub fn open_objections(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == NodeType::Objection && n.is_open())
            .collect()
    }

    /// Open (unresolved, not retracted) action nodes.
    pub fn open_actions(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == NodeType::Action && n.is_open())
            .collect()
    }

    /// Direct replies to a given node.
    pub fn replies_to(&self, node_id: &str) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| n.reply_to.as_deref() == Some(node_id))
            .collect()
    }

    /// Most recent non-retracted summary node, if any.
    ///
    /// 2.0: matches both raw 1.x `Summary` nodes (legacy reads) and canonical
    /// `Comment` nodes whose `legacy_subtype = "summary"` (native 2.0 writes
    /// from `git forum summary` and migrated 1.x events).
    pub fn latest_summary(&self) -> Option<&Node> {
        self.nodes.iter().rfind(|n| {
            !n.retracted
                && (n.node_type == NodeType::Summary
                    || n.legacy_subtype.as_deref() == Some("summary"))
        })
    }
}

/// Replay events to reconstruct thread state (lenient).
///
/// Silently no-ops on conditions that strict replay would flag (unknown
/// target node, second `facet_set` lifecycle, etc.). Read-side callers want
/// best-effort; doctor / migration / tests want [`replay_strict`].
///
/// Precondition: `events` is in chronological order; first must be `Create`.
pub fn replay(events: &[Event]) -> ForumResult<ThreadState> {
    let (state, _issues) = replay_with_issues(events)?;
    Ok(state)
}

/// Replay events strictly, returning every silent-no-op as a
/// [`StrictReplayIssue`] alongside the final state.
///
/// The state machine is identical to lenient `replay()` (first-write-wins
/// lifecycle, dedup tags, etc.) — strict mode only **observes** the
/// no-ops; it does not abort on them. A fully clean replay returns an
/// empty issue vector.
pub fn replay_strict(events: &[Event]) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues(events)
}

/// Like [`replay_strict`] but skips the post-pass that suppresses
/// `InvalidTransition` issues whose chain tail has self-healed.
///
/// Used by the workflow-repair tool (#uu9wxn1d) to recover the offending
/// event id even on chains that the public `replay_strict` would have
/// reported as clean. Read-side callers (doctor, search, display) want
/// the suppressed view; only the repair tool needs the raw stream.
pub fn replay_strict_unsuppressed(
    events: &[Event],
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues_inner(events, /* suppress_self_healed = */ false)
}

fn replay_with_issues(events: &[Event]) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues_inner(events, true)
}

fn replay_with_issues_inner(
    events: &[Event],
    suppress_self_healed: bool,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let first = events
        .first()
        .ok_or_else(|| ForumError::StateMachine("no events to replay".into()))?;

    if first.event_type != EventType::Create {
        return Err(ForumError::StateMachine(
            "first event must be 'create'".into(),
        ));
    }

    // Project the create event up-front: the seed needs `kind` + `title`
    // and there is no graceful "skip Create" path (an empty thread is
    // not representable in 2.0).
    let create = first.project().map_err(|e| match e {
        ProjectionError::MissingRequiredField { field } => {
            ForumError::StateMachine(format!("create event missing '{field}'"))
        }
    })?;
    let DomainEvent::Create {
        kind,
        title,
        body,
        branch,
        ..
    } = create
    else {
        return Err(ForumError::StateMachine(
            "first event must be 'create'".into(),
        ));
    };

    // `kind.initial_status()` returns a hardcoded canonical literal
    // (`"draft"` / `"open"`); parse_lenient is total over this input.
    let initial_status = ThreadStatus::parse_lenient(kind.initial_status())
        .expect("kind.initial_status() always returns a canonical 2.0 status name");
    let mut state = ThreadState {
        id: first.thread_id.clone(),
        kind,
        title,
        body,
        branch,
        status: initial_status,
        created_at: first.created_at,
        created_by: first.actor.clone(),
        events: vec![first.clone()],
        nodes: vec![],
        evidence_items: vec![],
        links: vec![],
        body_revision_count: 0,
        incorporated_node_ids: vec![],
        // Phase 2c: lifecycle is always populated. Default is the §2.3.3
        // kind-derived value (a 1.x compat fallback for chains without
        // an explicit `facet_set`); the first explicit `facet_set` then
        // overrides it and flips `lifecycle_explicit` below.
        lifecycle: super::legacy::v1::lifecycle_for_legacy_kind(kind),
        lifecycle_explicit: false,
        tags: Vec::new(),
    };

    let mut issues = Vec::new();
    for ev in &events[1..] {
        // Push the stored event onto the running history regardless of
        // projection outcome — display surfaces still want to render
        // events whose payload is malformed.
        state.events.push(ev.clone());
        match ev.project() {
            Ok(domain) => apply_event(&mut state, &domain, &mut issues)?,
            Err(ProjectionError::MissingRequiredField { field }) => {
                issues.push(StrictReplayIssue::MissingRequiredField {
                    event_id: ev.event_id.clone(),
                    event_type: ev.event_type,
                    field,
                });
            }
        }
    }
    if suppress_self_healed {
        suppress_self_healed_invalid_transitions(events, &state, &mut issues);
    }
    Ok((state, issues))
}

/// SPEC-2.0 §3.1 / #uu9wxn1d: drop `InvalidTransition` issues whose offending
/// event has been "self-healed" by a subsequent legal corrective sequence.
///
/// A self-heal is recognised when:
/// 1. The chain's final terminal status equals the issue's `to` (the visible
///    state the operator intended).
/// 2. After the offending event, every subsequent `state` event is on a legal
///    edge for the lifecycle.
/// 3. The running state visits at least one non-`to` state and walks back to
///    `to` via legal edges (i.e. the corrective tail is non-trivial).
///
/// Without (3), a chain that simply stops at the offending event would
/// trivially pass — we want to require an explicit operator-emitted
/// corrective sequence (the pattern `state open` → `state rejected` for the
/// `draft → rejected` case). Threads whose terminal sits on a sink state
/// (`withdrawn` in proposal lifecycle) cannot self-heal via append-only
/// because no legal outgoing edge exists; those issues remain reported.
fn suppress_self_healed_invalid_transitions(
    events: &[Event],
    state: &ThreadState,
    issues: &mut Vec<StrictReplayIssue>,
) {
    issues.retain(|issue| {
        let StrictReplayIssue::InvalidTransition {
            event_id,
            to: target,
            ..
        } = issue
        else {
            return true;
        };
        if state.status.as_str() != target {
            return true;
        }
        let Some(idx) = events.iter().position(|e| &e.event_id == event_id) else {
            return true;
        };
        !is_self_healed_after(&events[idx + 1..], state.lifecycle, target)
    });
}

fn is_self_healed_after(tail: &[Event], lifecycle: super::event::Lifecycle, target: &str) -> bool {
    let Some(target_status) = ThreadStatus::parse_lenient(target) else {
        return false;
    };
    let mut running = target_status;
    let mut left_target = false;
    for ev in tail {
        if ev.event_type != EventType::State {
            continue;
        }
        let Some(name) = ev.new_state.as_deref() else {
            continue;
        };
        let Some(parsed) = ThreadStatus::parse_lenient(name) else {
            return false;
        };
        if parsed == running {
            continue;
        }
        if !super::workflow::SPEC.is_valid_transition(lifecycle, running.as_str(), parsed.as_str())
        {
            return false;
        }
        running = parsed;
        if running.as_str() != target {
            left_target = true;
        }
        if left_target && running.as_str() == target {
            return true;
        }
    }
    false
}

fn apply_event(
    state: &mut ThreadState,
    event: &DomainEvent,
    issues: &mut Vec<StrictReplayIssue>,
) -> ForumResult<()> {
    match event {
        DomainEvent::State {
            meta,
            new_state,
            approvals,
        } => {
            match ThreadStatus::parse_lenient(new_state) {
                Some(parsed) => {
                    // SPEC-2.0 §3.1 (P0 #34ith16h): strict mode flags
                    // an illegal `from -> to` for the thread's
                    // lifecycle on the per-lifecycle filtered graph.
                    // Lenient mode applies the new status regardless
                    // so legacy chains keep replaying.
                    let from = state.status;
                    if from != parsed
                        && !super::workflow::SPEC.is_valid_transition(
                            state.lifecycle,
                            from.as_str(),
                            parsed.as_str(),
                        )
                    {
                        issues.push(StrictReplayIssue::InvalidTransition {
                            event_id: meta.event_id.clone(),
                            from: from.as_str().to_string(),
                            to: parsed.as_str().to_string(),
                            lifecycle: state.lifecycle.as_str().to_string(),
                        });
                    }
                    state.status = parsed;
                }
                // Lenient: keep the prior status. Strict mode surfaces
                // the unparseable value below.
                None => issues.push(StrictReplayIssue::InvalidStateValue {
                    event_id: meta.event_id.clone(),
                    value: new_state.clone(),
                }),
            }
            // SPEC-2.0 §2.8: 1.x State events carried approvals as a direct
            // field; 2.0 emits them as Approval-typed Say nodes. Synthesize
            // equivalent nodes here so policy guards see one source of truth.
            for approval in approvals {
                state.nodes.push(Node {
                    node_id: format!("{}#{}", meta.event_id, approval.actor_id),
                    node_type: NodeType::Approval,
                    body: String::new(),
                    actor: approval.actor_id.clone(),
                    created_at: approval.approved_at,
                    ..Node::default()
                });
            }
        }
        DomainEvent::Scope { branch, .. } => {
            // `branch = None` legitimately clears scope; lenient and strict agree.
            state.branch.clone_from(branch);
        }
        DomainEvent::Say {
            meta,
            node_type,
            body,
            reply_to,
            legacy_subtype,
            target_node_id,
        } => {
            state.nodes.push(Node {
                // 2.0: a Say node id is the writer's pre-allocated
                // `target_node_id` when present, else the event's
                // commit SHA (the common case for native 2.0 writes).
                node_id: target_node_id
                    .clone()
                    .unwrap_or_else(|| meta.event_id.clone()),
                node_type: *node_type,
                body: body.clone(),
                actor: meta.actor.clone(),
                created_at: meta.created_at,
                resolved: false,
                retracted: false,
                incorporated: false,
                reply_to: reply_to.clone(),
                legacy_subtype: legacy_subtype.clone(),
            });
        }
        DomainEvent::Edit {
            meta,
            target_node_id,
            body,
        } => {
            if let Some(node) = state
                .nodes
                .iter_mut()
                .find(|n| &n.node_id == target_node_id)
            {
                node.body = body.clone();
            } else {
                issues.push(StrictReplayIssue::UnknownTargetNode {
                    event_id: meta.event_id.clone(),
                    event_type: meta.event_type,
                    target_node_id: target_node_id.clone(),
                });
            }
        }
        DomainEvent::Retract {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| n.retracted = true),
        DomainEvent::Resolve {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| n.resolved = true),
        DomainEvent::Reopen {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| {
            n.resolved = false;
            n.retracted = false;
            n.incorporated = false;
        }),
        DomainEvent::Retype {
            meta,
            target_node_id,
            node_type,
            ..
        } => {
            if let Some(node) = state
                .nodes
                .iter_mut()
                .find(|n| &n.node_id == target_node_id)
            {
                node.node_type = *node_type;
            } else {
                issues.push(StrictReplayIssue::UnknownTargetNode {
                    event_id: meta.event_id.clone(),
                    event_type: meta.event_type,
                    target_node_id: target_node_id.clone(),
                });
            }
        }
        DomainEvent::ReviseBody {
            meta,
            body,
            incorporated_node_ids,
        } => {
            state.body = Some(body.clone());
            state.body_revision_count += 1;
            for node_id in incorporated_node_ids {
                let found = state
                    .nodes
                    .iter_mut()
                    .find(|n| n.node_id == *node_id)
                    .map(|node| node.incorporated = true);
                if found.is_none() {
                    issues.push(StrictReplayIssue::UnknownTargetNode {
                        event_id: meta.event_id.clone(),
                        event_type: meta.event_type,
                        target_node_id: node_id.clone(),
                    });
                }
                if !state.incorporated_node_ids.contains(node_id) {
                    state.incorporated_node_ids.push(node_id.clone());
                }
            }
        }
        DomainEvent::Link { meta, payload } => match payload {
            LinkPayload::Evidence(ev_data) => {
                let mut ev = ev_data.clone();
                ev.evidence_id = meta.event_id.clone();
                state.evidence_items.push(ev);
            }
            LinkPayload::Thread {
                target_thread_id,
                link_rel,
            } => {
                state.links.push(ThreadLink {
                    target_thread_id: target_thread_id.clone(),
                    rel: link_rel.clone(),
                });
            }
        },
        // No-ops during replay:
        DomainEvent::Create { .. } => {} // handled in replay() seed before apply_event loop
        DomainEvent::Verify { .. } | DomainEvent::Merge { .. } => {}
        // ADR-010 option (a): unknown variants no-op + emit a strict
        // issue. Unreachable in Phase A; Phase B wires the
        // `EventType::Other(String)` deserialiser to this arm.
        DomainEvent::Unknown { meta, .. } => {
            issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: meta.event_id.clone(),
                event_type: meta.event_type,
                field: "unknown_event_type",
            });
        }
        // SPEC-2.0 §2.4.1: per-event facet mutation, not full-state
        // replacement.
        DomainEvent::FacetSet {
            meta,
            lifecycle,
            tags_add,
            tags_remove,
        } => {
            // First-lifecycle-wins: §7.3 makes lifecycle immutable, so any
            // subsequent facet_set carrying `lifecycle` is silently ignored
            // at replay (write-side rejection with FacetTransitionDisallowed
            // is Track B's responsibility).
            if let Some(lc) = lifecycle {
                let parsed = Lifecycle::parse(lc);
                if parsed.is_none() {
                    issues.push(StrictReplayIssue::InvalidLifecycleValue {
                        event_id: meta.event_id.clone(),
                        value: lc.clone(),
                    });
                }
                if let Some(parsed_lc) = parsed {
                    if !state.lifecycle_explicit {
                        // First explicit facet_set wins. Override the
                        // kind-derived default.
                        state.lifecycle = parsed_lc;
                        state.lifecycle_explicit = true;
                    } else if state.lifecycle != parsed_lc {
                        issues.push(StrictReplayIssue::LifecycleResetAttempted {
                            event_id: meta.event_id.clone(),
                            existing: state.lifecycle.as_str().to_string(),
                            attempted: lc.clone(),
                        });
                    }
                    // else: idempotent re-set with the same value — no-op.
                }
            }
            // Within a single event, tags_add is applied before tags_remove
            // (an event that simultaneously adds and removes the same tag
            // is a removal). Insertion is set-style (no duplicates).
            //
            // Tag-grammar validation happens at the migration boundary
            // (`commands::migrate::project_state_to_doc`), NOT here:
            // legacy display surfaces want to render tags verbatim even
            // when they violate the 3.0 grammar (e.g. a 1-char tag
            // accepted by an earlier loose validator). Migration drops
            // invalid tags and records them as `kind: "tag"` omissions
            // in the report (task `9635buy0` objection `e285682f`).
            for tag in tags_add {
                if !state.tags.iter().any(|t| t == tag) {
                    state.tags.push(tag.clone());
                }
            }
            for tag in tags_remove {
                state.tags.retain(|t| t != tag);
            }
        }
    }
    Ok(())
}

/// Shared helper for `Retract` / `Resolve` / `Reopen`: locate the target
/// node by id, apply `mutate`, or record an
/// [`StrictReplayIssue::UnknownTargetNode`]. Projection has already
/// guaranteed a present `target_node_id` for these variants.
fn apply_node_flag(
    state: &mut ThreadState,
    meta: &EventMeta,
    target_node_id: &str,
    issues: &mut Vec<StrictReplayIssue>,
    mutate: impl FnOnce(&mut Node),
) {
    if let Some(node) = state.nodes.iter_mut().find(|n| n.node_id == target_node_id) {
        mutate(node);
    } else {
        issues.push(StrictReplayIssue::UnknownTargetNode {
            event_id: meta.event_id.clone(),
            event_type: meta.event_type,
            target_node_id: target_node_id.to_string(),
        });
    }
}

/// Walk the chain at `refs/forum/threads/<id>` oldest→newest and
/// project a [`ThreadState`].
///
/// Phase 2 transition (RFC `7ymtc4b2`): every commit's tree is
/// classified as a SPEC-3.0 *snapshot commit* (`thread.toml` blob) or
/// a v1/v2 *event commit* (`event.json` blob). The reader copes with
/// any mixture:
///
/// - **Pure snapshot** — single snapshot commit at tip → state is
///   materialized from the [`ThreadDocument`](super::snapshot::ThreadDocument).
/// - **Pure event chain** — every commit carries `event.json` →
///   delegated to [`replay`] over the loaded events.
/// - **Mixed (snapshot bottom, events on top)** — arises during the
///   Phase 2 cutover window: e.g. `git forum new` (slot-1 snapshot
///   write) followed by `git forum comment` (slot-2 event write,
///   pre-cutover). The snapshot at chain bottom seeds state; the
///   event tail is folded in via [`apply_event`].
/// - **Multiple snapshot commits** — each later snapshot supersedes
///   the earlier (the tree IS the cumulative state); accumulated
///   tail events are discarded when a fresh snapshot resets state.
///
/// This dispatch disappears in Phase 4 along with the event-chain
/// reader.
pub fn replay_thread(git: &GitOps, thread_id: &str) -> ForumResult<ThreadState> {
    let refname = format!("refs/forum/threads/{thread_id}");
    let tip = git
        .resolve_ref(&refname)?
        .ok_or_else(|| ForumError::Repo(format!("thread {thread_id} not found")))?;
    replay_thread_at(git, &tip)
}

/// Like [`replay_thread`], but walks from a caller-supplied rev
/// (typically a captured commit OID) instead of resolving the live
/// thread ref. Used by migrate to pin the legacy-chain replay
/// against the exact tip recorded for the eventual CAS write so a
/// concurrent event landing between read and write fails the CAS
/// instead of silently dropping events from the projected snapshot
/// (task `9635buy0`, objection `e630f01f`).
pub fn replay_thread_at(git: &GitOps, start_rev: &str) -> ForumResult<ThreadState> {
    // `rev_list` returns newest-first; replay needs oldest-first.
    let mut shas: Vec<String> = git.rev_list(start_rev)?;
    shas.reverse();

    let mut state: Option<ThreadState> = None;
    let mut tail_events: Vec<Event> = Vec::new();
    let mut issues: Vec<StrictReplayIssue> = Vec::new();

    for sha in &shas {
        let listing = git.run(&["ls-tree", "--name-only", sha])?;
        let names: Vec<&str> = listing.lines().collect();
        if names.contains(&"thread.toml") {
            // SPEC-3.0 snapshot commit — reset state to this snapshot's
            // view. Any prior tail events are subsumed.
            let doc = super::snapshot::read_snapshot_at(git, sha)?;
            state = Some(materialize_thread_state_from_snapshot(doc));
            tail_events.clear();
        } else if names.contains(&"event.json") {
            // Legacy v1/v2 event commit — accumulate for projection.
            tail_events.push(event::read_event(git, sha)?);
        }
        // Unknown tree shapes (e.g. an empty merge) are skipped; they
        // do not affect state under either storage model.
    }

    if let Some(mut s) = state {
        // Apply any events that landed AFTER the most recent snapshot.
        for ev in &tail_events {
            s.events.push(ev.clone());
            match ev.project() {
                Ok(domain) => apply_event(&mut s, &domain, &mut issues)?,
                Err(ProjectionError::MissingRequiredField { .. }) => {
                    // Lenient mode: a malformed event is silently
                    // skipped. Strict callers route through
                    // `replay_thread_strict` which surfaces this as
                    // `MissingRequiredField`.
                }
            }
        }
        Ok(s)
    } else if !tail_events.is_empty() {
        // No snapshot seed — pure legacy event chain.
        replay(&tail_events)
    } else {
        Err(ForumError::Repo(format!(
            "rev {start_rev} has no replayable content"
        )))
    }
}

/// Materialize a legacy [`ThreadState`] view from a SPEC-3.0
/// [`ThreadDocument`](super::snapshot::ThreadDocument).
///
/// Phase 2 bridge: read paths still consume `ThreadState`. Until each
/// command is cut over to read snapshots directly (slots 7a–7k), the
/// snapshot-tip case is translated into the legacy struct. `events`
/// is left empty — the snapshot model has no event chain, and the
/// surfaces that display events (legacy `log`, domain timeline) are
/// on the Phase 4 DELETE list.
fn materialize_thread_state_from_snapshot(doc: super::snapshot::ThreadDocument) -> ThreadState {
    use super::evidence::Evidence;
    use super::node::{NodeKind, NodeStatus};
    use super::snapshot::ThreadDocument;

    let ThreadDocument {
        snapshot,
        body,
        nodes,
        links,
        evidence,
    } = doc;

    let kind = category_to_legacy_kind(&snapshot.category, &snapshot.tags);
    let lifecycle = super::legacy::v1::lifecycle_for_legacy_kind(kind);
    let status = ThreadStatus::parse_lenient(&snapshot.status).unwrap_or_default();

    let nodes: Vec<Node> = nodes
        .into_iter()
        .map(|n| Node {
            node_id: n.record.id,
            node_type: match n.record.kind {
                NodeKind::Comment => NodeType::Comment,
                NodeKind::Approval => NodeType::Approval,
                NodeKind::Objection => NodeType::Objection,
                NodeKind::Action => NodeType::Action,
            },
            body: n.body,
            actor: n.record.created_by,
            created_at: n.record.created_at,
            resolved: matches!(n.record.status, NodeStatus::Resolved),
            retracted: matches!(n.record.status, NodeStatus::Retracted),
            incorporated: matches!(n.record.status, NodeStatus::Incorporated),
            reply_to: n.record.reply_to,
            legacy_subtype: n.record.legacy_label,
        })
        .collect();

    let links: Vec<ThreadLink> = links
        .entries
        .into_iter()
        .map(|l| ThreadLink {
            target_thread_id: l.target,
            rel: l.rel,
        })
        .collect();

    let evidence_items: Vec<Evidence> = evidence
        .entries
        .into_iter()
        .map(|e| Evidence {
            evidence_id: e.id,
            kind: e.kind,
            ref_target: e.ref_target,
            locator: None,
        })
        .collect();

    ThreadState {
        id: snapshot.id,
        kind,
        title: snapshot.title,
        body,
        branch: snapshot.branch,
        status,
        created_at: snapshot.created_at,
        created_by: snapshot.created_by,
        events: Vec::new(),
        nodes,
        evidence_items,
        links,
        body_revision_count: 0,
        incorporated_node_ids: Vec::new(),
        lifecycle,
        lifecycle_explicit: true,
        tags: snapshot.tags,
    }
}

/// Map a SPEC-3.0 category + tag set back to a legacy [`ThreadKind`]
/// for `ThreadState` materialization.
///
/// Phase 2 transitional: the v2 kind axis (Rfc/Dec/Issue/Task) is
/// folded onto SPEC-3.0's two built-in categories (`rfc`, `task`).
/// The kind is recovered from the canonical tag fingerprint defined
/// by SPEC-3.0 §8.3:
///
/// - `task` + `decision` → `Dec` (record lifecycle)
/// - `task` + `bug`      → `Issue`
/// - `task` otherwise    → `Task`
/// - `rfc`               → `Rfc`
fn category_to_legacy_kind(category: &str, tags: &[String]) -> ThreadKind {
    match category {
        "rfc" => ThreadKind::Rfc,
        "task" => {
            if tags.iter().any(|t| t == "decision") {
                ThreadKind::Dec
            } else if tags.iter().any(|t| t == "bug") {
                ThreadKind::Issue
            } else {
                ThreadKind::Task
            }
        }
        _ => ThreadKind::Issue,
    }
}

/// Load events from Git and replay strictly, returning every silent-no-op
/// alongside the materialized state. See [`replay_strict`].
pub fn replay_thread_strict(
    git: &GitOps,
    thread_id: &str,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let events = event::load_thread_events(git, thread_id)?;
    replay_strict(&events)
}

/// Like [`replay_thread_strict`], but reads events from a caller-
/// supplied rev (typically a captured tip OID) instead of resolving
/// the live thread ref. Used by migrate so the strict replay walks
/// the same chain that the eventual CAS write will guard against —
/// `replay_thread_strict` against the live ref would re-introduce
/// the read/write race fixed by objection `e630f01f`.
///
/// Mirrors the mixed-chain walk of [`replay_thread_at`]: a chain
/// whose bottom is a SPEC-3.0 snapshot commit (Phase-2 cutover
/// shape) seeds state from the snapshot and applies any event tail
/// strictly. A pure-event chain routes through [`replay_strict`].
/// `read_snapshot` only inspects the tip tree, so a tip that is an
/// event commit dispatches through migrate even when an ancestor
/// is a snapshot — the `_at` reader MUST handle that case
/// (task `9635buy0`, objection `bf678561`).
pub fn replay_thread_strict_at(
    git: &GitOps,
    start_rev: &str,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let mut shas: Vec<String> = git.rev_list(start_rev)?;
    shas.reverse();

    let mut state: Option<ThreadState> = None;
    let mut tail_events: Vec<Event> = Vec::new();

    for sha in &shas {
        let listing = git.run(&["ls-tree", "--name-only", sha])?;
        let names: Vec<&str> = listing.lines().collect();
        if names.contains(&"thread.toml") {
            // SPEC-3.0 snapshot ancestor — reset state to its view.
            // Anything before is subsumed; the snapshot's tags/links
            // were already validated at write time, so they enter
            // the strict path without further checks.
            let doc = super::snapshot::read_snapshot_at(git, sha)?;
            state = Some(materialize_thread_state_from_snapshot(doc));
            tail_events.clear();
        } else if names.contains(&"event.json") {
            tail_events.push(event::read_event(git, sha)?);
        }
        // Unknown tree shapes (empty merges, etc.) are skipped —
        // same lenience as `replay_thread_at`.
    }

    if let Some(mut s) = state {
        // Snapshot-bottom + event-tail. Apply tail events strictly.
        let mut issues = Vec::new();
        for ev in &tail_events {
            s.events.push(ev.clone());
            match ev.project() {
                Ok(domain) => apply_event(&mut s, &domain, &mut issues)?,
                Err(super::event::ProjectionError::MissingRequiredField { field }) => {
                    issues.push(StrictReplayIssue::MissingRequiredField {
                        event_id: ev.event_id.clone(),
                        event_type: ev.event_type,
                        field,
                    });
                }
            }
        }
        Ok((s, issues))
    } else if !tail_events.is_empty() {
        // Pure legacy event chain.
        replay_strict(&tail_events)
    } else {
        Err(ForumError::Repo(format!(
            "rev {start_rev} has no replayable content"
        )))
    }
}

/// Resolve a node reference across all threads.
///
/// Exact matches are preferred. If there is no exact match, a unique prefix
/// of at least [`MIN_NODE_ID_PREFIX_LEN`] characters is accepted.
pub fn resolve_node_id_global(git: &GitOps, node_ref: &str) -> ForumResult<String> {
    let lookups = all_node_lookups(git)?;
    resolve_node_id_global_from_lookups(&lookups, node_ref)
}

/// Resolve a node reference inside a single thread.
///
/// Exact matches are preferred. If there is no exact match, a unique prefix
/// of at least [`MIN_NODE_ID_PREFIX_LEN`] characters is accepted.
pub fn resolve_node_id_in_thread(
    git: &GitOps,
    thread_id: &str,
    node_ref: &str,
) -> ForumResult<String> {
    let state = replay_thread(git, thread_id)?;

    let exact_matches: Vec<&Node> = state
        .nodes
        .iter()
        .filter(|node| node.node_id == node_ref)
        .collect();
    match exact_matches.len() {
        1 => return Ok(exact_matches[0].node_id.clone()),
        2.. => {
            return Err(ForumError::Repo(format!(
                "node '{node_ref}' is ambiguous in thread '{thread_id}'"
            )));
        }
        0 => {}
    }

    if node_ref.len() < MIN_NODE_ID_PREFIX_LEN {
        return Err(ForumError::Repo(format!(
            "node id prefix '{node_ref}' is too short; use at least {MIN_NODE_ID_PREFIX_LEN} characters"
        )));
    }

    let matches: Vec<&Node> = state
        .nodes
        .iter()
        .filter(|node| node.node_id.starts_with(node_ref))
        .collect();
    match matches.len() {
        0 => Err(ForumError::Repo(format!(
            "node '{node_ref}' not found in thread '{thread_id}'"
        ))),
        1 => Ok(matches[0].node_id.clone()),
        _ => Err(ForumError::Repo(format_thread_ambiguity(
            thread_id, node_ref, &matches,
        ))),
    }
}

/// Find a node by ID across all threads.
pub fn find_node(git: &GitOps, node_ref: &str) -> ForumResult<NodeLookup> {
    let resolved = resolve_node_id_global(git, node_ref)?;
    let lookups = all_node_lookups(git)?;
    lookups
        .into_iter()
        .find(|lookup| lookup.node.node_id == resolved)
        .ok_or_else(|| ForumError::Repo(format!("node '{resolved}' not found")))
}

/// Find a node by ID inside a single thread.
pub fn find_node_in_thread(
    git: &GitOps,
    thread_id: &str,
    node_ref: &str,
) -> ForumResult<NodeLookup> {
    let state = replay_thread(git, thread_id)?;
    let resolved = resolve_node_id_in_thread(git, thread_id, node_ref)?;
    state
        .nodes
        .iter()
        .find(|node| node.node_id == resolved)
        .map(|node| build_node_lookup(&state, node))
        .ok_or_else(|| {
            ForumError::Repo(format!(
                "node '{resolved}' not found in thread '{thread_id}'"
            ))
        })
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

/// Resolve a user-supplied thread reference to a canonical full thread ID.
///
/// Accepts (per SPEC-2.0 §6.1.1 / §6.2):
/// - 2.0 display form (e.g. `@a7f3b2x1`) — leading `@` is stripped before matching
/// - 2.0 bare token (e.g. `a7f3b2x1`)
/// - Legacy full ID (e.g. `RFC-0001`, `ASK-a7f3b2x1`) — resolved either via a
///   live ref or the post-migration alias table (`refs/forum/aliases/<old-id>`)
/// - KIND-prefix (e.g. `RFC-a7f3`) — expanded if unambiguous
/// - Token-only prefix (e.g. `a7f3`) — matched against all thread IDs if unambiguous
/// - Case-insensitive variants of the above (e.g. `rfc-0001` resolves to `RFC-0001`)
///
/// Returns an error if the reference is ambiguous (with candidates listed)
/// or if no matching thread is found.
pub fn resolve_thread_id(git: &GitOps, user_input: &str) -> ForumResult<String> {
    let all_ids = list_thread_ids(git)?;
    match resolve_from_list(&all_ids, user_input) {
        Ok(id) => Ok(id),
        Err(direct_err) => {
            if let Some(token) = resolve_alias(git, user_input)? {
                Ok(token)
            } else {
                Err(direct_err)
            }
        }
    }
}

/// Look up `user_input` in the alias table populated by `git forum migrate`
/// (SPEC-2.0 §10.1). Returns the canonical bare-token thread ID, or `None`
/// if no alias matches.
///
/// Resolution path: confirm the alias ref exists, then derive the canonical
/// token from the legacy ID via the migrator's deterministic mapping
/// (`migrate::bare_token_for`). We deliberately do NOT chase the alias's tip
/// SHA — the canonical thread ref moves forward as new events are appended,
/// while the alias ref is frozen at the migration-time tip; following SHAs
/// would mean alias resolution stops working as soon as the migrated thread
/// receives any new event.
fn resolve_alias(git: &GitOps, user_input: &str) -> ForumResult<Option<String>> {
    let stripped = super::id::strip_thread_marker(user_input);
    if let Some(token) = canonical_for_legacy_id(git, stripped)? {
        return Ok(Some(token));
    }
    resolve_alias_case_insensitive(git, stripped)
}

fn canonical_for_legacy_id(git: &GitOps, legacy_id: &str) -> ForumResult<Option<String>> {
    if git
        .resolve_ref(&super::commands::migrate::alias_ref(legacy_id))?
        .is_none()
    {
        return Ok(None);
    }
    let token = super::commands::migrate::bare_token_for(legacy_id);
    if git.resolve_ref(&refs::thread_ref(&token))?.is_some() {
        Ok(Some(token))
    } else {
        Ok(None)
    }
}

fn resolve_alias_case_insensitive(git: &GitOps, user_input: &str) -> ForumResult<Option<String>> {
    let aliases = git.list_refs(super::commands::migrate::ALIASES_PREFIX)?;
    let target = user_input.to_ascii_uppercase();
    let mut hits: Vec<String> = aliases
        .iter()
        .filter_map(|r| r.strip_prefix(super::commands::migrate::ALIASES_PREFIX))
        .filter(|name| name.to_ascii_uppercase() == target)
        .map(|s| s.to_string())
        .collect();
    hits.sort();
    let alias = match hits.len() {
        0 => return Ok(None),
        1 => hits.remove(0),
        _ => {
            return Err(ForumError::Repo(format!(
                "ambiguous legacy alias '{user_input}'; candidates:\n  {}",
                hits.join("\n  ")
            )));
        }
    };
    canonical_for_legacy_id(git, &alias)
}

/// Pure resolution logic for testability — matches user input against a list of
/// known thread IDs using exact, prefix, token, and case-insensitive strategies.
fn resolve_from_list(all_ids: &[String], user_input: &str) -> ForumResult<String> {
    // 0. Strip the SPEC-2.0 §6.1 `@` thread marker if the user typed the
    //    display form. Refs and serialized fields are always bare.
    let user_input = super::id::strip_thread_marker(user_input);

    // 1. Exact match
    if all_ids.iter().any(|id| id == user_input) {
        return Ok(user_input.to_string());
    }

    // 2. KIND-prefix match (e.g. "RFC-a7f3" matches "RFC-a7f3b2x1")
    if user_input.contains('-') {
        let matches: Vec<&String> = all_ids
            .iter()
            .filter(|id| id.starts_with(user_input))
            .collect();
        match matches.len() {
            0 => {} // fall through to token-only
            1 => return Ok(matches[0].clone()),
            _ => {
                let candidates: Vec<&str> = matches.iter().map(|s| s.as_str()).collect();
                return Err(ForumError::Repo(format!(
                    "ambiguous thread reference '{user_input}'; candidates:\n  {}",
                    candidates.join("\n  ")
                )));
            }
        }
    }

    // 3. Token-only match (e.g. "a7f3b2x1" matches "RFC-a7f3b2x1" or 2.0 bare "a7f3b2x1")
    if !user_input.contains('-') {
        let matches: Vec<&String> = all_ids
            .iter()
            .filter(|id| match id.split_once('-') {
                Some((_, token)) => token.starts_with(user_input),
                // 2.0 bare-token storage: the whole id is the token.
                None => id.starts_with(user_input),
            })
            .collect();
        match matches.len() {
            1 => return Ok(matches[0].clone()),
            n if n > 1 => {
                let candidates: Vec<&str> = matches.iter().map(|s| s.as_str()).collect();
                return Err(ForumError::Repo(format!(
                    "ambiguous thread reference '{user_input}'; candidates:\n  {}",
                    candidates.join("\n  ")
                )));
            }
            _ => {}
        }
    }

    // 4. Case-insensitive exact match (e.g. "rfc-0001" matches "RFC-0001")
    let input_upper = user_input.to_ascii_uppercase();
    let ci_matches: Vec<&String> = all_ids
        .iter()
        .filter(|id| id.to_ascii_uppercase() == input_upper)
        .collect();
    match ci_matches.len() {
        1 => return Ok(ci_matches[0].clone()),
        n if n > 1 => {
            let candidates: Vec<&str> = ci_matches.iter().map(|s| s.as_str()).collect();
            return Err(ForumError::Repo(format!(
                "ambiguous thread reference '{user_input}'; did you mean one of:\n  {}",
                candidates.join("\n  ")
            )));
        }
        _ => {}
    }

    // 5. Case-insensitive prefix match (e.g. "rfc-a7f3" matches "RFC-a7f3b2x1")
    if user_input.contains('-') {
        let ci_prefix_matches: Vec<&String> = all_ids
            .iter()
            .filter(|id| id.to_ascii_uppercase().starts_with(&input_upper))
            .collect();
        match ci_prefix_matches.len() {
            0 => {}
            1 => return Ok(ci_prefix_matches[0].clone()),
            _ => {
                let candidates: Vec<&str> = ci_prefix_matches.iter().map(|s| s.as_str()).collect();
                return Err(ForumError::Repo(format!(
                    "ambiguous thread reference '{user_input}'; did you mean one of:\n  {}",
                    candidates.join("\n  ")
                )));
            }
        }
    }

    // 6. Case-insensitive token match (e.g. "A7F3B2X1" matches "RFC-a7f3b2x1" or bare "a7f3b2x1")
    if !user_input.contains('-') {
        let ci_token_matches: Vec<&String> = all_ids
            .iter()
            .filter(|id| match id.split_once('-') {
                Some((_, token)) => token.to_ascii_uppercase().starts_with(&input_upper),
                None => id.to_ascii_uppercase().starts_with(&input_upper),
            })
            .collect();
        match ci_token_matches.len() {
            1 => return Ok(ci_token_matches[0].clone()),
            n if n > 1 => {
                let candidates: Vec<&str> = ci_token_matches.iter().map(|s| s.as_str()).collect();
                return Err(ForumError::Repo(format!(
                    "ambiguous thread reference '{user_input}'; did you mean one of:\n  {}",
                    candidates.join("\n  ")
                )));
            }
            _ => {}
        }
    }

    Err(ForumError::Repo(format!(
        "thread '{user_input}' not found\n  hint: run `git forum ls` to see all threads"
    )))
}

fn all_node_lookups(git: &GitOps) -> ForumResult<Vec<NodeLookup>> {
    let mut lookups = Vec::new();
    for thread_id in list_thread_ids(git)? {
        let state = replay_thread(git, &thread_id)?;
        for node in &state.nodes {
            lookups.push(build_node_lookup(&state, node));
        }
    }
    Ok(lookups)
}

fn build_node_lookup(state: &ThreadState, node: &Node) -> NodeLookup {
    let events = state
        .events
        .iter()
        .filter(|ev| event_references_node(ev, node.node_id.as_str()))
        .cloned()
        .collect();
    NodeLookup {
        thread_id: state.id.clone(),
        thread_title: state.title.clone(),
        thread_kind: state.kind,
        thread_lifecycle: state.lifecycle,
        thread_tags: state.tags.clone(),
        node: node.clone(),
        links: state.links.clone(),
        events,
    }
}

fn say_node_id(event: &Event) -> &str {
    event
        .target_node_id
        .as_deref()
        .unwrap_or(event.event_id.as_str())
}

fn event_references_node(event: &Event, node_id: &str) -> bool {
    match event.event_type {
        EventType::Say => say_node_id(event) == node_id,
        _ => event.target_node_id.as_deref() == Some(node_id),
    }
}

fn resolve_node_id_global_from_lookups(
    lookups: &[NodeLookup],
    node_ref: &str,
) -> ForumResult<String> {
    let exact_matches: Vec<&NodeLookup> = lookups
        .iter()
        .filter(|lookup| lookup.node.node_id == node_ref)
        .collect();
    match exact_matches.len() {
        1 => return Ok(exact_matches[0].node.node_id.clone()),
        2.. => {
            return Err(ForumError::Repo(format!(
                "node '{node_ref}' is ambiguous across multiple threads"
            )));
        }
        0 => {}
    }

    if node_ref.len() < MIN_NODE_ID_PREFIX_LEN {
        return Err(ForumError::Repo(format!(
            "node id prefix '{node_ref}' is too short; use at least {MIN_NODE_ID_PREFIX_LEN} characters"
        )));
    }

    let matches: Vec<&NodeLookup> = lookups
        .iter()
        .filter(|lookup| lookup.node.node_id.starts_with(node_ref))
        .collect();
    match matches.len() {
        0 => Err(ForumError::Repo(format!("node '{node_ref}' not found"))),
        1 => Ok(matches[0].node.node_id.clone()),
        _ => Err(ForumError::Repo(format_global_ambiguity(
            node_ref, &matches,
        ))),
    }
}

fn format_thread_ambiguity(thread_id: &str, node_ref: &str, matches: &[&Node]) -> String {
    let mut message = format!("node id prefix '{node_ref}' is ambiguous in thread '{thread_id}'");
    message.push_str("\n  candidates:");
    for node in matches {
        message.push_str(&format!("\n  - {}  {}", node.node_id, node.node_type));
    }
    message
}

fn format_global_ambiguity(node_ref: &str, matches: &[&NodeLookup]) -> String {
    let mut message = format!("node id prefix '{node_ref}' is ambiguous");
    message.push_str("\n  candidates:");
    for lookup in matches {
        message.push_str(&format!(
            "\n  - {}  {} {}",
            lookup.node.node_id, lookup.thread_id, lookup.node.node_type
        ));
    }
    message
}

// --------------------------------------------------------------------
// SPEC-3.0 §2.1 + §4.2 `thread.toml` shape.
//
// `ThreadSnapshot` is the 3.0-native thread metadata type, distinct
// from the legacy `ThreadState` (which models replayed event-chain
// state). Body text is stored separately as `body.md` per SPEC-3.0
// §2.1 so plain Git diffs are useful; this type does NOT carry the
// body.
// --------------------------------------------------------------------

/// SPEC-3.0 §2.1 / §4.2 thread metadata.
///
/// Required fields per the SPEC-3.0 §2.1 table; `branch` and
/// `supersedes` are optional convenience fields per the same section.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub category: String,
    pub status: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
}

impl ThreadSnapshot {
    /// Schema version this implementation reads/writes.
    pub const SCHEMA_VERSION: u32 = 3;

    pub fn to_toml(&self) -> Result<String, ForumError> {
        toml::to_string(self)
            .map_err(|e| ForumError::SnapshotInvalid(format!("serialize thread.toml: {e}")))
    }

    pub fn from_toml(s: &str) -> Result<Self, ForumError> {
        // Pre-flight: probe `schema_version` through an intermediate
        // struct with `Option<u32>` so an *absent* field maps to
        // SnapshotSchemaUnsupported (per SPEC-3.0 §11) rather than a
        // generic TOML missing-field error. Codex objection
        // `2890e3edd4983bd3` on qa8u71j9.
        #[derive(serde::Deserialize)]
        struct SchemaVersionProbe {
            schema_version: Option<u32>,
        }
        let probe: SchemaVersionProbe = toml::from_str(s)?;
        let v = probe.schema_version.ok_or_else(|| {
            ForumError::SnapshotSchemaUnsupported(
                "thread.toml is missing required `schema_version` field".into(),
            )
        })?;
        if v != Self::SCHEMA_VERSION {
            return Err(ForumError::SnapshotSchemaUnsupported(format!(
                "thread.toml schema_version={v} (this build supports {})",
                Self::SCHEMA_VERSION
            )));
        }
        Ok(toml::from_str(s)?)
    }
}

#[cfg(test)]
mod thread_snapshot_tests {
    use super::*;

    fn sample_snapshot() -> ThreadSnapshot {
        ThreadSnapshot {
            schema_version: 3,
            id: "fg61bcmp".into(),
            title: "3.0: Snapshot storage".into(),
            category: "rfc".into(),
            status: "draft".into(),
            tags: vec!["cross-cutting".into()],
            created_at: "2026-05-02T23:31:40Z".parse().unwrap(),
            created_by: "ai/codex".into(),
            updated_at: "2026-05-02T23:31:40Z".parse().unwrap(),
            updated_by: "ai/codex".into(),
            branch: None,
            supersedes: Vec::new(),
        }
    }

    #[test]
    fn round_trip_minimal() {
        let original = sample_snapshot();
        let s = original.to_toml().unwrap();
        let parsed = ThreadSnapshot::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trip_with_optionals() {
        let original = ThreadSnapshot {
            branch: Some("feat/snapshot".into()),
            supersedes: vec!["thread-old1".into(), "thread-old2".into()],
            ..sample_snapshot()
        };
        let s = original.to_toml().unwrap();
        let parsed = ThreadSnapshot::from_toml(&s).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn schema_version_mismatch_rejected() {
        let mut snap = sample_snapshot();
        snap.schema_version = 2;
        let s = toml::to_string(&snap).unwrap();
        let err = ThreadSnapshot::from_toml(&s).unwrap_err();
        assert!(
            matches!(err, ForumError::SnapshotSchemaUnsupported(_)),
            "expected SnapshotSchemaUnsupported, got {err}"
        );
    }

    #[test]
    fn missing_schema_version_rejected_as_snapshot_schema_unsupported() {
        // Per SPEC-3.0 §11 SnapshotSchemaUnsupported triggers on
        // either an *absent* or *unsupported* `schema_version`.
        let bad = r#"
            id = "fg61bcmp"
            title = "T"
            category = "rfc"
            status = "draft"
            tags = []
            created_at = "2026-05-02T23:31:40Z"
            created_by = "ai/codex"
            updated_at = "2026-05-02T23:31:40Z"
            updated_by = "ai/codex"
        "#;
        let err = ThreadSnapshot::from_toml(bad).unwrap_err();
        assert!(
            matches!(err, ForumError::SnapshotSchemaUnsupported(_)),
            "expected SnapshotSchemaUnsupported for absent schema_version, got {err}"
        );
    }

    #[test]
    fn omits_unset_optionals() {
        let s = sample_snapshot().to_toml().unwrap();
        assert!(!s.contains("branch"), "unset branch should be omitted: {s}");
        assert!(
            !s.contains("supersedes"),
            "empty supersedes should be omitted: {s}"
        );
    }
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
            title: Some(title.into()),
            kind: Some(kind),
            ..Event::default()
        }
    }

    fn make_state(thread_id: &str, new_state: &str) -> Event {
        Event {
            event_id: "evt-0002".into(),
            thread_id: thread_id.into(),
            event_type: EventType::State,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            actor: "human/alice".into(),
            new_state: Some(new_state.into()),
            ..Event::default()
        }
    }

    #[test]
    fn replay_single_create() {
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "Test RFC")];
        let state = replay(&events).unwrap();
        assert_eq!(state.id, "RFC-0001");
        assert_eq!(state.kind, ThreadKind::Rfc);
        assert_eq!(state.title, "Test RFC");
        assert_eq!(state.body, None);
        assert_eq!(state.status, "draft");
        assert_eq!(state.created_by, "human/alice");
        assert_eq!(state.events.len(), 1);
    }

    #[test]
    fn replay_create_then_state() {
        // Phase 2a: 1.x "proposed" is normalized by parse_lenient into the
        // canonical 2.0 status `Open`. The original assertion against the
        // raw string is replaced by the parsed enum variant.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "Test RFC"),
            make_state("RFC-0001", "proposed"),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Open);
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

    // --- resolve_from_list tests ---

    fn ids(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolve_exact_match() {
        let all = ids(&["RFC-0001", "ASK-a7f3b2x1"]);
        assert_eq!(resolve_from_list(&all, "RFC-0001").unwrap(), "RFC-0001");
    }

    #[test]
    fn resolve_prefix_match() {
        let all = ids(&["RFC-a7f3b2x1", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "RFC-a7f3").unwrap(), "RFC-a7f3b2x1");
    }

    #[test]
    fn resolve_token_only_match() {
        let all = ids(&["RFC-a7f3b2x1", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "a7f3b2x1").unwrap(), "RFC-a7f3b2x1");
    }

    #[test]
    fn resolve_case_insensitive_exact() {
        let all = ids(&["RFC-0030", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "rfc-0030").unwrap(), "RFC-0030");
    }

    #[test]
    fn resolve_case_insensitive_prefix() {
        let all = ids(&["RFC-a7f3b2x1", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "rfc-a7f3").unwrap(), "RFC-a7f3b2x1");
    }

    #[test]
    fn resolve_case_insensitive_token() {
        let all = ids(&["RFC-a7f3b2x1", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "A7F3B2X1").unwrap(), "RFC-a7f3b2x1");
    }

    #[test]
    fn resolve_not_found_includes_hint() {
        let all = ids(&["RFC-0001"]);
        let err = resolve_from_list(&all, "nonexistent").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not found"), "got: {msg}");
        assert!(msg.contains("hint"), "should include hint; got: {msg}");
    }

    #[test]
    fn resolve_ambiguous_shows_candidates() {
        let all = ids(&["RFC-a7f30001", "RFC-a7f30002"]);
        let err = resolve_from_list(&all, "RFC-a7f3").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ambiguous"), "got: {msg}");
        assert!(msg.contains("RFC-a7f30001"), "got: {msg}");
        assert!(msg.contains("RFC-a7f30002"), "got: {msg}");
    }

    #[test]
    fn resolve_strips_at_marker_before_matching() {
        // SPEC-2.0 §6.1.1: `@` is accepted but optional at CLI input.
        let all = ids(&["RFC-a7f3b2x1", "a8b9c0d1"]);
        assert_eq!(
            resolve_from_list(&all, "@a7f3b2x1").unwrap(),
            "RFC-a7f3b2x1"
        );
        assert_eq!(resolve_from_list(&all, "@a8b9c0d1").unwrap(), "a8b9c0d1");
        assert_eq!(
            resolve_from_list(&all, "@RFC-a7f3").unwrap(),
            "RFC-a7f3b2x1"
        );
    }

    #[test]
    fn resolve_bare_token_exact_match() {
        // SPEC-2.0 §6.2: a 2.0 thread ref is its bare token.
        let all = ids(&["a7f3b2x1", "RFC-0001"]);
        assert_eq!(resolve_from_list(&all, "a7f3b2x1").unwrap(), "a7f3b2x1");
        assert_eq!(resolve_from_list(&all, "@a7f3b2x1").unwrap(), "a7f3b2x1");
    }

    #[test]
    fn resolve_bare_token_prefix_match() {
        // SPEC-2.0 §6.2: unambiguous prefixes (≥4 chars) accepted on bare-token storage too.
        let all = ids(&["a7f3b2x1", "ASK-0001"]);
        assert_eq!(resolve_from_list(&all, "a7f3").unwrap(), "a7f3b2x1");
        assert_eq!(resolve_from_list(&all, "@a7f3").unwrap(), "a7f3b2x1");
        // Case-insensitive bare-token prefix.
        assert_eq!(resolve_from_list(&all, "A7F3").unwrap(), "a7f3b2x1");
    }

    #[test]
    fn resolve_case_insensitive_ambiguous_shows_did_you_mean() {
        // Use a case where only the case-insensitive path triggers
        let all2 = ids(&["RFC-abcd1234", "RFC-ABCD1234"]);
        // This won't actually happen since thread IDs are always uppercase prefix + lowercase token,
        // but test the logic anyway
        let err = resolve_from_list(&all2, "rfc-abcd1234").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("did you mean"),
            "should show 'did you mean'; got: {msg}"
        );
    }

    // ---- facet_set replay (SPEC-2.0 §2.4.1) ----

    fn make_facet_set(
        thread_id: &str,
        seq: u32,
        lifecycle: Option<&str>,
        tags_add: &[&str],
        tags_remove: &[&str],
    ) -> Event {
        Event {
            event_id: format!("evt-facet-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::FacetSet,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            lifecycle: lifecycle.map(str::to_string),
            tags_add: tags_add.iter().map(|s| s.to_string()).collect(),
            tags_remove: tags_remove.iter().map(|s| s.to_string()).collect(),
            ..Event::default()
        }
    }

    #[test]
    fn facet_set_first_lifecycle_wins() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            // Second facet_set carrying lifecycle: silently ignored at replay
            // (write-side rejection is Track B).
            make_facet_set("RFC-0001", 2, Some("execution"), &[], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }

    #[test]
    fn facet_set_lifecycle_optional() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            // facet_set with no lifecycle, no tags — valid no-op (§2.4.1).
            make_facet_set("RFC-0001", 1, None, &[], &[]),
        ];
        let state = replay(&events).unwrap();
        // Phase 2c: facet_set without `lifecycle` doesn't flip `lifecycle_explicit`.
        // The lifecycle stays at its kind-derived default (`Rfc -> Proposal`).
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(!state.lifecycle_explicit);
        assert!(state.tags.is_empty());
    }

    #[test]
    fn facet_set_tags_add_then_remove_within_event() {
        // Within one event tags_add applies before tags_remove.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["bug", "ux"], &["bug"]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["ux".to_string()]);
    }

    #[test]
    fn facet_set_tags_accumulate_across_events() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["a", "b"], &[]),
            make_facet_set("RFC-0001", 2, None, &["c"], &["a"]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn facet_set_tags_add_dedupes() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["bug"], &[]),
            // Re-adding the same tag is a no-op.
            make_facet_set("RFC-0001", 2, None, &["bug"], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["bug".to_string()]);
    }

    #[test]
    fn lifecycle_accessor_falls_back_to_kind() {
        // No facet_set event in chain — derive from ThreadKind per §2.3.3.
        let state = replay(&[make_create("RFC-0001", ThreadKind::Rfc, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(
            !state.lifecycle_explicit,
            "kind-derived lifecycle is implicit"
        );

        let state = replay(&[make_create("ASK-0001", ThreadKind::Issue, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Execution);
        assert!(!state.lifecycle_explicit);

        let state = replay(&[make_create("DEC-0001", ThreadKind::Dec, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Record);
        assert!(!state.lifecycle_explicit);
    }

    #[test]
    fn lifecycle_accessor_prefers_explicit_facet_set() {
        // SPEC-2.0 §2.3.3 / §7.3: an explicit facet_set lifecycle drives
        // the state machine even on a thread whose ThreadKind would map
        // elsewhere. (Migration overlay scenario.)
        let events = vec![
            make_create("ASK-0001", ThreadKind::Issue, "T"),
            make_facet_set("ASK-0001", 1, Some("record"), &[], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Record);
        assert!(
            state.lifecycle_explicit,
            "explicit facet_set must flip the flag"
        );
    }

    #[test]
    fn facet_set_tags_remove_unknown_is_noop() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &[], &["nonexistent"]),
        ];
        let state = replay(&events).unwrap();
        assert!(state.tags.is_empty());
    }

    // ---- replay_strict (Phase 1: Finding 4) ----

    use super::super::validate::StrictReplayIssue;

    fn make_resolve(thread_id: &str, target: &str, seq: u32) -> Event {
        Event {
            event_id: format!("evt-resolve-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::Resolve,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some(target.into()),
            ..Event::default()
        }
    }

    fn make_edit(thread_id: &str, target: &str, body: Option<&str>, seq: u32) -> Event {
        Event {
            event_id: format!("evt-edit-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::Edit,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some(target.into()),
            body: body.map(str::to_string),
            ..Event::default()
        }
    }

    #[test]
    fn replay_strict_clean_thread_yields_no_issues() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &["bug"], &[]),
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }

    #[test]
    fn replay_strict_flags_resolve_on_unknown_node() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_resolve("RFC-0001", "ghost-node", 1),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::UnknownTargetNode { target_node_id, .. }] if target_node_id == "ghost-node"
        ));
    }

    #[test]
    fn replay_strict_flags_edit_missing_body() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_edit("RFC-0001", "any-node", None, 1),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        // We get both UnknownTargetNode is skipped — when body is missing we
        // never look up the node. Only MissingRequiredField is reported.
        assert_eq!(issues.len(), 1, "got: {issues:?}");
        assert!(matches!(
            &issues[0],
            StrictReplayIssue::MissingRequiredField { field, .. } if *field == "body"
        ));
    }

    #[test]
    fn replay_strict_flags_lifecycle_reset() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 2, Some("execution"), &[], &[]),
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        // Lenient first-wins still holds.
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::LifecycleResetAttempted { existing, attempted, .. }]
                if existing == "proposal" && attempted == "execution"
        ));
    }

    #[test]
    fn replay_strict_idempotent_lifecycle_reset_is_clean() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 2, Some("proposal"), &[], &[]),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.is_empty(),
            "idempotent re-set should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_flags_invalid_lifecycle_value() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("nonsense"), &[], &[]),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.iter().any(|i| matches!(
                i,
                StrictReplayIssue::InvalidLifecycleValue { value, .. } if value == "nonsense"
            )),
            "got: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_flags_state_event_missing_new_state() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            Event {
                event_id: "evt-state-bad".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::State,
                created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                actor: "human/alice".into(),
                ..Event::default()
            },
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::MissingRequiredField { field, .. }] if *field == "new_state"
        ));
    }

    #[test]
    fn replay_strict_flags_illegal_transition_for_lifecycle() {
        // SPEC-2.0 §3.1 / P0 #34ith16h: strict replay must surface a
        // state event whose `from -> to` edge is missing from the
        // per-lifecycle transition graph.
        //
        // Setup: an RFC (proposal lifecycle) starts at `draft`, goes to
        // `done`, then attempts `done -> review`. The `done -> review`
        // edge does not exist for any lifecycle; lenient replay applies
        // it anyway, strict surfaces the legality miss.
        let mut state_event = make_state("RFC-0001", "review");
        state_event.event_id = "evt-illegal".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "open"),
            make_state("RFC-0001", "done"),
            state_event,
        ];
        let (final_state, issues) = replay_strict(&events).unwrap();
        // Lenient semantic preserved: the new status was applied.
        assert_eq!(final_state.status, ThreadStatus::Review);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                StrictReplayIssue::InvalidTransition {
                    event_id, from, to, lifecycle
                } if event_id == "evt-illegal"
                    && from == "done"
                    && to == "review"
                    && lifecycle == "proposal"
            )),
            "expected an InvalidTransition issue, got: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_clean_for_legal_transition() {
        // Regression guard: a legal transition (open -> review on
        // proposal) must NOT emit InvalidTransition.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "open"),
            make_state("RFC-0001", "review"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "legal transition should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_legacy_state_synonyms_remain_legal() {
        // Lenient compatibility: a 1.x state name that normalizes onto a
        // canonical 2.0 transition must NOT trip InvalidTransition.
        // Here `proposed` (=> open) follows `draft` on a proposal — a
        // legal edge.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "proposed"),
            make_state("RFC-0001", "under-review"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.is_empty(),
            "1.x synonyms on a legal path should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_idempotent_state_does_not_flag() {
        // No-op transitions (status unchanged) skip the legality check —
        // a state event that re-asserts the current status is benign.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "draft"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(issues.is_empty(), "idempotent re-state: {issues:?}");
    }

    #[test]
    fn replay_strict_self_heal_suppresses_invalid_transition_after_corrective_tail() {
        // SPEC-2.0 §3.1 / #uu9wxn1d: a thread that hit `draft → rejected`
        // (illegal on proposal) and was repaired by appending
        // `state open` then `state rejected` should not surface the
        // InvalidTransition issue — the corrective tail walks back to
        // the visible terminal status (`rejected`) via legal edges.
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let mut fix1 = make_state("RFC-0001", "open");
        fix1.event_id = "evt-fix1".into();
        let mut fix2 = make_state("RFC-0001", "rejected");
        fix2.event_id = "evt-fix2".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            bad,
            fix1,
            fix2,
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Rejected);
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "self-healed chain must not surface InvalidTransition: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_no_self_heal_without_corrective_tail() {
        // Regression guard: a chain that ends at the offending event with
        // no corrective tail must STILL surface the InvalidTransition.
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "T"), bad];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "no corrective tail → issue must remain: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_no_self_heal_when_terminal_status_differs() {
        // If the chain's terminal status differs from the issue's `to`,
        // no self-heal — the operator's visible state isn't what the
        // illegal event aimed at.
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let mut fix1 = make_state("RFC-0001", "open");
        fix1.event_id = "evt-fix1".into();
        // Terminal = open, not rejected.
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "T"), bad, fix1];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Open);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "terminal mismatch → issue must remain: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_review_to_withdrawn_cannot_self_heal() {
        // `withdrawn` is a sink in proposal lifecycle (no outgoing legal
        // edges). A `review → withdrawn` violation cannot be self-healed
        // via append-only — the corrective walk would need an outgoing
        // edge from withdrawn that doesn't exist. Issue must remain.
        let mut intake = make_state("RFC-0001", "open");
        intake.event_id = "evt-intake".into();
        let mut review = make_state("RFC-0001", "review");
        review.event_id = "evt-review".into();
        let mut bad = make_state("RFC-0001", "withdrawn");
        bad.event_id = "evt-bad".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            intake,
            review,
            bad,
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Withdrawn);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "Category B (review→withdrawn) cannot self-heal: {issues:?}"
        );
    }

    #[test]
    fn replay_lenient_unchanged_under_strict_failures() {
        // Regression guard: read-side `replay()` must not start failing for
        // any of the conditions strict mode now flags.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_resolve("RFC-0001", "ghost-node", 1),
            make_facet_set("RFC-0001", 2, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 3, Some("execution"), &[], &[]),
        ];
        let state = replay(&events).expect("lenient replay must still succeed");
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }
}
