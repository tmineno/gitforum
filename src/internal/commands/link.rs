//! `git forum link <ID> <TARGET_ID> --rel <REL>` orchestration.
//!
//! Phase 2 slot 7k (RFC `7ymtc4b2`): appends rows to `links.toml`
//! directly via `snapshot::store::write_snapshot`. The legacy
//! `internal::evidence::add_thread_link` event-write path is no
//! longer invoked here.

use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::snapshot::{self, store::write_snapshot, Link};

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};
use super::shorthand_say::migrate_legacy_to_snapshot;

pub fn run_link(
    thread_id: &str,
    target_thread_id: &str,
    rel: &str,
    as_actor: Option<String>,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, thread_id)?;
    let target_thread_id = resolve_tid(&git, target_thread_id)?;
    let actor = resolve_actor(as_actor, &git);

    let mut doc = match snapshot::read_snapshot(&git, &thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(&git, &thread_id)?,
        Err(other) => return Err(other),
    };
    let now = clock.now();
    doc.links.entries.push(Link {
        target: target_thread_id.clone(),
        rel: rel.into(),
        created_at: now,
        created_by: actor.clone(),
    });
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();
    write_snapshot(
        &git,
        &thread_id,
        &doc,
        &format!("link {thread_id} -> {target_thread_id} ({rel})"),
    )?;
    println!("{thread_id} -> {target_thread_id} ({rel})");
    Ok(())
}
