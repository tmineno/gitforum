//! `git forum migrate` — one-shot v→3.0 migration (SPEC-3.0 §8).
//!
//! Walks `refs/forum/threads/*` and rewrites each event-chain ref to
//! a SPEC-3.0 §4.2 snapshot tree. Source events survive in two
//! places:
//!
//! - As ancestor commits of the new snapshot (the legacy chain
//!   becomes the parent of the snapshot commit; nothing is rewritten
//!   in place).
//! - As `legacy/events.ndjson` inside the snapshot tree, written
//!   verbatim from the source events in chain order (SPEC-3.0 §8.2).
//!   `read_snapshot` ignores this file semantically; subsequent
//!   3.0-native writes preserve it byte-identical via
//!   [`crate::internal::snapshot::store::write_snapshot`]'s
//!   parent-tree preservation rule.
//!
//! ADR-011 Decision 3: this command is the only sanctioned consumer
//! of `internal::legacy::*` and the only caller of
//! [`migrate_legacy_to_snapshot`]. The build-time gate in
//! `tests/legacy_gate_test.rs` enforces the import boundary.
//!
//! v→2 alias resolution survives as a read-side helper:
//! [`ALIASES_PREFIX`] and [`alias_ref`] are consumed by
//! `internal::thread::resolve_thread_id` (and `commands::hook`) so
//! prefixed legacy IDs (`RFC-0001`, `ASK-q3kfj49v`) keep resolving
//! after migration as long as the alias entries from a prior v→2
//! run still exist. v→3 itself does NOT create new aliases — it
//! writes the snapshot at the same ref name and is a no-op for the
//! ref topology.

use sha2::{Digest, Sha256};

use super::super::config::RepoPaths;
use super::super::error::{ForumError, ForumResult};
use super::super::event::{self, NodeType, ThreadKind};
use super::super::evidence::{EvidenceFile, EvidenceRecord};
use super::super::git_ops::GitOps;
use super::super::id_alloc;
use super::super::node::{NodeKind, NodeRecord, NodeStatus};
use super::super::refs;
use super::super::snapshot::{self, Link, Links, NodeWithBody, ThreadDocument};
use super::super::thread::{self, ThreadSnapshot, ThreadState};
use super::super::validate::StrictReplayIssue;

/// SPEC-2.0 §10: v→2 alias entries live under
/// `refs/forum/aliases/<old-id>` and point at the same commit as the
/// canonical thread ref. Consumed by
/// [`crate::internal::thread::resolve_thread_id`] so legacy IDs
/// continue to resolve. v3 does not create new aliases (the snapshot
/// is written at the source ref name) but preserves any alias
/// entries already on disk from a prior v→2 run.
pub const ALIASES_PREFIX: &str = "refs/forum/aliases/";

/// Construct the alias ref name for a legacy thread ID.
pub fn alias_ref(legacy_id: &str) -> String {
    format!("{ALIASES_PREFIX}{legacy_id}")
}

/// Compute the deterministic bare-token form for a legacy thread ID.
///
/// Survives Phase 3 because v→2 alias resolution still needs it:
/// `internal::thread::resolve_thread_id` and a few doctor/index call
/// sites canonicalize a user-supplied prefixed ID (`RFC-0001`,
/// `ASK-q3kfj49v`) to the bare token a prior v→2 run would have
/// minted, then look up that token's snapshot ref. v→3 itself does
/// NOT mint or rename — `migrate_one` writes at the source ref
/// name — but if a repo has been through v→2 already, the bare
/// token IS the source ref name and this helper resolves to it.
///
/// Cases:
/// - Already bare → return unchanged.
/// - Opaque (`KIND-<8 base36>`) → strip the prefix.
/// - Sequential (`KIND-NNNN`) → hash the legacy ID and project to 8
///   base36 chars; force the leading char to a letter (the bare-
///   token grammar forbids all-digit tokens).
pub fn bare_token_for(legacy_id: &str) -> String {
    if id_alloc::is_bare_token(legacy_id) {
        return legacy_id.to_string();
    }
    if let Some((_, token)) = legacy_id.split_once('-') {
        if token.len() == 8
            && token
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && !token.chars().all(|c| c.is_ascii_digit())
        {
            return token.to_string();
        }
    }
    derive_token_from_seed(legacy_id)
}

/// Hash `seed` to a deterministic 8-char base36 token whose leading
/// character is forced to a letter so the result satisfies the
/// bare-token grammar (`is_bare_token` returns true).
///
/// The algorithm is byte-exact identical to the v2 migrator. Repos
/// that already ran v→2 migration recorded alias entries minted by
/// this exact function (salt + slot for the forced letter prefix);
/// changing it would silently break alias resolution for sequential
/// legacy IDs (e.g. `RFC-0001`). Do NOT alter the salt, slot indices,
/// or alphabet without a coordinated change to every existing alias
/// ref in the wild.
fn derive_token_from_seed(seed: &str) -> String {
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut hasher = Sha256::new();
    hasher.update(b"git-forum/migrate/v2.0\n");
    hasher.update(seed.as_bytes());
    let hash = hasher.finalize();
    let mut chars = Vec::with_capacity(8);
    for &b in hash.iter().take(8) {
        chars.push(ALPHABET[(b as usize) % 36]);
    }
    // Force first character to a letter so the token is never
    // all-digit (id_alloc::is_bare_token rejects all-digit tokens).
    // The replacement letter is selected from `hash[8]` — NOT
    // `hash[0]` — to match the v2 algorithm byte-for-byte.
    if !(chars[0] as char).is_ascii_lowercase() {
        let h = hash[8] as usize;
        chars[0] = ALPHABET[10 + (h % 26)];
    }
    String::from_utf8(chars).expect("base36 alphabet is ASCII")
}

/// Args for [`run_arm`] — the SPEC-3.0 `git forum migrate` arm.
///
/// `to` is the target storage format. v3.0.0 only accepts `"3.0"`.
/// The clap layer (`src/main.rs`) already constrains the value to
/// `"3.0"` via a `PossibleValuesParser`; this struct stores the
/// validated string so the migration body can branch on it if a
/// future v3.x adds new targets.
pub struct MigrateArgs {
    pub to: String,
    pub dry_run: bool,
    pub as_actor: Option<String>,
}

/// The only migration target accepted in v3.0.0 (SPEC-3.0 §8.1).
pub const SUPPORTED_TARGET: &str = "3.0";

/// Uniform entry point for the `migrate` subcommand.
///
/// Resolves the actor (CLI override → git default → `system/migrate`
/// fallback) and runs the v→3.0 walk. The clap layer rejects any
/// `--to` other than `3.0` before we reach this entry point; the
/// explicit re-check defends against direct callers (tests, future
/// programmatic use).
pub fn run_arm(args: MigrateArgs, ctx: &super::context::Context) -> ForumResult<()> {
    if args.to != SUPPORTED_TARGET {
        return Err(ForumError::Config(format!(
            "unsupported migration target `{}`; v3.0.0 only accepts `--to {SUPPORTED_TARGET}`",
            args.to
        )));
    }
    let actor = args
        .as_actor
        .or_else(|| ctx.git.default_actor().map(str::to_string))
        .unwrap_or_else(|| "system/migrate".to_string());
    let outcome = run(&ctx.git, &ctx.paths, &actor, args.dry_run)?;
    if outcome.threads_with_errors > 0 {
        return Err(ForumError::Config(format!(
            "{} thread(s) failed to migrate; see error lines above",
            outcome.threads_with_errors
        )));
    }
    Ok(())
}

/// Outcome counts for one `git forum migrate` invocation.
///
/// Phase 3 step 6 (item 9-10) replaces this with a structured
/// per-ref report; for now it is a flat counter set used by the
/// summary line and the `run_arm` exit-code computation.
#[derive(Debug, Default)]
pub struct MigrationOutcome {
    pub threads_migrated: usize,
    pub threads_already_migrated: usize,
    pub threads_with_errors: usize,
}

/// Public entry point for `git forum migrate --to 3.0 [--dry-run]`.
///
/// Walks `refs/forum/threads/*`. For each ref tip:
///
/// - `read_snapshot` succeeds → already on 3.0; record + skip.
/// - `read_snapshot` returns [`ForumError::LegacyEventChain`] →
///   project the legacy chain to a 3.0 [`ThreadDocument`] via
///   [`migrate_legacy_to_snapshot`], serialize the source events to
///   NDJSON, and call
///   [`crate::internal::snapshot::store::write_snapshot_with_archive`]
///   to write the snapshot commit on top of the legacy tip. The
///   legacy commits remain reachable as ancestors of the new tip.
/// - any other `read_snapshot` error → record + continue (one bad
///   ref does not abort the run).
///
/// `dry_run` skips writes; the same plan/skip/error lines print.
///
/// Phase 3 step 6 (items 9-10) folds in the structured per-ref
/// report; this commit prints flat status lines and returns a
/// counter [`MigrationOutcome`].
pub fn run(
    git: &GitOps,
    _paths: &RepoPaths,
    actor: &str,
    dry_run: bool,
) -> ForumResult<MigrationOutcome> {
    let prefix = if dry_run { "[DRY-RUN] " } else { "" };
    println!("{prefix}git forum migrate --to 3.0");

    let refs_list = git.list_refs(refs::THREADS_PREFIX)?;
    let mut thread_ids: Vec<String> = refs_list
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    thread_ids.sort();

    let mut outcome = MigrationOutcome::default();
    for thread_id in &thread_ids {
        match snapshot::read_snapshot(git, thread_id) {
            Ok(_) => {
                outcome.threads_already_migrated += 1;
                println!("[skip] {thread_id} (already 3.0)");
            }
            Err(ForumError::LegacyEventChain) => {
                if dry_run {
                    outcome.threads_migrated += 1;
                    println!("[plan] {thread_id} -> 3.0 snapshot");
                } else {
                    match migrate_one(git, thread_id, actor) {
                        Ok(n_events) => {
                            outcome.threads_migrated += 1;
                            println!(
                                "[migrated] {thread_id} -> 3.0 snapshot \
                                 ({n_events} legacy event(s) archived)"
                            );
                        }
                        Err(e) => {
                            outcome.threads_with_errors += 1;
                            eprintln!("error: {thread_id}: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                outcome.threads_with_errors += 1;
                eprintln!("error: {thread_id}: {e}");
            }
        }
    }

    println!(
        "{prefix}migrated={} already={} errors={}",
        outcome.threads_migrated, outcome.threads_already_migrated, outcome.threads_with_errors
    );
    Ok(outcome)
}

/// Migrate one legacy thread to a 3.0 snapshot. Returns the number
/// of source events archived.
///
/// Concurrency: pins the chain tip before reading. The captured
/// OID is the source for both the projection (via
/// `migrate_legacy_to_snapshot_at` / `load_thread_events_at`) and
/// the CAS write (via `write_snapshot_with_archive_pinned`). If
/// another event lands between the pin and the write, the CAS
/// rejects with [`ForumError::SnapshotWriteConflict`] — better than
/// committing a snapshot whose archive is missing the racer's
/// event (task `9635buy0` objection `e630f01f`).
fn migrate_one(git: &GitOps, thread_id: &str, _actor: &str) -> ForumResult<usize> {
    let refname = refs::thread_ref(thread_id);
    let expected_tip = git.resolve_ref(&refname)?.ok_or_else(|| {
        ForumError::Repo(format!("ref {refname} disappeared during migration walk"))
    })?;

    let events = event::load_thread_events_at(git, &expected_tip)?;
    let archive = build_archive_ndjson(&events)?;
    let projection = migrate_legacy_to_snapshot_strict_at(git, thread_id, &expected_tip)?;

    // Item 7 (task `9635buy0`): malformed events surface to stderr
    // as warnings — they do NOT fail migration. Step 6 will route
    // these into a structured per-thread report; until then the
    // inline print is the surface so users still see them.
    for issue in &projection.issues {
        eprintln!("warning: {thread_id}: {issue}");
    }
    // Inferred-metadata note for 1.x chains (objection `efe64dba`):
    // category/tags came from kind inference rather than an
    // explicit `facet_set`. Informational, NOT a validation
    // failure.
    if projection.lifecycle_inferred {
        eprintln!(
            "note: {thread_id}: lifecycle inferred from legacy kind \
             (no facet_set event in chain — normal for 1.x)"
        );
    }

    let message = format!(
        "[git-forum] migrate {thread_id} to 3.0 ({} legacy event(s) archived)",
        events.len()
    );
    snapshot::write_snapshot_with_archive_pinned(
        git,
        thread_id,
        &projection.doc,
        &message,
        &archive,
        &expected_tip,
    )?;
    Ok(events.len())
}

/// Serialize legacy events as NDJSON in chain order (oldest first)
/// for the SPEC-3.0 §8.2 archive blob. One event per line, JSON
/// encoded via the `Event` type's existing `serde::Serialize`. No
/// semantic reconstruction (§8.2 says the archive is for inspection
/// and export, not for read-side semantics).
fn build_archive_ndjson(events: &[event::Event]) -> ForumResult<Vec<u8>> {
    let mut out = Vec::with_capacity(events.len() * 256);
    for ev in events {
        let line = serde_json::to_string(ev)?;
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

// ---------- v→3.0 snapshot bridge ----------

/// SPEC-3.0 §8.3 canonical-tag augmentation by **legacy kind**, which
/// is finer-grained than the lifecycle augmentation in
/// `thread_new::augment_tags_for_lifecycle`. `Lifecycle::Execution`
/// collapses `bug`/`issue`/`task`/`job`, so a lifecycle-only mapping
/// cannot distinguish a `bug`-source thread from a plain `task`.
///
/// Migration uses the legacy kind to recover the §8.3 canonical tag:
/// - `Issue` (covers v1 `bug`/`issue`/`ASK-*`) → `Some("bug")`
/// - `Dec` (covers v1 `dec`/`record`/`DEC-*`) → `Some("decision")`
/// - `Rfc` and `Task` → `None` (no canonical augmentation; the source
///   kind is already the category itself)
///
/// Migration-only helper: non-migrate code paths see SPEC-3.0
/// snapshots already and do not need legacy-kind awareness
/// (ADR-011 Decision 3).
pub fn legacy_kind_to_canonical_tag(kind: ThreadKind) -> Option<&'static str> {
    match kind {
        ThreadKind::Issue => Some("bug"),
        ThreadKind::Dec => Some("decision"),
        ThreadKind::Rfc | ThreadKind::Task => None,
    }
}

/// Read a legacy event-chain thread via the mixed-chain replay
/// reader and project the resulting state back into a SPEC-3.0
/// [`ThreadDocument`].
///
/// Per SPEC-3.0 §8.1 step 4 (and task `9635buy0` item 5), the
/// projected snapshot's `status` is the target category's
/// `initial_status` from `CategoryRegistry::built_in()`, NOT the
/// replayed legacy final status. Migration is intentionally lossy
/// on state: a closed v1 RFC migrates to `draft`, a working v2 task
/// to `open`, and so on. The legacy events themselves remain
/// reachable as ancestor commits and in `legacy/events.ndjson`.
///
/// Per SPEC-3.0 §8.3 (and task item 6), the projected `tags` include
/// canonical augmentation by **legacy kind** (`bug` for `bug`/`issue`,
/// `decision` for `dec`/`record`). The lifecycle-only augmentation
/// is not enough because `Lifecycle::Execution` collapses `bug` and
/// `task` into the same value.
pub fn migrate_legacy_to_snapshot(
    git: &GitOps,
    thread_id: &str,
) -> Result<ThreadDocument, ForumError> {
    let refname = refs::thread_ref(thread_id);
    let tip = git
        .resolve_ref(&refname)?
        .ok_or_else(|| ForumError::Repo(format!("thread {thread_id} not found")))?;
    migrate_legacy_to_snapshot_at(git, thread_id, &tip)
}

/// Lenient projection from a pinned tip. Drops [`StrictReplayIssue`]s
/// silently — only the migrate path's strict variant
/// ([`migrate_legacy_to_snapshot_strict_at`]) surfaces them. Kept
/// for projection unit tests that don't care about issue surfacing.
pub fn migrate_legacy_to_snapshot_at(
    git: &GitOps,
    thread_id: &str,
    start_rev: &str,
) -> Result<ThreadDocument, ForumError> {
    let state = thread::replay_thread_at(git, start_rev)?;
    if state.id != thread_id {
        return Err(ForumError::Repo(format!(
            "rev {start_rev} replays as thread `{}`, not `{thread_id}` — refs out of sync",
            state.id
        )));
    }
    project_state_to_doc(state)
}

/// Output of [`migrate_legacy_to_snapshot_strict_at`].
///
/// Carries the projected document plus enough side-channel data to
/// feed the structured migration report (task `9635buy0` step 6 /
/// items 9-10):
///
/// - `issues` — every [`StrictReplayIssue`] surfaced by strict
///   replay (unknown target node, malformed lifecycle, illegal
///   transition, etc.). Item 7: these flow into the per-thread
///   omissions list, NOT into a hard error — migration still
///   succeeds for the thread.
/// - `lifecycle_inferred` — true when the source chain had no
///   `facet_set` event and the lifecycle/category were derived
///   from the legacy kind. Item 7 calls this an
///   `inferred-metadata` note: informational for 1.x chains, NOT
///   an error (objection `efe64dba`).
#[derive(Debug)]
pub struct MigrationProjection {
    pub doc: ThreadDocument,
    pub issues: Vec<StrictReplayIssue>,
    pub lifecycle_inferred: bool,
}

/// Strict projection from a pinned tip. Routes through
/// [`thread::replay_thread_strict_at`] so malformed events surface
/// in the returned [`MigrationProjection::issues`] vector instead
/// of being silently dropped (task `9635buy0` item 7).
///
/// 1.x chains without a `facet_set` event are NOT an error here:
/// they reach this function with `state.lifecycle_explicit = false`
/// (lifecycle inferred from the create event's `kind`). The
/// returned `lifecycle_inferred` flag exposes this so the caller
/// can record an informational note in the migration report
/// without conflating it with a real validation failure
/// (objection `efe64dba`).
pub fn migrate_legacy_to_snapshot_strict_at(
    git: &GitOps,
    thread_id: &str,
    start_rev: &str,
) -> Result<MigrationProjection, ForumError> {
    let (state, issues) = thread::replay_thread_strict_at(git, start_rev)?;
    if state.id != thread_id {
        return Err(ForumError::Repo(format!(
            "rev {start_rev} replays as thread `{}`, not `{thread_id}` — refs out of sync",
            state.id
        )));
    }
    let lifecycle_inferred = !state.lifecycle_explicit;
    let doc = project_state_to_doc(state)?;
    Ok(MigrationProjection {
        doc,
        issues,
        lifecycle_inferred,
    })
}

/// Project a replayed legacy [`ThreadState`] to a SPEC-3.0
/// [`ThreadDocument`]. Shared by the lenient and strict pinned
/// projection paths.
fn project_state_to_doc(state: ThreadState) -> Result<ThreadDocument, ForumError> {
    let mut tags = state.tags.clone();
    super::thread_new::augment_tags_for_lifecycle(state.lifecycle, &mut tags);
    if let Some(canon) = legacy_kind_to_canonical_tag(state.kind) {
        if !tags.iter().any(|t| t == canon) {
            tags.push(canon.into());
        }
    }
    let category = super::thread_new::lifecycle_to_category(state.lifecycle).to_string();
    // SPEC-3.0 §8.1 step 4: target-category `initial_status`, not
    // the replayed legacy final status. `CategoryRegistry::built_in()`
    // is the right registry here (NOT `policy.effective_registry()`):
    // at migration time a v2 `policy.toml` may not parse under the
    // v3 parser, and the v3 built-ins are the authoritative migration
    // target by SPEC.
    let initial_status = super::super::policy::CategoryRegistry::built_in()
        .get(&category)
        .map(|def| def.initial_status.clone())
        .ok_or_else(|| {
            ForumError::Config(format!(
                "migration target category `{category}` not in built-in registry"
            ))
        })?;

    let nodes: Vec<NodeWithBody> = state
        .nodes
        .iter()
        .filter_map(|n| {
            let kind = match n.node_type.canonical() {
                NodeType::Comment => NodeKind::Comment,
                NodeType::Approval => NodeKind::Approval,
                NodeType::Objection => NodeKind::Objection,
                NodeType::Action => NodeKind::Action,
                _ => return None, // canonical() always returns one of the four
            };
            let status = if n.retracted {
                NodeStatus::Retracted
            } else if n.incorporated {
                NodeStatus::Incorporated
            } else if n.resolved {
                NodeStatus::Resolved
            } else {
                NodeStatus::Open
            };
            Some(NodeWithBody {
                record: NodeRecord {
                    id: n.node_id.clone(),
                    kind,
                    status,
                    created_at: n.created_at,
                    created_by: n.actor.clone(),
                    updated_at: None,
                    updated_by: None,
                    reply_to: n.reply_to.clone(),
                    legacy_label: n.legacy_subtype.clone(),
                },
                body: n.body.clone(),
            })
        })
        .collect();

    let links = Links {
        entries: state
            .links
            .iter()
            .map(|l| Link {
                target: l.target_thread_id.clone(),
                rel: l.rel.clone(),
                created_at: state.created_at,
                created_by: state.created_by.clone(),
            })
            .collect(),
    };

    let evidence = EvidenceFile {
        entries: state
            .evidence_items
            .iter()
            .map(|e| EvidenceRecord {
                id: e.evidence_id.clone(),
                kind: e.kind.clone(),
                ref_target: e.ref_target.clone(),
                created_at: state.created_at,
                created_by: state.created_by.clone(),
            })
            .collect(),
    };

    Ok(ThreadDocument {
        snapshot: ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: state.id,
            title: state.title,
            category,
            status: initial_status,
            tags,
            created_at: state.created_at,
            created_by: state.created_by.clone(),
            updated_at: state.created_at,
            updated_by: state.created_by,
            branch: state.branch,
            supersedes: Vec::new(),
        },
        body: state.body,
        nodes,
        links,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // SPEC-3.0 §8.3 canonical-tag table — kind-keyed augmentation.
    #[test]
    fn legacy_kind_to_canonical_tag_covers_spec_table() {
        assert_eq!(legacy_kind_to_canonical_tag(ThreadKind::Issue), Some("bug"));
        assert_eq!(
            legacy_kind_to_canonical_tag(ThreadKind::Dec),
            Some("decision")
        );
        // RFC and Task source kinds carry no canonical extra tag —
        // the category itself is the classification.
        assert_eq!(legacy_kind_to_canonical_tag(ThreadKind::Rfc), None);
        assert_eq!(legacy_kind_to_canonical_tag(ThreadKind::Task), None);
    }

    // Pin the v→2 alias-token algorithm to its byte-exact original.
    // Repos already migrated to 2.0 wrote alias entries at
    // `refs/forum/aliases/<legacy-id>` that point at
    // `refs/forum/threads/<derive_token_from_seed(legacy-id)>`. The
    // 2.0 alias-resolution path in `internal::thread::resolve_thread_id`
    // recomputes this mapping at read time, so any drift here
    // silently breaks alias resolution for sequential legacy IDs.
    #[test]
    fn bare_token_for_pinned_v2_mappings() {
        // Sequential IDs that hit `derive_token_from_seed` (the
        // common shape for repos that ran v2 migration).
        assert_eq!(bare_token_for("RFC-0001"), "sb7fmsjj");
        assert_eq!(bare_token_for("RFC-0042"), "eid815ln");
        assert_eq!(bare_token_for("ASK-0001"), "rd3gy4j9");
        assert_eq!(bare_token_for("JOB-0001"), "ogvib034");
        assert_eq!(bare_token_for("DEC-0001"), "yek2fmxc");
        // RFC-0007 is the smallest sequential RFC whose seed-hash
        // first byte was a digit — this exercises the forced-letter
        // branch that uses `hash[8]` (NOT `hash[0]`) to pick the
        // replacement letter.
        assert_eq!(bare_token_for("RFC-0007"), "bki6w227");
    }

    #[test]
    fn bare_token_for_strips_opaque_prefix() {
        assert_eq!(bare_token_for("RFC-a7f3b2x1"), "a7f3b2x1");
        assert_eq!(bare_token_for("ASK-q3kfj49v"), "q3kfj49v");
        assert_eq!(bare_token_for("JOB-x8n2q1d4"), "x8n2q1d4");
    }

    #[test]
    fn bare_token_for_passthrough_already_bare() {
        assert_eq!(bare_token_for("a7f3b2x1"), "a7f3b2x1");
    }

    #[test]
    fn bare_token_for_sequential_yields_legal_bare_token() {
        let token = bare_token_for("RFC-0007");
        assert!(
            id_alloc::is_bare_token(&token),
            "must satisfy bare-token grammar: {token}"
        );
    }
}
