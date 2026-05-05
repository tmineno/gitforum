use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{ForumError, ForumResult};
use super::evidence::Evidence;
use super::git_ops::GitOps;
use super::node::{Node, NodeKind};
use super::policy::Lifecycle;
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

    /// SPEC-2.0 §2.3.3: each 1.x kind maps to a canonical lifecycle
    /// facet. Used to derive `lifecycle` for legacy threads with no
    /// `facet_set` event in their chain. Mirrors the kind-preset row
    /// for each variant in `legacy::workflow::KIND_PRESETS`; v3.1
    /// step 3j inlined the mapping so this method does not reach
    /// into `internal::legacy::*`.
    pub fn lifecycle(self) -> Lifecycle {
        match self {
            Self::Rfc => Lifecycle::Proposal,
            Self::Issue | Self::Task => Lifecycle::Execution,
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

/// SPEC-2.0 §2.3.5 / SPEC-3.0 §2.3.5 tag grammar:
/// - ASCII lowercase only, `[a-z0-9-]`
/// - Starts with a letter `[a-z]`
/// - Length 2..=32
/// - Not a reserved literal (`all`, `none`, `any`, `untagged`)
///
/// Phase 4 Step 2a (RFC `7ymtc4b2`, task `913c4s9v`) relocated this
/// from `event.rs` so KEEP files (e.g. `tui/state.rs`) can validate
/// user-input tags without importing the v2 event module.
/// `internal::event::validate_tag` remains as a `pub use` re-export
/// for legacy callers retired in Steps 2/3.
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
    /// Most recent change timestamp. Snapshot-derived states use the
    /// snapshot's `updated_at`; legacy event-chain replays carry the
    /// most recent event's `created_at` (see `legacy::chain_replay`).
    /// v3.1 step 3j replaced the v2 `events: Vec<Event>` field — the
    /// only consumer of `events.last()` was `ls.rs`'s "updated"
    /// column, which now reads this field directly.
    pub updated_at: DateTime<Utc>,
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
}

impl ThreadState {
    /// Open (unresolved, not retracted) objection nodes.
    pub fn open_objections(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == NodeKind::Objection && n.is_open())
            .collect()
    }

    /// Open (unresolved, not retracted) action nodes.
    pub fn open_actions(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == NodeKind::Action && n.is_open())
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
        self.nodes
            .iter()
            .rfind(|n| !n.retracted && n.legacy_subtype.as_deref() == Some("summary"))
    }
}

/// Read the SPEC-3.0 snapshot at the thread ref tip and materialize
/// it as [`ThreadState`].
///
/// v3.1 step 3j (task `1v400j3l`) made this snapshot-only: the
/// mixed-chain / pure-event-chain replay path moved to
/// [`super::legacy::chain_replay::replay_chain_at`] and is reachable
/// only from `commands::migrate`. Threads whose tip still carries an
/// `event.json` (i.e. an unmigrated 1.x/2.x chain) surface
/// [`ForumError::LegacyEventChain`] — its message tells the user to
/// run `git forum migrate` first.
///
/// Errors:
/// - [`ForumError::Repo`] — thread ref does not exist.
/// - [`ForumError::SnapshotMissing`] — tip tree lacks `thread.toml`.
/// - [`ForumError::LegacyEventChain`] — tip is an unmigrated event commit.
/// - [`ForumError::SnapshotSchemaUnsupported`] / [`ForumError::SnapshotInvalid`] /
///   [`ForumError::Toml`] — bad snapshot payload.
pub fn replay_thread(git: &GitOps, thread_id: &str) -> ForumResult<ThreadState> {
    let doc = super::snapshot::read_snapshot(git, thread_id)?;
    Ok(materialize_thread_state_from_snapshot(doc))
}

/// Strict variant of [`replay_thread`].
///
/// Snapshots are validated at write time, so the issue vector is
/// always empty for snapshot-only reads. The signature is preserved
/// for callers (doctor, migration verification) that historically
/// fanned out across both the snapshot and event-chain paths.
pub fn replay_thread_strict(
    git: &GitOps,
    thread_id: &str,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    Ok((replay_thread(git, thread_id)?, Vec::new()))
}

/// Materialize a [`ThreadState`] from a SPEC-3.0
/// [`ThreadDocument`](super::snapshot::ThreadDocument).
///
/// `pub` so the legacy event-chain reader
/// (`legacy::chain_replay::replay_chain_at`) can seed state from the
/// snapshot at chain bottom on a mixed chain.
pub fn materialize_thread_state_from_snapshot(doc: super::snapshot::ThreadDocument) -> ThreadState {
    use super::evidence::Evidence;
    use super::node::NodeStatus;
    use super::snapshot::ThreadDocument;

    let ThreadDocument {
        snapshot,
        body,
        nodes,
        links,
        evidence,
    } = doc;

    let kind = category_to_legacy_kind(&snapshot.category, &snapshot.tags);
    let lifecycle =
        super::policy::legacy_lifecycle_for_category(&snapshot.category, &snapshot.tags);
    let status = ThreadStatus::parse_lenient(&snapshot.status).unwrap_or_default();

    let nodes: Vec<Node> = nodes
        .into_iter()
        .map(|n| Node {
            node_id: n.record.id,
            node_type: n.record.kind,
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
        updated_at: snapshot.updated_at,
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
    NodeLookup {
        thread_id: state.id.clone(),
        thread_title: state.title.clone(),
        thread_kind: state.kind,
        thread_lifecycle: state.lifecycle,
        thread_tags: state.tags.clone(),
        node: node.clone(),
        links: state.links.clone(),
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
}
