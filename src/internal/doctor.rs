use super::config::RepoPaths;
use super::error::ForumResult;
use super::git_ops::GitOps;
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

/// Run health checks on a git-forum repository.
pub fn run_doctor(git: &GitOps, paths: &RepoPaths) -> ForumResult<DoctorReport> {
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

    // 7. Replay each thread
    if let Some(ids) = &thread_ids {
        for id in ids {
            let ref_name = refs::thread_ref(id);
            match thread::replay_thread(git, id) {
                Ok(_) => checks.push(ok(&format!("replay {ref_name}"))),
                Err(e) => checks.push(fail(&format!("replay {ref_name}"), &e.to_string())),
            }
        }
    }

    Ok(DoctorReport { checks })
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
