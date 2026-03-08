use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType};
use super::git_ops::GitOps;
use super::refs;
use super::run::{Run, RunStatus};

/// Allocate the next human-readable run label (e.g. `RUN-0001`).
///
/// Preconditions: git is bound to a valid repo.
/// Postconditions: returned label is unique among existing run refs.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: none (read-only).
pub fn alloc_run_label(git: &GitOps) -> ForumResult<String> {
    let all_refs = git.list_refs(refs::RUNS_PREFIX)?;
    let max = all_refs
        .iter()
        .filter_map(|r| {
            let label = r.strip_prefix(refs::RUNS_PREFIX)?;
            let (pfx, num) = label.split_once('-')?;
            if pfx == "RUN" {
                num.parse::<u32>().ok()
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    Ok(format!("RUN-{:04}", max + 1))
}

/// Spawn a new AI run: write `run.json` at `refs/forum/runs/<label>` and
/// emit a `Spawn` event in the thread's event stream.
///
/// Preconditions: git is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a run ref is created and a Spawn event is written to the thread.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates refs.
pub fn spawn_run(
    git: &GitOps,
    thread_id: &str,
    actor_id: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    let now = clock.now();
    let run_label = alloc_run_label(git)?;

    let run = Run {
        run_id: String::new(),
        run_label: run_label.clone(),
        actor_id: actor_id.to_string(),
        thread_id: thread_id.to_string(),
        started_at: now,
        ended_at: None,
        status: RunStatus::Running,
        model: None,
        prompt: None,
        result: None,
        tool_calls: vec![],
    };

    let json = serde_json::to_string_pretty(&run)?;
    let blob_sha = git.hash_object(json.as_bytes())?;
    let tree_sha = git.mktree_single("run.json", &blob_sha)?;
    let ref_name = refs::run_ref(&run_label);
    let message = format!("[git-forum] spawn {run_label} for {thread_id}");
    let commit_sha = git.commit_tree(&tree_sha, &[], &message)?;
    git.update_ref(&ref_name, &commit_sha)?;

    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Spawn,
        created_at: now,
        actor: actor_id.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: Some(run_label.clone()),
        branch: None,
    };
    super::event::write_event(git, &ev)?;

    Ok(run_label)
}

/// Load a single run by label.
///
/// Preconditions: run_label exists as `refs/forum/runs/<run_label>`.
/// Postconditions: returned Run has run_id and run_label populated from Git.
/// Failure modes: ForumError::Repo if not found; ForumError::Git on read failure.
/// Side effects: none.
pub fn read_run(git: &GitOps, run_label: &str) -> ForumResult<Run> {
    let ref_name = refs::run_ref(run_label);
    let commit_sha = git
        .resolve_ref(&ref_name)?
        .ok_or_else(|| ForumError::Repo(format!("run '{run_label}' not found")))?;
    let json = git.show_file(&commit_sha, "run.json")?;
    let mut run: Run = serde_json::from_str(&json)?;
    run.run_id = commit_sha;
    run.run_label = run_label.to_string();
    Ok(run)
}

/// List all runs in label order.
///
/// Preconditions: git is bound to a valid repo.
/// Postconditions: returned Vec is sorted by run label.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: none.
pub fn list_runs(git: &GitOps) -> ForumResult<Vec<Run>> {
    let ref_names = git.list_refs(refs::RUNS_PREFIX)?;
    let mut labels: Vec<String> = ref_names
        .iter()
        .filter_map(|r| r.strip_prefix(refs::RUNS_PREFIX).map(|s| s.to_string()))
        .collect();
    labels.sort();
    let mut runs = Vec::with_capacity(labels.len());
    for label in &labels {
        runs.push(read_run(git, label)?);
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    #[test]
    fn alloc_run_label_format() {
        let formatted = format!("RUN-{:04}", 1u32);
        assert_eq!(formatted, "RUN-0001");
        let formatted2 = format!("RUN-{:04}", 42u32);
        assert_eq!(formatted2, "RUN-0042");
    }
}
