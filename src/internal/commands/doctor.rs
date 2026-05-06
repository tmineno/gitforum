//! `git forum doctor` orchestration + report data model.
//!
//! task `1hg98odf`: SPEC-3.0 §11/§12 snapshot
//! integrity checks replace the SQLite-index health block. The
//! strict-replay pass is retained because `replay_thread_strict`
//! is mixed-chain aware and surfaces tail-event corruption on
//! event-chain threads still in the repo. A new check decodes the
//! snapshot tip via `internal::snapshot::read_snapshot` for each
//! thread ref; `LegacyEventChain` results emit a `migrate-required`
//! WARN so the operator knows a task `9635buy0` `git forum migrate` is
//! pending.

use super::super::config::RepoPaths;
use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::init;
use super::super::refs;
use super::super::snapshot;
use super::super::thread;
use super::context::Context;
use super::hook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Ok,
    /// Informational. Never fails the run; passes a passable() check.
    /// Used by RFC `fls856j3` §5 advisories like
    /// `auth-without-published INFO` where the absence of a
    /// published ref is a legitimate state, not a problem.
    Info,
    Warn,
    Fail,
}

pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    /// SPEC-2.0 §B.6 cross-thread advisories — strictly informational
    /// observations like "parent RFC @x9k2 is done but has 1 implementing
    /// child still open". Per CORE-VALUE.md "Advisories", these never affect
    /// `all_passed()` and never gate any operation.
    pub advisories: Vec<String>,
}

pub struct DoctorCheck {
    pub name: String,
    pub level: CheckLevel,
    pub detail: Option<String>,
}

impl DoctorCheck {
    /// Backward-compatible helper: true when level is Ok or Warn.
    pub fn passed(&self) -> bool {
        self.level != CheckLevel::Fail
    }
}

impl DoctorReport {
    /// Returns true when no check has failed (warnings are allowed).
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed())
    }
}

const TEMPLATE_FILES: &[&str] = &["issue.md", "rfc.md", "dec.md", "task.md"];

/// Run health checks on a git-forum repository (lenient mode).
///
/// "Lenient" = the same posture lenient `replay()` takes for read paths:
/// silent-no-op conditions inside event replay are NOT promoted to checks.
/// Use [`run_doctor_strict`] (CLI: `git forum doctor --strict`) for the
/// migration / CI posture that surfaces every such no-op as FAIL.
pub fn run_doctor(git: &GitOps, paths: &RepoPaths) -> ForumResult<DoctorReport> {
    run_with_mode(git, paths, false)
}

/// Run health checks with strict event-replay validation enabled.
///
/// Each [`validate::StrictReplayIssue`](super::super::validate::StrictReplayIssue)
/// becomes its own `strict-replay <ref>` FAIL check, so triage knows exactly
/// which event broke which invariant. Intended for migration verification,
/// CI gates, and audit runs — default `run_doctor` stays lenient so existing
/// repos with historical write-side mistakes don't go red on every run.
pub fn run_doctor_strict(git: &GitOps, paths: &RepoPaths) -> ForumResult<DoctorReport> {
    run_with_mode(git, paths, true)
}

fn run_with_mode(git: &GitOps, paths: &RepoPaths, strict: bool) -> ForumResult<DoctorReport> {
    let mut checks = Vec::new();

    // 0. Fresh-clone detection. A repo that was just cloned from a
    //    git-forum-using upstream looks like this: `.forum/policy.toml`
    //    is tracked (so it's present in the worktree), but `.git/forum/`
    //    has never been created locally and `refs/forum/*` haven't been
    //    fetched yet (refspec defaults don't include them — see
    //    SPEC-3.0 §10). Without this check the user sees a noisy FAIL
    //    on `.git/forum/ directory` and a separate WARN on missing
    //    refspec, with no obvious path forward. Surface a single clear
    //    "run `git forum init`" message and downgrade the .git/forum/
    //    FAIL to WARN so doctor doesn't exit non-zero on a benign
    //    post-clone state.
    let fresh_clone = is_fresh_clone(git, paths);
    if fresh_clone {
        checks.push(warn(
            "fresh clone detected",
            "this repo has .forum/ config but no local forum state yet; run `git forum init` to register the fetch refspec, fetch refs/forum/*, and create .git/forum/",
        ));
    }

    // 1. .forum/ directory
    checks.push(check_dir(".forum/ directory", &paths.dot_forum));

    // 2. .forum/policy.toml exists and parses
    let policy_path = paths.dot_forum.join("policy.toml");
    if policy_path.exists() {
        match std::fs::read_to_string(&policy_path) {
            Ok(content) => match content.parse::<toml::Table>() {
                Ok(_) => checks.push(ok("policy.toml valid")),
                Err(e) => checks.push(fail("policy.toml valid", &e.to_string())),
            },
            Err(e) => checks.push(fail("policy.toml valid", &e.to_string())),
        }
    } else {
        checks.push(fail("policy.toml exists", "file not found"));
    }

    // 3. .forum/templates/ directory and files
    let templates_dir = paths.dot_forum.join("templates");
    checks.push(check_dir(".forum/templates/ directory", &templates_dir));
    if templates_dir.is_dir() {
        for filename in TEMPLATE_FILES {
            let path = templates_dir.join(filename);
            if path.is_file() {
                match std::fs::metadata(&path) {
                    Ok(meta) if meta.len() > 0 => {
                        checks.push(ok(&format!("template {filename}")));
                    }
                    Ok(_) => {
                        checks.push(fail(&format!("template {filename}"), "file is empty"));
                    }
                    Err(e) => {
                        checks.push(fail(&format!("template {filename}"), &e.to_string()));
                    }
                }
            } else {
                checks.push(fail(
                    &format!("template {filename}"),
                    &format!("file not found; create .forum/templates/{filename}"),
                ));
            }
        }
    }

    // 4. .git/forum/ directory. Missing is normally a FAIL, but on a
    //    fresh clone it's the expected state — covered by the top-level
    //    "fresh clone detected" WARN, so don't double-fail here.
    if paths.git_forum.is_dir() {
        checks.push(ok(".git/forum/ directory"));
    } else if fresh_clone {
        checks.push(warn(
            ".git/forum/ directory",
            "not yet created; will be created by `git forum init`",
        ));
    } else {
        checks.push(fail(".git/forum/ directory", "directory not found"));
    }

    // 5. Forum fetch refspec for each remote
    match git.run(&["remote"]) {
        Ok(remotes_output) => {
            let remotes: Vec<&str> = remotes_output
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            if remotes.is_empty() {
                checks.push(ok("forum refspec (no remotes)"));
            } else {
                for remote in &remotes {
                    match init::has_forum_refspec(git, remote) {
                        Ok(true) => {
                            checks.push(ok(&format!("forum refspec ({remote})")));
                        }
                        Ok(false) => {
                            checks.push(warn(
                                &format!("forum refspec ({remote})"),
                                &format!(
                                    "remote '{remote}' lacks refs/forum/* fetch refspec; run `git forum init` to fix"
                                ),
                            ));
                        }
                        Err(e) => {
                            checks.push(warn(
                                &format!("forum refspec ({remote})"),
                                &format!("could not check: {e}"),
                            ));
                        }
                    }
                }
            }
        }
        Err(_) => {
            checks.push(ok("forum refspec (no remotes)"));
        }
    }

    // 6. Thread refs (informational)
    let thread_ids = thread::list_thread_ids(git).ok();
    let thread_count = thread_ids.as_ref().map_or(0, |ids| ids.len());
    match &thread_ids {
        Some(ids) => checks.push(ok(&format!("thread refs: {} found", ids.len()))),
        None => checks.push(fail("thread refs", "could not list refs")),
    }

    let _ = thread_count;
    // 6. Snapshot integrity (SPEC-3.0 §11/§12). For each non-orphan
    //    thread ref, decode `thread.toml` via `snapshot::read_snapshot`.
    //    Threads still on the legacy event chain (no `thread.toml` at
    //    the tip tree) emit a `migrate-required` WARN — they continue
    //    to read correctly via the mixed-chain replay path, but task `9635buy0`
    //    `git forum migrate` will fold them into the snapshot layout.
    //    Orphan refs (no usable thread history at all) are handled
    //    by the replay block below as `orphan ref` WARNs.
    if let Some(ids) = &thread_ids {
        let mut snapshot_ok = 0u32;
        for id in ids {
            if matches!(is_orphan_ref(git, id), Ok(true)) {
                continue; // orphan refs handled in the replay block
            }
            let ref_name = refs::thread_ref(id);
            match snapshot::read_snapshot(git, id) {
                Ok(_doc) => {
                    snapshot_ok += 1;
                }
                Err(ForumError::LegacyEventChain) => {
                    checks.push(warn(
                        &format!("migrate-required {ref_name}"),
                        "thread is on the legacy event chain; run `git forum migrate` (task `9635buy0`)",
                    ));
                }
                Err(e) => {
                    checks.push(fail(&format!("snapshot decode {ref_name}"), &e.to_string()));
                }
            }
        }
        if snapshot_ok > 0 {
            checks.push(ok(&format!(
                "snapshot integrity: {snapshot_ok} thread.toml decoded"
            )));
        }
    }

    // 6b. Published-namespace consistency (RFC fls856j3 §5, §7).
    publish_namespace_checks(git, &mut checks);

    // 7. Index blob integrity
    match hook::fix_index_blobs(git) {
        Ok(result) => {
            if result.fixed.is_empty() && result.warnings.is_empty() {
                checks.push(ok("index blobs"));
            } else {
                let mut details = Vec::new();
                for (path, sha) in &result.fixed {
                    details.push(format!("re-hashed {path} (was {sha})"));
                }
                for (path, sha) in &result.warnings {
                    details.push(format!("{path} missing blob {sha}, no working-tree copy"));
                }
                if result.warnings.is_empty() {
                    checks.push(warn("index blobs", &details.join("; ")));
                } else {
                    checks.push(fail("index blobs", &details.join("; ")));
                }
            }
        }
        Err(e) => {
            checks.push(warn("index blobs", &format!("could not check: {e}")));
        }
    }

    // 8. Replay each thread (strict). Strict mode surfaces silent no-ops
    //    that lenient `replay()` swallows for read-side compatibility — see
    //    `validate::StrictReplayIssue`. Each issue becomes its own FAIL line
    //    so triage knows exactly which event broke the invariant.
    //
    //    A replay error whose root cause is "the thread ref's bottom commit
    //    is not a valid event.json" is downgraded to a `WARN` orphan-ref
    //    finding with a `prune-orphans` hint, since the ref carries no
    //    recoverable thread and the fix is deletion, not data repair. Genuine
    //    mid-chain corruption keeps `FAIL` semantics.
    let mut replayed_states: Vec<thread::ThreadState> = Vec::new();
    if let Some(ids) = &thread_ids {
        for id in ids {
            let ref_name = refs::thread_ref(id);
            match thread::replay_thread_strict(git, id) {
                Ok((state, issues)) => {
                    checks.push(ok(&format!("replay {ref_name}")));
                    if strict {
                        for issue in issues {
                            checks.push(fail(
                                &format!("strict-replay {ref_name}"),
                                &issue.to_string(),
                            ));
                        }
                    }
                    replayed_states.push(state);
                }
                Err(e) => match is_orphan_ref(git, id) {
                    Ok(true) => checks.push(warn(
                        &format!("orphan ref {ref_name}"),
                        "ref has no usable create event; run `git forum prune-orphans` to delete",
                    )),
                    _ => checks.push(fail(&format!("replay {ref_name}"), &e.to_string())),
                },
            }
        }
    }

    // 9. Cross-thread advisories (SPEC-2.0 §B.6).
    //
    // Strictly informational: never appended to `checks`, never affects exit
    // status. The pattern surfaced is "parent is `done` but has children
    // still in flight" — common cleanup oversight after closing an RFC.
    let advisories = build_cross_thread_advisories(&replayed_states);

    Ok(DoctorReport { checks, advisories })
}

/// `true` when this looks like a freshly cloned repo whose upstream
/// uses git-forum but where `git forum init` hasn't been run yet.
///
/// Signature:
///   - `.forum/policy.toml` is present (came from clone)
///   - `.git/forum/logs/` is missing (no per-clone state yet)
///   - no `refs/forum/threads/*` refs locally
///   - at least one git remote is configured
///
/// All four must hold; otherwise this is some other state (e.g. mid-
/// migration, partially-initialised repo, brand-new project) and
/// doctor's normal per-check output is the right diagnosis.
fn is_fresh_clone(git: &GitOps, paths: &RepoPaths) -> bool {
    if !paths.dot_forum.join("policy.toml").is_file() {
        return false;
    }
    if paths.git_forum.join("logs").is_dir() {
        return false;
    }
    let has_thread_refs = git
        .list_refs("refs/forum/threads/")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if has_thread_refs {
        return false;
    }
    let has_remotes = git
        .run(&["remote"])
        .map(|s| s.lines().any(|l| !l.trim().is_empty()))
        .unwrap_or(false);
    has_remotes
}

/// `true` when the ref's bottom (oldest) commit's tree carries no
/// recognisable git-forum content — neither a SPEC-3.0 `thread.toml`
/// nor a SPEC-2.0 `event.json`. An empty ref (no commits at all) is
/// also reported as orphan.
///
/// 3.0-native replacement for `legacy::event::is_orphan_ref`: this
/// looks at tree shape only and does not parse either codec, so the
/// doctor's orphan probe doesn't reach into `internal::legacy`.
fn is_orphan_ref(git: &GitOps, thread_id: &str) -> ForumResult<bool> {
    let ref_name = refs::thread_ref(thread_id);
    let shas = git.rev_list(&ref_name)?;
    let Some(oldest) = shas.last() else {
        return Ok(true);
    };
    let tree_listing = git.run(&["ls-tree", "-r", "--full-tree", "--name-only", oldest])?;
    let has_recognised = tree_listing
        .lines()
        .any(|p| p == "thread.toml" || p == "event.json");
    Ok(!has_recognised)
}

/// Collect cross-thread advisory lines from already-replayed thread states.
///
/// One observation today: parent threads in terminal `done` state that still
/// have incoming `--rel implements` children whose state isn't `done`. Per
/// CORE-VALUE.md, this is read-only display — doctor does not propose,
/// suggest, or perform any state change in response.
///
/// Link target IDs in legacy events use the `KIND-XXXXXXXX` form; we
/// canonicalize via the migrate mapping (`migrate::bare_token_for`) so that
/// links recorded before the Track C migration still match their parent's
/// canonical bare-token ID in the `by_id` lookup.
fn build_cross_thread_advisories(states: &[thread::ThreadState]) -> Vec<String> {
    use std::collections::HashMap;

    let by_id: HashMap<&str, &thread::ThreadState> =
        states.iter().map(|s| (s.id.as_str(), s)).collect();
    let known_ids: std::collections::HashSet<&str> = by_id.keys().copied().collect();

    let mut implements_children_by_parent: HashMap<String, Vec<&thread::ThreadState>> =
        HashMap::new();
    for s in states {
        for link in &s.links {
            if link.rel != "implements" {
                continue;
            }
            let canonical = canonicalize_target(&link.target_thread_id, &known_ids);
            implements_children_by_parent
                .entry(canonical)
                .or_default()
                .push(s);
        }
    }

    let mut out = Vec::new();
    let mut parent_ids: Vec<String> = implements_children_by_parent.keys().cloned().collect();
    parent_ids.sort();
    for parent_id in &parent_ids {
        let Some(parent) = by_id.get(parent_id.as_str()) else {
            continue; // referenced parent not in this repo's refs — skip silently
        };
        if parent.status != "done" {
            continue;
        }
        let children = &implements_children_by_parent[parent_id];
        let open_children: Vec<&&thread::ThreadState> =
            children.iter().filter(|c| c.status != "done").collect();
        if open_children.is_empty() {
            continue;
        }
        let count = open_children.len();
        let plural = if count == 1 { "child" } else { "children" };
        let ids: Vec<String> = open_children.iter().map(|c| c.id.clone()).collect();
        out.push(format!(
            "{} ({}) has {count} implementing {plural} still open ({})",
            parent.id,
            parent.status,
            ids.join(", ")
        ));
    }
    out
}

fn canonicalize_target(target: &str, known_ids: &std::collections::HashSet<&str>) -> String {
    if known_ids.contains(target) {
        return target.to_string();
    }
    let canonical = super::migrate::bare_token_for(target);
    if canonical != target && known_ids.contains(canonical.as_str()) {
        canonical
    } else {
        target.to_string()
    }
}

// ============================================================
//  Doctor command — `git forum doctor` orchestration
// ============================================================

/// Args for [`run`] — `git forum doctor`.
pub struct DoctorArgs {
    pub verbose: bool,
    pub strict: bool,
}

/// Uniform entry point for the `doctor` subcommand.
///
/// Selects lenient vs strict mode, prints the formatted report, and
/// exits non-zero iff a check failed (warnings and advisories never
/// gate exit per CORE-VALUE.md "Advisories").
pub fn run(args: DoctorArgs, ctx: &Context) -> Result<(), ForumError> {
    let report = if args.strict {
        run_doctor_strict(&ctx.git, &ctx.paths)?
    } else {
        run_doctor(&ctx.git, &ctx.paths)?
    };
    print_report(&report, args.verbose);
    if !report.all_passed() {
        std::process::exit(1);
    }
    Ok(())
}

/// Render the formatted doctor report. Replay checks collapse into a
/// single summary line unless one of them failed; non-replay checks
/// always emit failures and warnings, and emit OK lines only in
/// `verbose` mode.
fn print_report(report: &DoctorReport, verbose: bool) {
    let mut replay_ok = 0u32;
    let mut replay_fail: Vec<&DoctorCheck> = Vec::new();
    let mut ok_count = 0u32;
    let mut warn_count = 0u32;
    let mut fail_count = 0u32;

    for check in &report.checks {
        match check.level {
            CheckLevel::Ok => ok_count += 1,
            CheckLevel::Info => {} // Informational — not counted toward ok/warn/fail tallies.
            CheckLevel::Warn => warn_count += 1,
            CheckLevel::Fail => fail_count += 1,
        }
        let is_replay = check.name.starts_with("replay ");
        if is_replay {
            if check.level == CheckLevel::Ok {
                replay_ok += 1;
                continue;
            } else {
                replay_fail.push(check);
                continue;
            }
        }
        if check.level != CheckLevel::Ok || verbose {
            let marker = match check.level {
                CheckLevel::Ok => " ok ",
                CheckLevel::Info => "INFO",
                CheckLevel::Warn => "WARN",
                CheckLevel::Fail => "FAIL",
            };
            print!("[{marker}] {}", check.name);
            if let Some(detail) = &check.detail {
                print!(" -- {detail}");
            }
            println!();
        }
    }

    let total_replay = replay_ok + replay_fail.len() as u32;
    if total_replay > 0 {
        if replay_fail.is_empty() {
            println!("[ ok ] replay: {replay_ok} threads replayed successfully");
        } else {
            for check in &replay_fail {
                let detail = check.detail.as_deref().unwrap_or("unknown error");
                println!("[FAIL] {} -- {}", check.name, detail);
            }
            if replay_ok > 0 {
                println!("[ ok ] replay: {replay_ok} other threads ok");
            }
        }
    }

    for advisory in &report.advisories {
        println!("[ADV ] {advisory}");
    }

    println!();
    if fail_count == 0 && warn_count == 0 {
        println!("All {ok_count} checks passed.");
    } else {
        let parts: Vec<String> = [
            (fail_count, "failed"),
            (warn_count, "warning"),
            (ok_count, "passed"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect();
        println!("{}", parts.join(", "));
    }
}

fn check_dir(name: &str, path: &std::path::Path) -> DoctorCheck {
    if path.is_dir() {
        ok(name)
    } else {
        fail(name, "directory not found")
    }
}

/// Cross-check the `refs/forum/threads/*` and `refs/forum/published/*`
/// namespaces and append any RFC `fls856j3` §5 / §7 advisories.
///
/// Emitted advisories:
/// - `auth-without-published INFO` — public thread without a
///   published counterpart (likely "you forgot to push", but
///   informational since not-publishing is legitimate).
/// - `visibility-mismatch WARN` — both refs exist but the published
///   tree disagrees with the authoritative visibility (would only
///   happen across writer-version skews).
/// - `stale-published WARN` — published ref exists locally with no
///   authoritative counterpart, or with an authoritative thread
///   marked private. Suggests an interrupted withdrawal — re-run
///   `git forum push` to retry the remote delete (RFC §7).
fn publish_namespace_checks(git: &GitOps, checks: &mut Vec<DoctorCheck>) {
    use std::collections::HashSet;

    let auth_ids: Vec<String> = match git.list_refs(refs::THREADS_PREFIX) {
        Ok(refs) => refs
            .iter()
            .filter_map(|r| refs::thread_id_from_ref(r).map(|s| s.to_string()))
            .collect(),
        Err(_) => Vec::new(),
    };
    let pub_ids: HashSet<String> = match git.list_refs(refs::PUBLISHED_PREFIX) {
        Ok(refs) => refs
            .iter()
            .filter_map(|r| refs::thread_id_from_published_ref(r).map(|s| s.to_string()))
            .collect(),
        Err(_) => HashSet::new(),
    };

    // First pass: walk authoritative threads, classify each.
    let mut auth_set: HashSet<&str> = HashSet::new();
    for id in &auth_ids {
        auth_set.insert(id.as_str());
        let visibility = match snapshot::read_snapshot(git, id) {
            Ok(doc) => doc.snapshot.visibility,
            Err(_) => {
                // Read errors are surfaced by the snapshot-decode
                // check above; don't double-report here.
                continue;
            }
        };
        let has_published = pub_ids.contains(id);
        match (visibility, has_published) {
            (thread::Visibility::Public, false) => {
                checks.push(info(
                    &format!("auth-without-published {id}"),
                    "thread is public but has no refs/forum/published/<id>; run `git forum push` to publish",
                ));
            }
            (thread::Visibility::Private, true) => {
                checks.push(warn(
                    &format!("stale-published {id}"),
                    "thread is private but a published ref still exists locally; re-run `git forum push` to retry the withdrawal",
                ));
            }
            (thread::Visibility::Public, true) => {
                // Published tree should mirror the authoritative
                // visibility ("public"). The visibility-mismatch
                // case fires when the published tree's thread.toml
                // disagrees, which would happen across writer-
                // version skews.
                if let Ok(doc) = read_published_snapshot(git, id) {
                    if doc.snapshot.visibility != thread::Visibility::Public {
                        checks.push(warn(
                            &format!("visibility-mismatch {id}"),
                            "published tree's visibility disagrees with authoritative; re-run `git forum push` to refresh",
                        ));
                    }
                }
            }
            (thread::Visibility::Private, false) => {
                // Normal: private thread, no published counterpart.
            }
        }
    }

    // Second pass: published refs without an authoritative
    // counterpart. On public-consumer clones (RFC §3:
    // `init --public-only`) this is the steady state — orphans
    // are normal because the authoritative namespace is never
    // imported. Detect that case from configured fetch refspecs:
    // a clone is public-consumer iff at least one remote is
    // configured for forum refs *and* none of them fetch the
    // authoritative namespace.
    if is_public_consumer_clone(git) {
        return;
    }
    for pid in &pub_ids {
        if !auth_set.contains(pid.as_str()) {
            checks.push(warn(
                &format!("stale-published {pid}"),
                "published ref exists locally but the authoritative thread is absent; re-run `git forum push` to retry the withdrawal",
            ));
        }
    }
}

/// Detect a public-consumer clone (RFC §3 `init --public-only`).
/// True iff at least one remote is configured for forum refs and
/// none of those forum-related refspecs cover the authoritative
/// namespace.
///
/// Repos with no remotes (e.g. local-only publisher checkouts) are
/// treated as trusted-collaborator: they own the authoritative
/// namespace, so an orphan published ref is anomalous and worth
/// surfacing.
fn is_public_consumer_clone(git: &GitOps) -> bool {
    let remotes = match git.run(&["remote"]) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut saw_published = false;
    let mut saw_authoritative = false;
    for remote in remotes.lines() {
        let remote = remote.trim();
        if remote.is_empty() {
            continue;
        }
        let key = format!("remote.{remote}.fetch");
        let values = match git.run(&["config", "--get-all", &key]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for v in values.lines() {
            let v = v.trim();
            if v == init::FORUM_REFSPEC || v == init::THREADS_REFSPEC {
                saw_authoritative = true;
            }
            if v == init::PUBLISHED_REFSPEC {
                saw_published = true;
            }
        }
    }
    saw_published && !saw_authoritative
}

/// Read a snapshot specifically from the published-namespace ref,
/// bypassing the §5 fallback (which would resolve through to the
/// authoritative ref first). Used by the visibility-mismatch check
/// where we need to inspect the *published* tree.
fn read_published_snapshot(
    git: &GitOps,
    thread_id: &str,
) -> Result<crate::internal::snapshot::ThreadDocument, ForumError> {
    let refname = refs::published_ref(thread_id);
    let tip = git
        .resolve_ref(&refname)?
        .ok_or_else(|| ForumError::SnapshotMissing(format!("{refname} does not exist")))?;
    crate::internal::snapshot::store::read_snapshot_at(git, &tip)
}

fn ok(name: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        level: CheckLevel::Ok,
        detail: None,
    }
}

fn warn(name: &str, detail: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        level: CheckLevel::Warn,
        detail: Some(detail.to_string()),
    }
}

fn info(name: &str, detail: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        level: CheckLevel::Info,
        detail: Some(detail.to_string()),
    }
}

fn fail(name: &str, detail: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        level: CheckLevel::Fail,
        detail: Some(detail.to_string()),
    }
}
