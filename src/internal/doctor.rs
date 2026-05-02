use super::config::RepoPaths;
use super::error::ForumResult;
use super::event;
use super::git_ops::GitOps;
use super::hook;
use super::index;
use super::init;
use super::refs;
use super::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Ok,
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
/// Each [`validate::StrictReplayIssue`](super::validate::StrictReplayIssue)
/// becomes its own `strict-replay <ref>` FAIL check, so triage knows exactly
/// which event broke which invariant. Intended for migration verification,
/// CI gates, and audit runs — default `run_doctor` stays lenient so existing
/// repos with historical write-side mistakes don't go red on every run.
pub fn run_doctor_strict(git: &GitOps, paths: &RepoPaths) -> ForumResult<DoctorReport> {
    run_with_mode(git, paths, true)
}

fn run_with_mode(git: &GitOps, paths: &RepoPaths, strict: bool) -> ForumResult<DoctorReport> {
    let mut checks = Vec::new();

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

    // 4. .git/forum/ directory
    checks.push(check_dir(".git/forum/ directory", &paths.git_forum));

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

    // 6. SQLite index health
    let db_path = paths.git_forum.join("index.db");
    if db_path.is_file() {
        match index::open_db(&db_path) {
            Ok(conn) => {
                // integrity check
                match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
                    Ok(result) if result == "ok" => {
                        checks.push(ok("index integrity"));
                    }
                    Ok(result) => {
                        checks.push(fail("index integrity", &result));
                    }
                    Err(e) => {
                        checks.push(fail("index integrity", &e.to_string()));
                    }
                }

                // staleness check: compare thread counts
                match index::list_threads(&conn) {
                    Ok(rows) => {
                        let index_count = rows.len();
                        if index_count == thread_count {
                            checks.push(ok(&format!(
                                "index freshness: {index_count} threads indexed"
                            )));
                        } else {
                            checks.push(warn(
                                "index freshness",
                                &format!(
                                    "index has {index_count} threads but {thread_count} refs exist; run `git forum reindex`"
                                ),
                            ));
                        }
                    }
                    Err(e) => {
                        checks.push(fail("index freshness", &e.to_string()));
                    }
                }
            }
            Err(e) => {
                checks.push(fail("index integrity", &e.to_string()));
            }
        }
    } else {
        checks.push(warn(
            "index database",
            "not found; run `git forum reindex` to create",
        ));
    }

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
                Err(e) => match event::is_orphan_ref(git, id) {
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
        if parent.status != event::ThreadStatus::Done {
            continue;
        }
        let children = &implements_children_by_parent[parent_id];
        let open_children: Vec<&&thread::ThreadState> = children
            .iter()
            .filter(|c| c.status != event::ThreadStatus::Done)
            .collect();
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

fn check_dir(name: &str, path: &std::path::Path) -> DoctorCheck {
    if path.is_dir() {
        ok(name)
    } else {
        fail(name, "directory not found")
    }
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

fn fail(name: &str, detail: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        level: CheckLevel::Fail,
        detail: Some(detail.to_string()),
    }
}
