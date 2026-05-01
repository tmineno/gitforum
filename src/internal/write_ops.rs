use chrono::{DateTime, Utc};

use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{self, Event, EventType, NodeType};
use super::git_ops::GitOps;
use super::thread;

/// Add a typed discussion node to a thread.
///
/// Preconditions: `git` is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Say event is written and the thread ref updated.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn say_node(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    reply_to: Option<&str>,
) -> ForumResult<String> {
    say_node_core(
        git, thread_id, node_type, body, actor, clock, reply_to, None,
    )
}

/// Add a typed discussion node with a timestamp override.
///
/// Like `say_node`, but uses the given `created_at` instead of the clock.
/// Intended for import/migration scenarios.
#[allow(clippy::too_many_arguments)]
pub fn say_node_with_timestamp(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    reply_to: Option<&str>,
    created_at: DateTime<Utc>,
) -> ForumResult<String> {
    say_node_core(
        git,
        thread_id,
        node_type,
        body,
        actor,
        clock,
        reply_to,
        Some(created_at),
    )
}

#[allow(clippy::too_many_arguments)]
fn say_node_core(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    reply_to: Option<&str>,
    created_at: Option<DateTime<Utc>>,
) -> ForumResult<String> {
    // SPEC-2.0 §2.5 / §9.3 / ADR-006: write the canonical node_type and
    // preserve the user-stated rhetorical label in legacy_subtype.
    let legacy_subtype = node_type.legacy_subtype_label();
    let mut ev = Event::base(thread_id, EventType::Say, actor, clock)
        .with_body(body)
        .with_node_type(node_type.canonical())
        .with_reply_to(reply_to);
    if let Some(label) = legacy_subtype {
        ev = ev.with_legacy_subtype(label);
    }
    if let Some(ts) = created_at {
        ev = ev.with_created_at(ts);
    }
    super::event::write_event(git, &ev)
}

/// Write a `facet_set` event mutating a thread's lifecycle / tag facets.
///
/// SPEC-2.0 §2.4.1 / §7.3:
/// - `lifecycle` is only honored on the thread's first `facet_set`
///   event. A subsequent `facet_set` carrying `lifecycle` is rejected
///   with `FacetTransitionDisallowed`.
/// - Tag values must satisfy §2.3.5 grammar; violations raise
///   `InvalidTagSyntax`.
/// - Empty payloads are valid no-ops.
#[allow(clippy::too_many_arguments)]
pub fn write_facet_set(
    git: &GitOps,
    thread_id: &str,
    lifecycle: Option<&str>,
    tags_add: &[String],
    tags_remove: &[String],
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    // §2.3.5: tag grammar enforced at write time.
    for tag in tags_add.iter().chain(tags_remove.iter()) {
        event::validate_tag(tag).map_err(ForumError::InvalidTagSyntax)?;
    }
    // §7.3 lifecycle is immutable once set: replay the existing chain
    // and reject any new `facet_set` that carries `lifecycle` if one
    // was already established.
    if let Some(new_lifecycle) = lifecycle {
        let state = thread::replay_thread(git, thread_id)?;
        if let Some(existing) = &state.lifecycle {
            if existing != new_lifecycle {
                return Err(ForumError::FacetTransitionDisallowed(format!(
                    "lifecycle is immutable after creation: thread is `{existing}`, \
                     refusing to set to `{new_lifecycle}` (SPEC-2.0 §7.3)"
                )));
            }
            // Same value resubmission — silently ignore, matches replay
            // first-wins semantics (§2.4.1).
        }
    }
    let mut ev = Event::base(thread_id, EventType::FacetSet, actor, clock);
    if let Some(lc) = lifecycle {
        ev = ev.with_lifecycle(lc);
    }
    if !tags_add.is_empty() {
        ev = ev.with_tags_add(tags_add.to_vec());
    }
    if !tags_remove.is_empty() {
        ev = ev.with_tags_remove(tags_remove.to_vec());
    }
    event::write_event(git, &ev)
}

/// Revise the body of a thread, optionally incorporating referenced nodes.
///
/// Preconditions: thread_id exists; all incorporated node IDs must exist in the thread.
/// Postconditions: a ReviseBody event is written with the new body.
/// Failure modes: ForumError::Git on subprocess failure; ForumError::Repo if
///   an incorporated node ID is not found in the thread.
/// Side effects: writes git objects, updates ref.
pub fn revise_body(
    git: &GitOps,
    thread_id: &str,
    body: &str,
    incorporates: &[String],
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let resolved_ids: Vec<String> = if incorporates.is_empty() {
        vec![]
    } else {
        incorporates
            .iter()
            .map(|id| thread::resolve_node_id_in_thread(git, thread_id, id))
            .collect::<Result<Vec<_>, _>>()?
    };
    let ev = Event::base(thread_id, EventType::ReviseBody, actor, clock)
        .with_body(body)
        .with_incorporated_node_ids(resolved_ids);
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Revise the body of an existing node.
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: an Edit event is written with the new body.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn revise_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let ev = Event::base(thread_id, EventType::Edit, actor, clock)
        .with_body(body)
        .with_target_node_id(node_id);
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Change the type of an existing node.
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: a Retype event is written with the new node type.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn retype_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    new_type: NodeType,
    old_type: NodeType,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    // SPEC-2.0 §2.5: persist the canonical type; preserve the user's stated
    // rhetorical label as legacy_subtype.
    let legacy_subtype = new_type.legacy_subtype_label();
    let mut ev = Event::base(thread_id, EventType::Retype, actor, clock)
        .with_target_node_id(node_id)
        .with_node_type(new_type.canonical())
        .with_old_node_type(old_type);
    if let Some(label) = legacy_subtype {
        ev = ev.with_legacy_subtype(label);
    }
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Apply a lifecycle event (Retract, Resolve, or Reopen) to a node.
///
/// Preconditions: thread_id and node_id exist; event_type is Retract, Resolve, or Reopen.
/// Postconditions: the corresponding event is written.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn node_lifecycle(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
    event_type: EventType,
) -> ForumResult<()> {
    let ev = Event::base(thread_id, event_type, actor, clock).with_target_node_id(node_id);
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Retract a node (soft-delete: marks retracted in replay).
pub fn retract_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Retract)
}

/// Resolve a node (marks it addressed, e.g. an objection that has been answered).
pub fn resolve_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Resolve)
}

/// Reopen a resolved or retracted node.
pub fn reopen_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Reopen)
}

#[cfg(test)]
mod facet_set_tests {
    use super::*;
    use chrono::TimeZone;
    use chrono::Utc;

    fn fixed_clock() -> super::super::clock::FixedClock {
        super::super::clock::FixedClock {
            instant: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn setup() -> (
        crate::internal::config::RepoPaths,
        super::super::git_ops::GitOps,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        // Clear inherited GIT_* env vars so this `git init` actually creates
        // a repo at `dir.path()` instead of inheriting the parent's GIT_DIR
        // (e.g. when these tests run inside a pre-commit hook). Mirrors
        // tests/support/repo.rs::TestRepo.
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            // Strip inherited GIT_* so `git init` lands in `dir.path()`,
            // not the parent worktree's git dir (e.g. when invoked from
            // a pre-commit hook context).
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .output()
            .unwrap();
        let paths = crate::internal::config::RepoPaths::from_repo_root(dir.path());
        crate::internal::init::init_forum(&paths).unwrap();
        let git = super::super::git_ops::GitOps::new(dir.path().to_path_buf());
        (paths, git, dir)
    }

    fn make_rfc(git: &super::super::git_ops::GitOps) -> String {
        crate::internal::create::create_thread(
            git,
            super::super::event::ThreadKind::Rfc,
            "Test",
            None,
            "human/alice",
            &fixed_clock(),
        )
        .unwrap()
    }

    #[test]
    fn write_facet_set_rejects_invalid_tag_syntax() {
        let (_paths, git, _dir) = setup();
        let id = make_rfc(&git);
        // Uppercase character — §2.3.5 violation.
        let err = write_facet_set(
            &git,
            &id,
            None,
            &["BadTag".into()],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap_err();
        assert!(
            matches!(err, ForumError::InvalidTagSyntax(_)),
            "expected InvalidTagSyntax, got {err:?}"
        );
    }

    #[test]
    fn write_facet_set_rejects_reserved_tag() {
        let (_paths, git, _dir) = setup();
        let id = make_rfc(&git);
        let err = write_facet_set(
            &git,
            &id,
            None,
            &["untagged".into()],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap_err();
        assert!(matches!(err, ForumError::InvalidTagSyntax(_)));
    }

    #[test]
    fn write_facet_set_rejects_lifecycle_mutation() {
        let (_paths, git, _dir) = setup();
        let id = make_rfc(&git);
        write_facet_set(
            &git,
            &id,
            Some("proposal"),
            &[],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap();
        let err = write_facet_set(
            &git,
            &id,
            Some("execution"),
            &[],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap_err();
        assert!(
            matches!(err, ForumError::FacetTransitionDisallowed(_)),
            "expected FacetTransitionDisallowed, got {err:?}"
        );
    }

    #[test]
    fn write_facet_set_allows_repeated_same_lifecycle() {
        let (_paths, git, _dir) = setup();
        let id = make_rfc(&git);
        write_facet_set(
            &git,
            &id,
            Some("proposal"),
            &[],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap();
        // Re-submitting the same lifecycle is a no-op (§2.4.1 first-wins).
        write_facet_set(
            &git,
            &id,
            Some("proposal"),
            &[],
            &[],
            "human/alice",
            &fixed_clock(),
        )
        .unwrap();
    }
}
