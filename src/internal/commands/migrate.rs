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

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::config::RepoPaths;
use super::super::error::{ForumError, ForumResult};
use super::super::evidence::{EvidenceFile, EvidenceRecord};
use super::super::git_ops::GitOps;
use super::super::id_alloc;
use super::super::legacy::event::{self, ThreadKind};
use super::super::node::{NodeRecord, NodeStatus};
use super::super::refs;
use super::super::snapshot::{self, Link, Links, NodeWithBody, ThreadDocument};
use super::super::thread::{ThreadSnapshot, ThreadState};
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
///
/// Exit code: non-zero only when at least one thread's outcome is
/// [`ThreadOutcome::Error`]. `migrated-with-omissions` is success
/// (SPEC-3.0 §8 + task item 10).
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
    let report = run(&ctx.git, &ctx.paths, &actor, args.dry_run)?;
    let errors = report
        .threads
        .iter()
        .filter(|t| matches!(t.outcome, ThreadOutcome::Error))
        .count();
    if errors > 0 {
        return Err(ForumError::Config(format!(
            "{errors} thread(s) failed to migrate; see error lines above and report at \
             {}",
            ctx.paths.git_forum.join(REPORT_FILENAME).display(),
        )));
    }
    Ok(())
}

/// JSON file name under `RepoPaths::git_forum` for the
/// machine-readable migration report (SPEC-3.0 §4.3 local clone
/// state; task `9635buy0` item 10).
pub const REPORT_FILENAME: &str = "migration-report.json";

/// Per-ref outcome of a migration walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadOutcome {
    /// Legacy chain projected and snapshot written (or, in
    /// `--dry-run`, would be written). May carry omissions.
    Migrated,
    /// Tip already a SPEC-3.0 snapshot; no action needed.
    AlreadyMigrated,
    /// Read, projection, or write failed irrecoverably for this
    /// ref. Other refs continue; the run as a whole returns a
    /// non-zero exit code.
    Error,
}

/// One projection-time anomaly that survived in the report.
///
/// SPEC-3.0 §8.2 / task `9635buy0` item 9: every strict-replay
/// issue or otherwise-dropped material on a successfully-migrated
/// thread is recorded here so post-migration inspection has a
/// machine-readable trail.
#[derive(Debug, Clone, Serialize)]
pub struct Omission {
    pub kind: String,
    pub item: String,
    pub reason: String,
}

/// Per-thread report entry written to `migration-report.json`.
///
/// Fields with `skip_serializing_if` collapse to absent JSON keys
/// when empty so the on-disk file stays compact.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadReport {
    pub thread_id: String,
    pub ref_name: String,
    pub outcome: ThreadOutcome,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub omissions: Vec<Omission>,
    /// Present only when `outcome == Error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Present when category/tags were inferred from the legacy
    /// kind (1.x chain with no facet_set). Informational —
    /// objection `efe64dba` requires this NOT to fail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_metadata: Option<String>,
    /// Number of legacy events archived (only for migrated outcomes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_events: Option<usize>,
}

/// Top-level migration report (SPEC-3.0 §8 / task item 9).
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub generated_at: DateTime<Utc>,
    pub dry_run: bool,
    pub threads: Vec<ThreadReport>,
}

/// Public entry point for `git forum migrate --to 3.0 [--dry-run]`.
///
/// Walks `refs/forum/threads/*`. Per-ref:
///
/// - `read_snapshot` succeeds → `outcome = AlreadyMigrated`; skip.
/// - `read_snapshot` returns [`ForumError::LegacyEventChain`] →
///   project (strict, pinned tip), in non-dry-run mode write the
///   snapshot via [`snapshot::store::write_snapshot_with_archive_pinned`].
///   Strict-replay issues record as `omissions`; outcome is
///   `Migrated`.
/// - any other `read_snapshot` / load / project / write error →
///   `outcome = Error`; subsequent refs continue.
///
/// Persists the structured report as JSON at
/// `paths.git_forum.join("migration-report.json")` (SPEC-3.0 §4.3
/// local-clone state). `dry_run` skips snapshot writes but the
/// report is still produced and persisted so callers can inspect
/// the planned work.
pub fn run(
    git: &GitOps,
    paths: &RepoPaths,
    _actor: &str,
    dry_run: bool,
) -> ForumResult<MigrationReport> {
    let prefix = if dry_run { "[DRY-RUN] " } else { "" };
    println!("{prefix}git forum migrate --to 3.0");

    let refs_list = git.list_refs(refs::THREADS_PREFIX)?;
    let mut thread_ids: Vec<String> = refs_list
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    thread_ids.sort();

    let mut threads: Vec<ThreadReport> = Vec::with_capacity(thread_ids.len());
    for thread_id in &thread_ids {
        let report = process_one(git, thread_id, dry_run);
        // Surface skip/error to user-facing streams; migrated +
        // plan lines are printed inside `process_one`.
        match report.outcome {
            ThreadOutcome::AlreadyMigrated => {
                println!("[skip] {} (already 3.0)", report.thread_id);
            }
            ThreadOutcome::Error => {
                if let Some(err) = &report.error {
                    eprintln!("error: {}: {err}", report.thread_id);
                }
            }
            ThreadOutcome::Migrated => {}
        }
        threads.push(report);
    }

    let migrated = threads
        .iter()
        .filter(|t| matches!(t.outcome, ThreadOutcome::Migrated))
        .count();
    let already = threads
        .iter()
        .filter(|t| matches!(t.outcome, ThreadOutcome::AlreadyMigrated))
        .count();
    let errors = threads
        .iter()
        .filter(|t| matches!(t.outcome, ThreadOutcome::Error))
        .count();
    println!("{prefix}migrated={migrated} already={already} errors={errors}");

    let report = MigrationReport {
        generated_at: Utc::now(),
        dry_run,
        threads,
    };

    // Persist under `.git/forum/` (SPEC-3.0 §4.3 local-clone state;
    // never the working tree). The directory is created by
    // `init_forum`; create defensively in case migrate is invoked
    // pre-init or the directory was removed manually.
    std::fs::create_dir_all(&paths.git_forum)?;
    let report_path = paths.git_forum.join(REPORT_FILENAME);
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, json)?;
    println!("{prefix}wrote report to {}", report_path.display());

    Ok(report)
}

/// Build a [`ThreadReport`] for one ref. Handles probe → project →
/// (write if not dry-run). All error paths fold into
/// `outcome = Error` with `error` populated; the walk in [`run`]
/// continues to the next ref.
///
/// Concurrency: pins the chain tip before reading. The captured
/// OID feeds both the projection (`migrate_legacy_to_snapshot_strict_at` /
/// `load_event_tail_at`) and the CAS write
/// (`write_snapshot_with_archive_pinned`). If another event lands
/// between the pin and the write, the CAS rejects with
/// [`ForumError::SnapshotWriteConflict`] — recorded here as
/// `outcome = Error` instead of silently committing a snapshot
/// whose archive is missing the racer's event (task `9635buy0`
/// objection `e630f01f`).
fn process_one(git: &GitOps, thread_id: &str, dry_run: bool) -> ThreadReport {
    let ref_name = refs::thread_ref(thread_id);
    let mut report = ThreadReport {
        thread_id: thread_id.to_string(),
        ref_name: ref_name.clone(),
        outcome: ThreadOutcome::Error,
        omissions: Vec::new(),
        error: None,
        inferred_metadata: None,
        archived_events: None,
    };

    match snapshot::read_snapshot(git, thread_id) {
        Ok(_) => {
            report.outcome = ThreadOutcome::AlreadyMigrated;
            return report;
        }
        Err(ForumError::LegacyEventChain) => {} // proceed
        Err(e) => {
            report.error = Some(format!("{e}"));
            return report;
        }
    }

    let expected_tip = match git.resolve_ref(&ref_name) {
        Ok(Some(tip)) => tip,
        Ok(None) => {
            report.error = Some(format!("ref {ref_name} disappeared during migration walk"));
            return report;
        }
        Err(e) => {
            report.error = Some(format!("resolve_ref({ref_name}): {e}"));
            return report;
        }
    };

    // Phase-2 cutover ref shape: tip is event, ancestor may be a
    // snapshot. Walk only the event tail; surface the snapshot
    // ancestor (if any) as an `archive` omission so the report
    // explains why the new `legacy/events.ndjson` carries only the
    // tail (task `9635buy0`, objection `bf678561`).
    let (events, snapshot_ancestor) = match event::load_event_tail_at(git, &expected_tip) {
        Ok(pair) => pair,
        Err(e) => {
            report.error = Some(format!("load_event_tail_at: {e}"));
            return report;
        }
    };

    let projection = match migrate_legacy_to_snapshot_strict_at(git, thread_id, &expected_tip) {
        Ok(p) => p,
        Err(e) => {
            report.error = Some(format!("projection: {e}"));
            return report;
        }
    };

    for issue in &projection.issues {
        report.omissions.push(strict_issue_to_omission(issue));
    }
    report
        .omissions
        .extend(projection.omissions.iter().cloned());
    if let Some(anc) = &snapshot_ancestor {
        report.omissions.push(Omission {
            kind: "archive".into(),
            item: anc.clone(),
            reason: "snapshot ancestor; new legacy/events.ndjson contains only events after this commit (the ancestor's archive is preserved on its own commit)".into(),
        });
    }
    if projection.lifecycle_inferred {
        report.inferred_metadata = Some(
            "lifecycle inferred from legacy kind (no facet_set in chain — normal for 1.x)".into(),
        );
    }

    // Inline surface — keeps the user-facing stream useful even
    // before they crack open the JSON report.
    for o in &report.omissions {
        eprintln!(
            "warning: {thread_id}: [{}] {} — {}",
            o.kind, o.item, o.reason
        );
    }
    if let Some(note) = &report.inferred_metadata {
        eprintln!("note: {thread_id}: {note}");
    }

    if dry_run {
        report.outcome = ThreadOutcome::Migrated;
        report.archived_events = Some(events.len());
        println!("[plan] {thread_id} -> 3.0 snapshot");
        return report;
    }

    let archive = match build_archive_ndjson(&events) {
        Ok(a) => a,
        Err(e) => {
            report.error = Some(format!("build_archive_ndjson: {e}"));
            return report;
        }
    };
    let message = format!(
        "[git-forum] migrate {thread_id} to 3.0 ({} legacy event(s) archived)",
        events.len()
    );
    if let Err(e) = snapshot::write_snapshot_with_archive_pinned(
        git,
        thread_id,
        &projection.doc,
        &message,
        &archive,
        &expected_tip,
    ) {
        report.error = Some(format!("write_snapshot_with_archive_pinned: {e}"));
        return report;
    }

    report.outcome = ThreadOutcome::Migrated;
    report.archived_events = Some(events.len());
    println!(
        "[migrated] {thread_id} -> 3.0 snapshot ({} legacy event(s) archived)",
        events.len()
    );
    report
}

/// Project a [`StrictReplayIssue`] into an [`Omission`] entry for
/// the structured report. Each variant maps to a `(kind, item,
/// reason)` triple so JSON consumers can filter by `kind` without
/// pattern-matching on free-form text.
fn strict_issue_to_omission(issue: &StrictReplayIssue) -> Omission {
    use StrictReplayIssue::*;
    match issue {
        UnknownTargetNode {
            event_id,
            event_type,
            target_node_id,
        } => Omission {
            kind: "node".into(),
            item: target_node_id.clone(),
            reason: format!("{event_type} event {event_id} targets unknown node"),
        },
        MissingRequiredField {
            event_id,
            event_type,
            field,
        } => Omission {
            kind: "field".into(),
            item: (*field).into(),
            reason: format!("{event_type} event {event_id} is missing required field"),
        },
        LifecycleResetAttempted {
            event_id,
            existing,
            attempted,
        } => Omission {
            kind: "lifecycle".into(),
            item: attempted.clone(),
            reason: format!(
                "facet_set event {event_id} attempted to reset lifecycle from `{existing}`"
            ),
        },
        InvalidLifecycleValue { event_id, value } => Omission {
            kind: "lifecycle".into(),
            item: value.clone(),
            reason: format!("facet_set event {event_id} carries unknown lifecycle"),
        },
        InvalidStateValue { event_id, value } => Omission {
            kind: "state".into(),
            item: value.clone(),
            reason: format!("state event {event_id} carries unparseable status"),
        },
        InvalidTransition {
            event_id,
            from,
            to,
            lifecycle,
        } => Omission {
            kind: "transition".into(),
            item: format!("{from}->{to}"),
            reason: format!("state event {event_id} edge not legal under lifecycle `{lifecycle}`"),
        },
    }
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
/// and projection [`Omission`]s silently — only the migrate path's
/// strict variant ([`migrate_legacy_to_snapshot_strict_at`])
/// surfaces them. Kept for projection unit tests that don't care
/// about issue surfacing.
pub fn migrate_legacy_to_snapshot_at(
    git: &GitOps,
    thread_id: &str,
    start_rev: &str,
) -> Result<ThreadDocument, ForumError> {
    let state = super::super::legacy::chain_replay::replay_chain_at(git, start_rev)?;
    if state.id != thread_id {
        return Err(ForumError::Repo(format!(
            "rev {start_rev} replays as thread `{}`, not `{thread_id}` — refs out of sync",
            state.id
        )));
    }
    let (doc, _omissions) = project_state_to_doc(state)?;
    Ok(doc)
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
/// - `omissions` — projection-time omissions that don't belong on
///   the [`StrictReplayIssue`] enum: invalid tags dropped per
///   §2.4, in particular (objection `e285682f`). Surfaced
///   alongside `issues` in the per-thread report.
/// - `lifecycle_inferred` — true when the source chain had no
///   `facet_set` event and the lifecycle/category were derived
///   from the legacy kind. Item 7 calls this an
///   `inferred-metadata` note: informational for 1.x chains, NOT
///   an error (objection `efe64dba`).
#[derive(Debug)]
pub struct MigrationProjection {
    pub doc: ThreadDocument,
    pub issues: Vec<StrictReplayIssue>,
    pub omissions: Vec<Omission>,
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
    let (state, issues) =
        super::super::legacy::chain_replay::replay_chain_strict_at(git, start_rev)?;
    if state.id != thread_id {
        return Err(ForumError::Repo(format!(
            "rev {start_rev} replays as thread `{}`, not `{thread_id}` — refs out of sync",
            state.id
        )));
    }
    let lifecycle_inferred = !state.lifecycle_explicit;
    let (doc, omissions) = project_state_to_doc(state)?;
    Ok(MigrationProjection {
        doc,
        issues,
        omissions,
        lifecycle_inferred,
    })
}

/// Return a tree-safe form of a legacy node ID.
///
/// Legacy approval nodes use `<event_sha>#<actor_id>` and `actor_id`
/// commonly carries a namespace separator (`human/alice`).
/// `git mktree` rejects path components that contain `/`, so the
/// projection MUST scrub legacy node IDs before they are used as
/// `nodes/<id>.toml` filenames in the snapshot tree (SPEC-3.0
/// §4.2).
///
/// Replacement is deterministic — `/` → `-` everywhere — so any
/// `reply_to` references on other nodes can apply the same
/// transform and remain consistent. There is a vanishingly small
/// chance of collision when two legacy actor IDs differ only by a
/// `/` vs `-` at the same position (e.g. `human/alice-bob` vs
/// `human-alice/bob` both fold to `human-alice-bob`); we accept
/// this loss as part of the SPEC-3.0 §8 one-way lossy migration
/// contract.
fn tree_safe_node_id(legacy_id: &str) -> String {
    legacy_id.replace('/', "-")
}

/// Project a replayed legacy [`ThreadState`] to a SPEC-3.0
/// [`ThreadDocument`]. Shared by the lenient and strict pinned
/// projection paths.
///
/// Returns `(doc, omissions)` so the strict path can surface
/// projection-time omissions in the report. The lenient
/// pinned wrapper (`migrate_legacy_to_snapshot_at`) discards
/// them.
///
/// Tag validation (SPEC-2.0 §2.4 / SPEC-3.0 §2.4): legacy chains
/// can carry tags that violate the 3.0 grammar (length, leading
/// letter, allowed character set, reserved literal). Migration
/// drops invalid tags from the projected snapshot's `tags` and
/// records each as a `kind: "tag"` omission, per task `9635buy0`
/// objection `e285682f`. The legacy event chain still has the
/// original tag visible via `legacy/events.ndjson`.
fn project_state_to_doc(state: ThreadState) -> Result<(ThreadDocument, Vec<Omission>), ForumError> {
    let mut omissions: Vec<Omission> = Vec::new();
    let mut tags: Vec<String> = Vec::with_capacity(state.tags.len());
    for tag in &state.tags {
        match super::super::legacy::event::validate_tag(tag) {
            Ok(()) => {
                if !tags.iter().any(|t| t == tag) {
                    tags.push(tag.clone());
                }
            }
            Err(reason) => omissions.push(Omission {
                kind: "tag".into(),
                item: tag.clone(),
                reason,
            }),
        }
    }
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
        .map(|n| {
            // After v3.1 step 3g, v2 Node.node_type is already NodeKind
            // (the canonical 4); the v1 fold happened upstream during
            // legacy event chain replay.
            let status = if n.retracted {
                NodeStatus::Retracted
            } else if n.incorporated {
                NodeStatus::Incorporated
            } else if n.resolved {
                NodeStatus::Resolved
            } else {
                NodeStatus::Open
            };
            NodeWithBody {
                record: NodeRecord {
                    id: tree_safe_node_id(&n.node_id),
                    kind: n.node_type,
                    status,
                    created_at: n.created_at,
                    created_by: n.actor.clone(),
                    updated_at: None,
                    updated_by: None,
                    reply_to: n.reply_to.as_deref().map(tree_safe_node_id),
                    legacy_label: n.legacy_subtype.clone(),
                },
                body: n.body.clone(),
            }
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

    Ok((
        ThreadDocument {
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
        },
        omissions,
    ))
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

    #[test]
    fn tree_safe_node_id_strips_actor_namespace_slash() {
        // v2 approval nodes use `<event_sha>#<actor_id>` and the
        // actor commonly carries a namespace (`human/alice`).
        // The projection MUST strip the slash so the node ID is
        // legal as a `nodes/<id>.toml` filename in the SPEC-3.0
        // §4.2 snapshot tree.
        assert_eq!(
            tree_safe_node_id("8ae3c8ffc9f505641a86e628947212b4cc995ceb#human/alice"),
            "8ae3c8ffc9f505641a86e628947212b4cc995ceb#human-alice"
        );
        // Pure event SHAs (the common comment/objection/action
        // shape) pass through unchanged.
        let plain = "8ae3c8ffc9f505641a86e628947212b4cc995ceb";
        assert_eq!(tree_safe_node_id(plain), plain);
        // Multiple slashes (defensive — shouldn't happen in v2 but
        // we replace every occurrence to keep the contract simple).
        assert_eq!(tree_safe_node_id("abc#human/foo/bar"), "abc#human-foo-bar");
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
