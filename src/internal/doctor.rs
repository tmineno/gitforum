use super::config::RepoPaths;
use super::error::ForumResult;
use super::git_ops::GitOps;
use super::refs;
use super::thread;

pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

pub struct DoctorCheck {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
}

impl DoctorReport {
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
}

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

    // 3. .git/forum/ directory
    checks.push(check_dir(".git/forum/ directory", &paths.git_forum));

    // 4. Thread refs (informational)
    match thread::list_thread_ids(git) {
        Ok(ids) => checks.push(ok(&format!("thread refs: {} found", ids.len()))),
        Err(e) => checks.push(fail("thread refs", &e.to_string())),
    }

    // 5. Replay each thread
    if let Ok(ids) = thread::list_thread_ids(git) {
        for id in &ids {
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
        passed: true,
        detail: None,
    }
}

fn fail(name: &str, detail: &str) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        passed: false,
        detail: Some(detail.to_string()),
    }
}
