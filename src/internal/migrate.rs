//! `git forum migrate` — one-shot 1.x → 2.0 migration tool (Track C).
//!
//! Implements ADR-004 / SPEC-2.0 §10. The migrator:
//!
//! 1. Walks `refs/forum/threads/<KIND>-XXXX...` and rewrites each thread to
//!    `refs/forum/threads/<bare-token>`. The original kind-prefixed name is
//!    preserved as a read-only alias entry under `refs/forum/aliases/<old-id>`
//!    so external links (e.g. `RFC-0001`, `ASK-XXXXXXXX`) keep resolving.
//! 2. Rewrites each event in the chain:
//!    - `thread_id` → bare token
//!    - `new_state` (1.x names) → 2.0 canonical (`migrate_legacy_state`)
//!    - `node_type` (legacy 1.x prose-only types) → `comment` with
//!      `legacy_subtype` preserved (ADR-006 / §2.5)
//! 3. Appends a synthetic `facet_set` event populating `lifecycle` and the
//!    conventional `tags` per the §2.3.3 mapping.
//! 4. Auto-rewrites `policy.toml` keys: `creation_rules.<kind>` →
//!    `creation_rules.<lifecycle>`/tag overlay; `[[guards]] on = "<kind>:..."`
//!    → `lifecycle=<lifecycle>` predicate. Each rewrite is logged with
//!    file/line.
//! 5. Emits a warning for any `policy.toml` line still mentioning the
//!    removed `at_least_one_summary` predicate (ADR-006).
//! 6. Scans `.forum/` shipped scripts/READMEs for kind-prefixed subcommand
//!    invocations (`git forum rfc new`, etc.) and emits a one-time warning
//!    per occurrence (RFC-nm3d31yk Q1: subcommand groupings removed in 2.0).
//!
//! Idempotent: a thread already in bare-token form whose chain already
//! contains a `facet_set` event is skipped.
//!
//! `--dry-run` reports the plan without writing anything.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::config::RepoPaths;
use super::error::{ForumError, ForumResult};
use super::event::{self, Event, EventType, Lifecycle, ThreadKind};
use super::git_ops::GitOps;
use super::id_alloc;
use super::refs;
use super::state_machine;

/// SPEC-2.0 §10: 2.0 alias entries live under `refs/forum/aliases/<old-id>`
/// and point at the same commit as the canonical thread ref. They are
/// consulted by [`super::thread::resolve_thread_id`] so legacy IDs continue
/// to resolve after migration.
pub const ALIASES_PREFIX: &str = "refs/forum/aliases/";

/// Construct the alias ref name for a legacy thread ID.
pub fn alias_ref(legacy_id: &str) -> String {
    format!("{ALIASES_PREFIX}{legacy_id}")
}

/// Top-level migration plan.
#[derive(Debug, Default)]
pub struct MigrationPlan {
    pub threads: Vec<ThreadPlan>,
    /// Lines in `.forum/policy.toml` flagged for `at_least_one_summary`.
    pub policy_warnings: Vec<String>,
    /// Policy-key rewrites performed (or planned).
    pub policy_rewrites: Vec<String>,
    /// Subcommand-grouping occurrences detected in `.forum/` scripts.
    pub script_warnings: Vec<String>,
}

/// Migration plan for a single 1.x thread.
#[derive(Debug)]
pub struct ThreadPlan {
    pub legacy_id: String,
    pub new_id: String,
    pub kind: ThreadKind,
    pub lifecycle: Lifecycle,
    pub conventional_tags: Vec<String>,
    /// Number of node events whose type was rewritten to canonical.
    pub node_rewrites: usize,
    /// Number of state events whose `new_state` was normalized to a 2.0 name.
    pub state_rewrites: usize,
    pub event_count: usize,
    /// True when no rewrite work is needed (already 2.0 canonical).
    pub already_migrated: bool,
}

/// Outcome of an actual migration run.
#[derive(Debug, Default)]
pub struct MigrationOutcome {
    pub threads_migrated: usize,
    pub threads_skipped: usize,
    pub policy_warnings: Vec<String>,
    pub policy_rewrites: Vec<String>,
    pub script_warnings: Vec<String>,
}

/// SPEC-2.0 §2.3.3 — the conventional tag overlay applied to each 1.x kind.
fn conventional_tags(kind: ThreadKind) -> Vec<String> {
    match kind {
        ThreadKind::Rfc => vec!["cross-cutting".into()],
        ThreadKind::Issue => vec!["bug".into()],
        ThreadKind::Task => vec!["task".into()],
        ThreadKind::Dec => vec![],
    }
}

/// Compute a deterministic 2.0 bare token for a legacy thread ID.
///
/// - Opaque IDs (`KIND-<8 base36>`): strip the prefix; the suffix is already a
///   bare token and has the same uniqueness guarantee.
/// - Sequential IDs (`KIND-NNNN`): hash the legacy ID and project to 8 base36
///   characters. The leading character is forced to a letter so the result is
///   never all-digit (the bare-token grammar forbids that).
/// - Already-bare tokens: returned unchanged.
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
    // Sequential or otherwise-irregular: derive deterministic token.
    derive_token_from_seed(legacy_id)
}

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
    // Force first character to a letter so the token is never all-digit
    // (id_alloc::is_bare_token rejects all-digit tokens).
    if !(chars[0] as char).is_ascii_lowercase() {
        let h = hash[8] as usize;
        chars[0] = ALPHABET[10 + (h % 26)];
    }
    String::from_utf8(chars).expect("base36 alphabet is ASCII")
}

/// Determine which 1.x kind a thread carries from its Create event, falling
/// back to the legacy ref-name prefix when the field is absent.
fn detect_kind(events: &[Event], legacy_id: &str) -> ForumResult<ThreadKind> {
    if let Some(create) = events.first() {
        if let Some(kind) = create.kind {
            return Ok(kind);
        }
    }
    legacy_id
        .split_once('-')
        .and_then(|(prefix, _)| ThreadKind::from_id_prefix(prefix))
        .ok_or_else(|| {
            ForumError::Repo(format!(
                "thread {legacy_id}: cannot determine kind (Create event lacks `kind` and \
                 ref name has no recognizable prefix)"
            ))
        })
}

/// Whether a chain already contains a `facet_set` event (the idempotency
/// signal: a chain with one is fully 2.0 and needs no further work).
fn chain_already_migrated(events: &[Event]) -> bool {
    events
        .iter()
        .any(|e| e.event_type == EventType::FacetSet && e.lifecycle.is_some())
}

/// Build the migration plan for a single thread without modifying anything.
fn plan_thread(git: &GitOps, legacy_id: &str) -> ForumResult<ThreadPlan> {
    let events = event::load_thread_events(git, legacy_id)?;
    let kind = detect_kind(&events, legacy_id)?;
    let new_id = bare_token_for(legacy_id);

    let mut node_rewrites = 0usize;
    let mut state_rewrites = 0usize;
    for ev in &events {
        match ev.event_type {
            EventType::Say | EventType::Retype => {
                if let Some(nt) = ev.node_type {
                    if !nt.is_canonical() {
                        node_rewrites += 1;
                    }
                }
            }
            EventType::State => {
                if let Some(s) = ev.new_state.as_deref() {
                    if state_machine::migrate_legacy_state(kind, s) != s {
                        state_rewrites += 1;
                    }
                }
            }
            _ => {}
        }
    }

    let already_migrated =
        new_id == legacy_id && chain_already_migrated(&events) && node_rewrites == 0;

    Ok(ThreadPlan {
        legacy_id: legacy_id.to_string(),
        new_id,
        kind,
        lifecycle: kind.lifecycle(),
        conventional_tags: conventional_tags(kind),
        node_rewrites,
        state_rewrites,
        event_count: events.len(),
        already_migrated,
    })
}

/// Transform an event for the migrated chain.
///
/// - `thread_id` → bare token
/// - canonicalize legacy node types (sets `legacy_subtype`)
/// - normalize 1.x state names to 2.0 (`migrate_legacy_state`)
fn rewrite_event(ev: &Event, kind: ThreadKind, new_id: &str) -> Event {
    let mut out = ev.clone();
    out.event_id = String::new(); // recomputed at write time
    out.thread_id = new_id.to_string();

    if let Some(node_type) = out.node_type {
        if !node_type.is_canonical() {
            if let Some(label) = node_type.legacy_subtype_label() {
                if out.legacy_subtype.is_none() {
                    out.legacy_subtype = Some(label.to_string());
                }
            }
            out.node_type = Some(node_type.canonical());
        }
    }
    if let Some(old_node_type) = out.old_node_type {
        if !old_node_type.is_canonical() {
            out.old_node_type = Some(old_node_type.canonical());
        }
    }

    if out.event_type == EventType::State {
        if let Some(s) = out.new_state.clone() {
            let migrated = state_machine::migrate_legacy_state(kind, &s).to_string();
            if migrated != s {
                out.new_state = Some(migrated);
            }
        }
    }

    out
}

/// Build the trailing `facet_set` event that records `lifecycle` + tags for
/// a migrated thread (SPEC-2.0 §10.1, §2.3.3).
fn build_facet_set_event(plan: &ThreadPlan, parent_event: &Event, actor: &str) -> Event {
    let created_at = parent_event.created_at + chrono::Duration::milliseconds(1);
    Event {
        thread_id: plan.new_id.clone(),
        event_type: EventType::FacetSet,
        created_at,
        actor: actor.to_string(),
        ..Event::default()
    }
    .with_lifecycle(plan.lifecycle.as_str())
    .with_tags_add(plan.conventional_tags.clone())
}

/// Execute a planned migration for one thread, building the new commit chain
/// and updating refs.  Returns the new tip SHA.
fn apply_thread(git: &GitOps, plan: &ThreadPlan, actor: &str) -> ForumResult<String> {
    let events = event::load_thread_events(git, &plan.legacy_id)?;
    let mut parent: Option<String> = None;
    let mut last_event: Option<Event> = None;
    for ev in &events {
        let rewritten = rewrite_event(ev, plan.kind, &plan.new_id);
        let parent_sha = write_commit(git, &rewritten, parent.as_deref())?;
        parent = Some(parent_sha);
        last_event = Some(rewritten);
    }
    let last = last_event.expect("thread has at least one event");
    let facet_event = build_facet_set_event(plan, &last, actor);
    let final_sha = write_commit(git, &facet_event, parent.as_deref())?;

    let new_ref = refs::thread_ref(&plan.new_id);
    let alias_ref_name = alias_ref(&plan.legacy_id);

    if plan.new_id == plan.legacy_id {
        // Already-bare token: the legacy ref *is* the canonical ref;
        // overwrite it in place. No alias needed.
        git.update_ref(&new_ref, &final_sha)?;
    } else {
        let legacy_ref = refs::thread_ref(&plan.legacy_id);
        // Create the new canonical ref (must not already exist; if a prior
        // migrate run already moved it, the caller filters that case out via
        // `already_migrated`).
        match git.resolve_ref(&new_ref)? {
            Some(_) => git.update_ref(&new_ref, &final_sha)?,
            None => git.create_ref(&new_ref, &final_sha)?,
        }
        // Move the legacy ref out of `refs/forum/threads/` so list_thread_ids
        // returns the canonical token only. We park it under
        // `refs/forum/aliases/<old-id>` pointing at the migrated tip so
        // resolve_thread_id can still find it.
        git.update_ref(&alias_ref_name, &final_sha)?;
        git.delete_ref(&legacy_ref)?;
    }

    Ok(final_sha)
}

/// Write a single migrated event as a Git commit, returning its SHA. Unlike
/// [`event::write_event`], this does not consult or update any ref — the
/// migrator builds the chain by piping each commit's parent forward.
fn write_commit(git: &GitOps, event: &Event, parent: Option<&str>) -> ForumResult<String> {
    event.validate()?;
    let json = serde_json::to_string_pretty(event)?;
    let blob = git.hash_object(json.as_bytes())?;
    let tree = git.mktree_single("event.json", &blob)?;
    let parents: Vec<&str> = parent.into_iter().collect();
    let message = format!(
        "[git-forum] {} {} (migrated)",
        event.event_type, event.thread_id
    );
    git.commit_tree(&tree, &parents, &message)
}

/// Public entry point for `git forum migrate [--dry-run]`.
///
/// `actor` is recorded as the author of the synthetic `facet_set` events
/// (typically `"system/migrate"`).
pub fn run(
    git: &GitOps,
    paths: &RepoPaths,
    actor: &str,
    dry_run: bool,
) -> ForumResult<MigrationOutcome> {
    let plan = build_plan(git, paths)?;
    print_plan_header(&plan, dry_run);
    let mut outcome = MigrationOutcome {
        policy_warnings: plan.policy_warnings.clone(),
        script_warnings: plan.script_warnings.clone(),
        ..MigrationOutcome::default()
    };

    for warn in &plan.policy_warnings {
        eprintln!("warning: {warn}");
    }
    for warn in &plan.script_warnings {
        eprintln!("warning: {warn}");
    }

    for tp in &plan.threads {
        if tp.already_migrated {
            outcome.threads_skipped += 1;
            println!(
                "[skip] {} (already 2.0 canonical: id={}, lifecycle={}, tags=[{}])",
                tp.legacy_id,
                tp.new_id,
                tp.lifecycle,
                tp.conventional_tags.join(", "),
            );
            continue;
        }
        if dry_run {
            println!(
                "[plan] {} -> @{} ({} events; {} node rewrites; {} state rewrites; \
                 lifecycle={} tags=[{}])",
                tp.legacy_id,
                tp.new_id,
                tp.event_count,
                tp.node_rewrites,
                tp.state_rewrites,
                tp.lifecycle,
                tp.conventional_tags.join(", "),
            );
            outcome.threads_migrated += 1;
        } else {
            let new_tip = apply_thread(git, tp, actor)?;
            outcome.threads_migrated += 1;
            println!(
                "[migrated] {} -> @{} (tip={}, +{} node rewrites, +{} state rewrites)",
                tp.legacy_id,
                tp.new_id,
                &new_tip[..new_tip.len().min(12)],
                tp.node_rewrites,
                tp.state_rewrites,
            );
        }
    }

    if !dry_run {
        let rewrites = rewrite_policy_file(paths)?;
        for r in &rewrites {
            println!("[policy] {r}");
        }
        outcome.policy_rewrites = rewrites;
    } else {
        // Surface the rewrites that *would* happen.
        outcome.policy_rewrites = plan.policy_rewrites.clone();
        for r in &plan.policy_rewrites {
            println!("[plan-policy] {r}");
        }
    }

    print_summary(&outcome, dry_run);
    Ok(outcome)
}

/// Build the full migration plan (all threads, policy diagnostics, script
/// scan). No writes.
pub fn build_plan(git: &GitOps, paths: &RepoPaths) -> ForumResult<MigrationPlan> {
    let mut plan = MigrationPlan::default();

    // Thread plans.
    let refs_list = git.list_refs(refs::THREADS_PREFIX)?;
    let mut legacy_ids: Vec<String> = refs_list
        .iter()
        .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
        .collect();
    legacy_ids.sort();
    for id in &legacy_ids {
        match plan_thread(git, id) {
            Ok(tp) => plan.threads.push(tp),
            Err(e) => eprintln!("warning: skipping {id}: {e}"),
        }
    }

    // Policy diagnostics (warnings + planned rewrites).
    let policy_path = paths.dot_forum.join("policy.toml");
    if policy_path.exists() {
        let text = std::fs::read_to_string(&policy_path)
            .map_err(|e| ForumError::Config(format!("cannot read policy.toml: {e}")))?;
        plan.policy_warnings = scan_at_least_one_summary(&policy_path, &text);
        plan.policy_rewrites = plan_policy_rewrites(&text);
    }

    // Subcommand-grouping scan.
    plan.script_warnings = scan_subcommand_groupings(&paths.dot_forum)?;

    Ok(plan)
}

fn print_plan_header(plan: &MigrationPlan, dry_run: bool) {
    let prefix = if dry_run { "[DRY-RUN] " } else { "" };
    println!(
        "{prefix}git forum migrate: {} thread(s) considered, {} policy diagnostic(s), \
         {} script warning(s)",
        plan.threads.len(),
        plan.policy_warnings.len(),
        plan.script_warnings.len(),
    );
}

fn print_summary(outcome: &MigrationOutcome, dry_run: bool) {
    let verb = if dry_run { "would migrate" } else { "migrated" };
    println!(
        "Summary: {verb} {} thread(s); {} skipped; {} policy rewrite(s); \
         {} policy warning(s); {} script warning(s).",
        outcome.threads_migrated,
        outcome.threads_skipped,
        outcome.policy_rewrites.len(),
        outcome.policy_warnings.len(),
        outcome.script_warnings.len(),
    );
}

// ---------- Policy auto-rewrite ----------

/// SPEC-2.0 §10.1: lines mentioning the removed `at_least_one_summary`
/// predicate produce a warning naming the source file + line number.
fn scan_at_least_one_summary(path: &Path, text: &str) -> Vec<String> {
    let display = path.display();
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.contains("at_least_one_summary"))
        .map(|(i, _)| {
            format!(
                "{display}:{lineno}: predicate `at_least_one_summary` is removed (ADR-006); \
                 it no longer fires. Either delete the predicate or require a non-empty \
                 `body_sections` entry on the relevant `creation_rules`.",
                lineno = i + 1,
            )
        })
        .collect()
}

/// Predict the policy-key rewrites the migrator would perform without
/// writing. Each entry names the file + line of the legacy key.
fn plan_policy_rewrites(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let lineno = i + 1;
        if let Some(key) = legacy_creation_rule_key(line) {
            let target = legacy_creation_rule_target(&key);
            out.push(format!(
                "policy.toml:{lineno}: creation_rules.{key} → {target}"
            ));
        }
        if let Some(scope) = legacy_guard_scope(line) {
            let lifecycle = match scope.as_str() {
                "rfc" => "proposal",
                "dec" => "record",
                _ => "execution",
            };
            out.push(format!(
                "policy.toml:{lineno}: [[guards]] on = \"{scope}:...\" → \
                 \"lifecycle={lifecycle} : ...\""
            ));
        }
    }
    out
}

/// Detect a `[creation_rules.<kind>]` header and return `<kind>` if it is one
/// of the four legacy kinds. Returns `None` otherwise.
fn legacy_creation_rule_key(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    let key = body.strip_prefix("creation_rules.")?.trim();
    // Accept only bare kind keys; skip nested overlays like "rfc.tag.foo".
    if key.contains('.') {
        return None;
    }
    matches!(key, "rfc" | "issue" | "dec" | "task").then(|| key.to_string())
}

/// Detect a guard `on = "<kind>:from->to"` line (1.x scoped form). Returns
/// `<kind>` if matched.
fn legacy_guard_scope(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("on")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.split('"').next()?;
    let scope = value.split(':').next()?.trim();
    matches!(scope, "rfc" | "issue" | "dec" | "task").then(|| scope.to_string())
}

fn legacy_creation_rule_target(kind: &str) -> String {
    match kind {
        "rfc" => "creation_rules.proposal.tag.cross-cutting".into(),
        "issue" => "creation_rules.execution.tag.bug".into(),
        "task" => "creation_rules.execution.tag.task".into(),
        "dec" => "creation_rules.record".into(),
        other => format!("creation_rules.{other}"),
    }
}

/// Perform the policy.toml rewrite in place. Returns one log entry per
/// rewrite (including the file:line of the source key).
fn rewrite_policy_file(paths: &RepoPaths) -> ForumResult<Vec<String>> {
    let policy_path = paths.dot_forum.join("policy.toml");
    if !policy_path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&policy_path)
        .map_err(|e| ForumError::Config(format!("cannot read policy.toml: {e}")))?;
    let (rewritten, log) = rewrite_policy_text(&text);
    if rewritten == text {
        return Ok(log);
    }
    std::fs::write(&policy_path, rewritten)
        .map_err(|e| ForumError::Config(format!("cannot write policy.toml: {e}")))?;
    Ok(log)
}

/// Pure text rewrite of the legacy policy keys. Lines outside the affected
/// constructs pass through unchanged; comments are preserved.
pub fn rewrite_policy_text(input: &str) -> (String, Vec<String>) {
    let mut out_lines = Vec::with_capacity(input.lines().count());
    let mut log = Vec::new();
    let mut current_section: Option<String> = None;
    for (i, line) in input.lines().enumerate() {
        let lineno = i + 1;
        // Track section headers so we can rewrite `body_sections = ...` etc.
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(trimmed[1..trimmed.len() - 1].to_string());
            if let Some(key) = legacy_creation_rule_key(line) {
                let (header, target_label) = match key.as_str() {
                    "rfc" => (
                        "[creation_rules.proposal.tag.cross-cutting]".to_string(),
                        legacy_creation_rule_target("rfc"),
                    ),
                    "issue" => (
                        "[creation_rules.execution.tag.bug]".to_string(),
                        legacy_creation_rule_target("issue"),
                    ),
                    "task" => (
                        "[creation_rules.execution.tag.task]".to_string(),
                        legacy_creation_rule_target("task"),
                    ),
                    "dec" => (
                        "[creation_rules.record]".to_string(),
                        legacy_creation_rule_target("dec"),
                    ),
                    _ => unreachable!(),
                };
                out_lines.push(header.clone());
                log.push(format!(
                    "policy.toml:{lineno}: creation_rules.{key} → {target_label}"
                ));
                current_section = Some(header[1..header.len() - 1].to_string());
                continue;
            }
        }

        if let Some(scope) = legacy_guard_scope(line) {
            let lifecycle = match scope.as_str() {
                "rfc" => "proposal",
                "dec" => "record",
                _ => "execution",
            };
            // Rewrite `on = "<scope>:from->to"` → `on = "lifecycle=<lifecycle> : from->to"`.
            // Preserve original indentation and any trailing characters.
            let needle_quoted = format!("\"{scope}:");
            let replacement = format!("\"lifecycle={lifecycle} : ");
            let new_line = line.replacen(&needle_quoted, &replacement, 1);
            out_lines.push(new_line);
            log.push(format!(
                "policy.toml:{lineno}: [[guards]] on = \"{scope}:...\" → \
                 \"lifecycle={lifecycle} : ...\""
            ));
            continue;
        }
        out_lines.push(line.to_string());
    }
    let _ = current_section;
    let mut joined = out_lines.join("\n");
    if input.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    (joined, log)
}

// ---------- Script scan (RFC-nm3d31yk Q1) ----------

/// Recursively scan `.forum/` for shipped helper scripts/READMEs that
/// invoke the kind-prefixed subcommand groupings. Each line of each match
/// becomes a separate warning with file:line context.
pub fn scan_subcommand_groupings(root: &Path) -> ForumResult<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    stack.push(path);
                    continue;
                }
            }
            if !is_scannable_file(&path) {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            for (idx, line) in text.lines().enumerate() {
                if let Some(form) = match_kind_subcommand(line) {
                    out.push(format!(
                        "{}:{}: legacy kind-prefixed subcommand `{form}` — \
                         use the top-level form (`git forum {} <kind>` etc.). \
                         Removed in 2.0 per SPEC-2.0 §10.2 / RFC-nm3d31yk Q1.",
                        path.display(),
                        idx + 1,
                        top_level_equivalent(&form),
                    ));
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

fn is_scannable_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == "policy.toml" || name == "actors.toml" {
        return false;
    }
    // Heuristic: scan textual files only.
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("sh") | Some("bash") | Some("zsh") | Some("md") | Some("txt") | Some("toml")
    ) || path.extension().is_none()
}

/// Match the `git forum <kind> <verb>` patterns. Returns the matched
/// substring (without the leading `git forum `) or `None`.
fn match_kind_subcommand(line: &str) -> Option<String> {
    const KINDS: &[&str] = &["rfc", "issue", "ask", "dec", "task", "job"];
    const VERBS: &[&str] = &[
        "new",
        "ls",
        "list",
        "close",
        "accept",
        "reject",
        "withdraw",
        "deprecate",
        "propose",
        "pend",
        "show",
        "log",
        "state",
        "comment",
        "claim",
        "question",
        "summary",
        "review",
        "risk",
        "objection",
        "action",
        "alternative",
        "assumption",
        "evidence",
        "verify",
        "diff",
        "status",
        "node",
        "link",
    ];
    // Tokenize on whitespace; tolerate quotes / leading characters.
    let trimmed = line.trim();
    // Find the literal `git forum`.
    let idx = trimmed.find("git forum ")?;
    let rest = &trimmed[idx + "git forum ".len()..];
    let mut tokens = rest.split_whitespace();
    let first = tokens.next()?;
    let second = tokens.next()?;
    let kind_lc = first.to_ascii_lowercase();
    if !KINDS.contains(&kind_lc.as_str()) {
        return None;
    }
    let verb_lc = second.to_ascii_lowercase();
    if !VERBS.contains(&verb_lc.as_str()) {
        return None;
    }
    Some(format!("git forum {first} {second}"))
}

fn top_level_equivalent(form: &str) -> String {
    // The migration warning suggests the canonical command shape; the user
    // chooses the exact rewrite. We surface the verb so the suggestion lines
    // up with what they typed.
    let verb = form.split_whitespace().nth(3).unwrap_or("...");
    verb.to_string()
}

// ---------- Public surface ----------

/// SPEC-2.0 §10.1: the §2.3.3 conventional-tag mapping table, exposed for
/// other tooling and tests.
pub fn lifecycle_for_kind(kind: ThreadKind) -> Lifecycle {
    kind.lifecycle()
}

/// Map a list of legacy IDs to their post-migration bare tokens. Useful for
/// diagnostics and external tooling that wants to predict what `migrate`
/// would do without invoking it.
pub fn predicted_token_map(ids: &[String]) -> BTreeMap<String, String> {
    ids.iter()
        .map(|id| (id.clone(), bare_token_for(id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::event::NodeType;
    use super::*;

    #[test]
    fn bare_token_strips_opaque_prefix() {
        assert_eq!(bare_token_for("RFC-a7f3b2x1"), "a7f3b2x1");
        assert_eq!(bare_token_for("ASK-q3kfj49v"), "q3kfj49v");
        assert_eq!(bare_token_for("JOB-x8n2q1d4"), "x8n2q1d4");
    }

    #[test]
    fn bare_token_passthrough() {
        assert_eq!(bare_token_for("a7f3b2x1"), "a7f3b2x1");
    }

    #[test]
    fn bare_token_sequential_is_deterministic_and_legal() {
        let id1 = bare_token_for("RFC-0001");
        let id2 = bare_token_for("RFC-0001");
        assert_eq!(id1, id2, "deterministic for the same input");
        assert!(
            id_alloc::is_bare_token(&id1),
            "must be a valid bare token: {id1}"
        );
        let id3 = bare_token_for("ASK-0001");
        assert_ne!(id1, id3, "different inputs must yield different tokens");
    }

    #[test]
    fn conventional_tags_match_spec() {
        assert_eq!(conventional_tags(ThreadKind::Rfc), vec!["cross-cutting"]);
        assert_eq!(conventional_tags(ThreadKind::Issue), vec!["bug"]);
        assert_eq!(conventional_tags(ThreadKind::Task), vec!["task"]);
        assert!(conventional_tags(ThreadKind::Dec).is_empty());
    }

    #[test]
    fn rewrite_event_canonicalizes_node_type() {
        let mut ev = Event {
            event_type: EventType::Say,
            thread_id: "RFC-0001".into(),
            node_type: Some(NodeType::Question),
            ..Event::default()
        };
        ev.body = Some("hello".into());
        let out = rewrite_event(&ev, ThreadKind::Rfc, "a7f3b2x1");
        assert_eq!(out.thread_id, "a7f3b2x1");
        assert_eq!(out.node_type, Some(NodeType::Comment));
        assert_eq!(out.legacy_subtype.as_deref(), Some("question"));
    }

    #[test]
    fn rewrite_event_preserves_canonical_node_type() {
        let ev = Event {
            event_type: EventType::Say,
            thread_id: "RFC-a7f3b2x1".into(),
            node_type: Some(NodeType::Action),
            body: Some("body".into()),
            ..Event::default()
        };
        let out = rewrite_event(&ev, ThreadKind::Rfc, "a7f3b2x1");
        assert_eq!(out.node_type, Some(NodeType::Action));
        assert!(out.legacy_subtype.is_none());
    }

    #[test]
    fn rewrite_event_normalizes_state() {
        let ev = Event {
            event_type: EventType::State,
            thread_id: "RFC-0001".into(),
            new_state: Some("under-review".into()),
            ..Event::default()
        };
        let out = rewrite_event(&ev, ThreadKind::Rfc, "a7f3b2x1");
        assert_eq!(out.new_state.as_deref(), Some("review"));
    }

    #[test]
    fn rewrite_event_drops_withdrawn_for_execution_kind() {
        let ev = Event {
            event_type: EventType::State,
            thread_id: "ASK-0001".into(),
            new_state: Some("withdrawn".into()),
            ..Event::default()
        };
        let out = rewrite_event(&ev, ThreadKind::Issue, "a7f3b2x1");
        assert_eq!(out.new_state.as_deref(), Some("rejected"));
    }

    #[test]
    fn legacy_creation_rule_key_detects_kinds() {
        assert_eq!(
            legacy_creation_rule_key("[creation_rules.rfc]"),
            Some("rfc".into())
        );
        assert_eq!(
            legacy_creation_rule_key("  [creation_rules.task]  "),
            Some("task".into())
        );
        assert_eq!(legacy_creation_rule_key("[creation_rules.proposal]"), None);
        assert_eq!(legacy_creation_rule_key("[creation_rules.rfc.tag.x]"), None);
        assert_eq!(legacy_creation_rule_key("not a header"), None);
    }

    #[test]
    fn legacy_guard_scope_detects() {
        assert_eq!(
            legacy_guard_scope("on = \"rfc:under-review->accepted\""),
            Some("rfc".into())
        );
        assert_eq!(
            legacy_guard_scope("  on = \"issue:open->closed\"  "),
            Some("issue".into())
        );
        assert_eq!(legacy_guard_scope("on = \"open->closed\""), None);
        assert_eq!(
            legacy_guard_scope("on = \"lifecycle=proposal : review->done\""),
            None
        );
    }

    #[test]
    fn rewrite_policy_text_translates_creation_rules() {
        let input = "\
[creation_rules.rfc]
required_body = true
body_sections = [\"Goal\"]

[creation_rules.task]
required_body = false
";
        let (out, log) = rewrite_policy_text(input);
        assert!(out.contains("[creation_rules.proposal.tag.cross-cutting]"));
        assert!(out.contains("[creation_rules.execution.tag.task]"));
        assert!(!out.contains("[creation_rules.rfc]"));
        assert_eq!(log.len(), 2);
        assert!(log[0].contains("creation_rules.rfc"));
        assert!(log[0].contains("creation_rules.proposal.tag.cross-cutting"));
    }

    #[test]
    fn rewrite_policy_text_translates_guard_scope() {
        let input = "\
[[guards]]
on = \"rfc:under-review->accepted\"
requires = [\"one_human_approval\"]
";
        let (out, log) = rewrite_policy_text(input);
        assert!(
            out.contains("on = \"lifecycle=proposal : under-review->accepted\""),
            "got: {out}"
        );
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("rfc:..."));
    }

    #[test]
    fn rewrite_policy_text_idempotent_on_already_rewritten_input() {
        let input = "\
[creation_rules.proposal.tag.cross-cutting]
required_body = true

[[guards]]
on = \"lifecycle=proposal : review->done\"
requires = []
";
        let (out, log) = rewrite_policy_text(input);
        assert_eq!(out, input);
        assert!(log.is_empty());
    }

    #[test]
    fn scan_at_least_one_summary_names_lines() {
        let path = Path::new(".forum/policy.toml");
        let text = "[a]\nrequires = [\"at_least_one_summary\"]\n";
        let warnings = scan_at_least_one_summary(path, text);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(".forum/policy.toml:2"));
    }

    #[test]
    fn match_kind_subcommand_detects_legacy_form() {
        assert_eq!(
            match_kind_subcommand("git forum rfc new \"My RFC\""),
            Some("git forum rfc new".into())
        );
        assert_eq!(
            match_kind_subcommand("  $ git forum issue close ASK-1"),
            Some("git forum issue close".into())
        );
        // Top-level form is not flagged.
        assert!(match_kind_subcommand("git forum new rfc \"My RFC\"").is_none());
        assert!(match_kind_subcommand("git forum close ASK-1").is_none());
        // Unrelated text.
        assert!(match_kind_subcommand("// comment about rfc workflow").is_none());
    }

    #[test]
    fn predicted_token_map_round_trips() {
        let map =
            predicted_token_map(&["RFC-0001".into(), "RFC-a7f3b2x1".into(), "a7f3b2x1".into()]);
        assert_eq!(
            map.get("RFC-a7f3b2x1").map(String::as_str),
            Some("a7f3b2x1")
        );
        assert_eq!(map.get("a7f3b2x1").map(String::as_str), Some("a7f3b2x1"));
        let derived = map.get("RFC-0001").unwrap();
        assert!(id_alloc::is_bare_token(derived));
    }
}
