use chrono::{DateTime, Utc};

use super::error::{ForumError, ForumResult};
use super::event::{self, Event, EventType, Lifecycle, NodeType, ThreadKind, ThreadStatus};
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::node::Node;
use super::refs;
use super::validate::StrictReplayIssue;

pub const MIN_NODE_ID_PREFIX_LEN: usize = 4;

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
    /// value is derived from `kind.lifecycle()` (the §2.3.3 legacy mapping)
    /// at replay start; the first `facet_set` event carrying `lifecycle`
    /// overrides it and sets [`lifecycle_explicit`](Self::lifecycle_explicit).
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
    pub thread_kind: ThreadKind,
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

fn replay_with_issues(events: &[Event]) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
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

    // `kind.initial_status()` returns a hardcoded canonical literal
    // (`"draft"` / `"open"`); parse_lenient is total over this input.
    let initial_status = ThreadStatus::parse_lenient(kind.initial_status())
        .expect("kind.initial_status() always returns a canonical 2.0 status name");
    let mut state = ThreadState {
        id: first.thread_id.clone(),
        kind,
        title,
        body: first.body.clone(),
        branch: first.branch.clone(),
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
        // kind-derived value; the first explicit `facet_set` overrides it
        // and flips `lifecycle_explicit` below.
        lifecycle: kind.lifecycle(),
        lifecycle_explicit: false,
        tags: Vec::new(),
    };

    let mut issues = Vec::new();
    for ev in &events[1..] {
        apply_event(&mut state, ev, &mut issues)?;
    }
    Ok((state, issues))
}

fn apply_event(
    state: &mut ThreadState,
    event: &Event,
    issues: &mut Vec<StrictReplayIssue>,
) -> ForumResult<()> {
    state.events.push(event.clone());
    match event.event_type {
        EventType::State => {
            match event.new_state.as_deref() {
                Some(name) => match ThreadStatus::parse_lenient(name) {
                    Some(parsed) => state.status = parsed,
                    // Lenient: keep the prior status. Strict mode surfaces
                    // the unparseable value below.
                    None => issues.push(StrictReplayIssue::InvalidStateValue {
                        event_id: event.event_id.clone(),
                        value: name.to_string(),
                    }),
                },
                None => issues.push(StrictReplayIssue::MissingRequiredField {
                    event_id: event.event_id.clone(),
                    event_type: event.event_type,
                    field: "new_state",
                }),
            }
            // SPEC-2.0 §2.8: 1.x State events carried approvals as a direct
            // field; 2.0 emits them as Approval-typed Say nodes. Synthesize
            // equivalent nodes here so policy guards see one source of truth.
            for approval in &event.approvals {
                state.nodes.push(Node {
                    node_id: format!("{}#{}", event.event_id, approval.actor_id),
                    node_type: NodeType::Approval,
                    body: String::new(),
                    actor: approval.actor_id.clone(),
                    created_at: approval.approved_at,
                    ..Node::default()
                });
            }
        }
        EventType::Scope => {
            // `branch = None` legitimately clears scope, so absence is not an
            // issue here; lenient and strict agree.
            state.branch.clone_from(&event.branch);
        }
        EventType::Say => match (event.node_type, &event.body) {
            (Some(node_type), Some(body)) => {
                state.nodes.push(Node {
                    node_id: say_node_id(event).to_string(),
                    node_type,
                    body: body.clone(),
                    actor: event.actor.clone(),
                    created_at: event.created_at,
                    resolved: false,
                    retracted: false,
                    incorporated: false,
                    reply_to: event.reply_to.clone(),
                    legacy_subtype: event.legacy_subtype.clone(),
                });
            }
            (None, _) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "node_type",
            }),
            (_, None) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "body",
            }),
        },
        EventType::Edit => match (&event.target_node_id, &event.body) {
            (Some(node_id), Some(body)) => {
                if let Some(node) = state.nodes.iter_mut().find(|n| &n.node_id == node_id) {
                    node.body = body.clone();
                } else {
                    issues.push(StrictReplayIssue::UnknownTargetNode {
                        event_id: event.event_id.clone(),
                        event_type: event.event_type,
                        target_node_id: node_id.clone(),
                    });
                }
            }
            (None, _) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "target_node_id",
            }),
            (_, None) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "body",
            }),
        },
        EventType::Retract => apply_node_flag(state, event, issues, |n| n.retracted = true),
        EventType::Resolve => apply_node_flag(state, event, issues, |n| n.resolved = true),
        EventType::Reopen => apply_node_flag(state, event, issues, |n| {
            n.resolved = false;
            n.retracted = false;
            n.incorporated = false;
        }),
        EventType::Retype => match (&event.target_node_id, event.node_type) {
            (Some(node_id), Some(new_type)) => {
                if let Some(node) = state.nodes.iter_mut().find(|n| &n.node_id == node_id) {
                    node.node_type = new_type;
                } else {
                    issues.push(StrictReplayIssue::UnknownTargetNode {
                        event_id: event.event_id.clone(),
                        event_type: event.event_type,
                        target_node_id: node_id.clone(),
                    });
                }
            }
            (None, _) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "target_node_id",
            }),
            (_, None) => issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                field: "node_type",
            }),
        },
        EventType::ReviseBody => {
            if let Some(ref body) = event.body {
                state.body = Some(body.clone());
                state.body_revision_count += 1;
            } else {
                issues.push(StrictReplayIssue::MissingRequiredField {
                    event_id: event.event_id.clone(),
                    event_type: event.event_type,
                    field: "body",
                });
            }
            for node_id in &event.incorporated_node_ids {
                let found = state
                    .nodes
                    .iter_mut()
                    .find(|n| n.node_id == *node_id)
                    .map(|node| node.incorporated = true);
                if found.is_none() {
                    issues.push(StrictReplayIssue::UnknownTargetNode {
                        event_id: event.event_id.clone(),
                        event_type: event.event_type,
                        target_node_id: node_id.clone(),
                    });
                }
                if !state.incorporated_node_ids.contains(node_id) {
                    state.incorporated_node_ids.push(node_id.clone());
                }
            }
        }
        EventType::Link => {
            if let Some(ev_data) = &event.evidence {
                let mut ev = ev_data.clone();
                ev.evidence_id = event.event_id.clone();
                state.evidence_items.push(ev);
            } else if let (Some(target), Some(rel)) = (&event.target_node_id, &event.link_rel) {
                state.links.push(ThreadLink {
                    target_thread_id: target.clone(),
                    rel: rel.clone(),
                });
            } else {
                // Neither evidence nor (target+rel) present — Link with no
                // payload is meaningless.
                issues.push(StrictReplayIssue::MissingRequiredField {
                    event_id: event.event_id.clone(),
                    event_type: event.event_type,
                    field: "evidence_or_target_link",
                });
            }
        }
        // These event types are no-ops during replay:
        EventType::Create => {} // handled in replay() before apply_event loop
        EventType::Verify | EventType::Merge => {}
        // SPEC-2.0 §2.4.1: per-event facet mutation, not a full-state
        // replacement.
        EventType::FacetSet => {
            // First-lifecycle-wins: §7.3 makes lifecycle immutable, so any
            // subsequent facet_set carrying `lifecycle` is silently ignored
            // at replay (write-side rejection with FacetTransitionDisallowed
            // is Track B's responsibility).
            if let Some(ref lc) = event.lifecycle {
                let parsed = Lifecycle::parse(lc);
                if parsed.is_none() {
                    issues.push(StrictReplayIssue::InvalidLifecycleValue {
                        event_id: event.event_id.clone(),
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
                            event_id: event.event_id.clone(),
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
            for tag in &event.tags_add {
                if !state.tags.iter().any(|t| t == tag) {
                    state.tags.push(tag.clone());
                }
            }
            for tag in &event.tags_remove {
                state.tags.retain(|t| t != tag);
            }
        }
    }
    Ok(())
}

/// Shared helper for `Retract` / `Resolve` / `Reopen`: locate the target node
/// by id, apply `mutate`, or record an [`StrictReplayIssue::UnknownTargetNode`].
fn apply_node_flag(
    state: &mut ThreadState,
    event: &Event,
    issues: &mut Vec<StrictReplayIssue>,
    mutate: impl FnOnce(&mut Node),
) {
    match &event.target_node_id {
        Some(node_id) => {
            if let Some(node) = state.nodes.iter_mut().find(|n| &n.node_id == node_id) {
                mutate(node);
            } else {
                issues.push(StrictReplayIssue::UnknownTargetNode {
                    event_id: event.event_id.clone(),
                    event_type: event.event_type,
                    target_node_id: node_id.clone(),
                });
            }
        }
        None => issues.push(StrictReplayIssue::MissingRequiredField {
            event_id: event.event_id.clone(),
            event_type: event.event_type,
            field: "target_node_id",
        }),
    }
}

/// Load events from Git and replay to get thread state (lenient).
pub fn replay_thread(git: &GitOps, thread_id: &str) -> ForumResult<ThreadState> {
    let events = event::load_thread_events(git, thread_id)?;
    replay(&events)
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
        .resolve_ref(&super::migrate::alias_ref(legacy_id))?
        .is_none()
    {
        return Ok(None);
    }
    let token = super::migrate::bare_token_for(legacy_id);
    if git.resolve_ref(&refs::thread_ref(&token))?.is_some() {
        Ok(Some(token))
    } else {
        Ok(None)
    }
}

fn resolve_alias_case_insensitive(git: &GitOps, user_input: &str) -> ForumResult<Option<String>> {
    let aliases = git.list_refs(super::migrate::ALIASES_PREFIX)?;
    let target = user_input.to_ascii_uppercase();
    let mut hits: Vec<String> = aliases
        .iter()
        .filter_map(|r| r.strip_prefix(super::migrate::ALIASES_PREFIX))
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
