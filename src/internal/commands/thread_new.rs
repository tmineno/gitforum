//! `git forum new <preset>` and `git forum thread new --lifecycle ...`
//! orchestration.
//!
//! Both CLI surfaces collapse onto [`run_canonical_thread_new`]; the kind
//! preset surface (`Commands::New`) resolves a preset row first via
//! [`SPEC.preset_lookup`](crate::internal::workflow::WorkflowSpec::preset_lookup)
//! and feeds the resulting `(lifecycle, tags)` into this function. The
//! canonical surface (`ThreadCmd::New`) parses lifecycle/tags from the
//! command line directly.

use std::fs;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use crate::internal::clock::Clock;
use crate::internal::create;
use crate::internal::editor;
use crate::internal::error::ForumError;
use crate::internal::event::{Lifecycle, NodeType, ThreadKind};
use crate::internal::evidence::{self, EvidenceKind};
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::show;
use crate::internal::state_change;
use crate::internal::thread;
use crate::internal::write_ops;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};

/// Inline-node bodies attached during `git forum new <preset>` (kind preset
/// path only). The canonical `thread new --lifecycle ...` form does not
/// accept inline nodes — scripts compose them with `node add` after.
#[derive(Default)]
pub struct ThreadNewInline {
    pub claim: Vec<String>,
    pub question: Vec<String>,
    pub objection: Vec<String>,
    pub action: Vec<String>,
    pub risk: Vec<String>,
    pub summary: Vec<String>,
}

/// Lifecycle/tag-keyed thread creation, the single dispatch shared by both
/// the `git forum new <preset>` everyday surface and the canonical
/// `git forum thread new --lifecycle ... --tag ...` power-user form.
///
/// Behaviors:
/// - Picks a backing [`ThreadKind`] (proposal→Rfc, execution→Issue,
///   record→Dec). The kind survives only on the `Create` event for legacy
///   reads; the lifecycle/tag set is the canonical 2.0 facet state, applied
///   via a follow-on `facet_set` event.
/// - `--from-thread`: rejects creating a thread in `execution` lifecycle
///   sourced from a `proposal` thread (SPEC-2.0 §9.3 lifecycle restatement
///   of the 1.x §9.2 RFC→issue rule). Use `link --rel implements` instead.
/// - Auto-deprecates the source on proposal→proposal supersede.
#[allow(clippy::too_many_arguments)]
pub fn run_canonical_thread_new(
    title: Option<String>,
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    branch: Option<String>,
    link_to: Vec<String>,
    rel: Option<String>,
    as_actor: Option<String>,
    from_commit: Option<String>,
    from_thread: Option<String>,
    inline: ThreadNewInline,
    force: bool,
    lifecycle: Lifecycle,
    tags: Vec<String>,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);

    let kind = match lifecycle {
        Lifecycle::Proposal => ThreadKind::Rfc,
        Lifecycle::Execution => ThreadKind::Issue,
        Lifecycle::Record => ThreadKind::Dec,
    };
    let edit_hint = format!("Compose body for new {lifecycle} thread");

    let (effective_title, effective_body, commit_ref, source_thread) = if let Some(ref source_id) =
        from_thread
    {
        let source_id = &resolve_tid(&git, source_id)?;
        let source = thread::replay_thread(&git, source_id)?;
        // SPEC-2.0 §9.3 lifecycle-keyed restatement of 1.x §9.2: an
        // execution thread cannot supersede a proposal. Use
        // `link --rel implements` instead.
        if source.lifecycle == Lifecycle::Proposal && lifecycle == Lifecycle::Execution {
            return Err(ForumError::Config(
                "cannot create an execution thread --from-thread a proposal thread; \
                     an execution thread does not supersede a proposal. \
                     Use `git forum link <NEW> <SOURCE> --rel implements` instead."
                    .into(),
            ));
        }
        let t = title.unwrap_or_else(|| format!("v2: {}", source.title));
        let b = resolve_thread_body(body, body_file, edit, &edit_hint)?.or(source.body.clone());
        (t, b, None, Some((source_id.clone(), source.lifecycle)))
    } else if let Some(rev) = from_commit {
        let commit_sha = git.resolve_commit(&rev)?;
        let msg = git.run(&["log", "-1", "--format=%B", &commit_sha])?;
        let mut lines = msg.lines();
        let subject = lines.next().unwrap_or("").to_string();
        let body_text: String = lines
            .skip_while(|l| l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let t = title.unwrap_or(subject);
        let b =
            resolve_thread_body(body, body_file, edit, &edit_hint)?.or(if body_text.is_empty() {
                None
            } else {
                Some(body_text)
            });
        (t, b, Some(commit_sha), None)
    } else {
        let t = title.ok_or_else(|| {
            ForumError::Config("title is required (or use --from-commit / --from-thread)".into())
        })?;
        let b = resolve_thread_body(body, body_file, edit, &edit_hint)?;
        (t, b, None, None)
    };

    // §7.2 creation rules — keyed on the real `lifecycle`/`tags` the caller
    // selected, not the backing kind.
    let violations = operation_check::check_op(
        &policy,
        operation_check::Op::Create {
            lifecycle,
            tags: &tags,
            body: effective_body.as_deref(),
        },
    );
    let _ = &effective_title; // Title is not consulted by §7.2 rules.
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let thread_id = create::create_thread_with_branch(
        &git,
        kind,
        &effective_title,
        effective_body.as_deref(),
        branch.as_deref(),
        &actor,
        clock,
    )?;

    // Persist `lifecycle` and the requested tag set as a `facet_set` event so
    // 2.0 native threads carry the canonical facet state in their chain
    // rather than only deriving it from `kind` at replay time. Skipped when
    // there are no tags AND the lifecycle matches the kind's auto-derivation
    // (the replay fallback handles it).
    let needs_facet_set = !tags.is_empty() || lifecycle != kind.lifecycle();
    if needs_facet_set {
        write_ops::write_facet_set(
            &git,
            &thread_id,
            Some(lifecycle.as_str()),
            &tags,
            &[],
            &actor,
            clock,
        )?;
    }

    if !link_to.is_empty() {
        let rel = rel
            .as_deref()
            .ok_or_else(|| ForumError::Config("--rel is required when --link-to is used".into()))?;
        for target in &link_to {
            let resolved_target = resolve_tid(&git, target)?;
            evidence::add_thread_link(&git, &thread_id, &resolved_target, rel, &actor, clock)?;
        }
    }
    if let Some(sha) = commit_ref {
        evidence::add_evidence(
            &git,
            &thread_id,
            EvidenceKind::Commit,
            &sha,
            None,
            &actor,
            clock,
        )?;
    }
    // --from-thread supersede: bidirectional link, plus auto-deprecate the
    // source on proposal→proposal (the 1.x RFC→RFC supersede pattern,
    // restated in lifecycle terms).
    if let Some((source_id, source_lifecycle)) = source_thread {
        evidence::add_thread_link(&git, &thread_id, &source_id, "supersedes", &actor, clock)?;
        evidence::add_thread_link(&git, &source_id, &thread_id, "superseded-by", &actor, clock)?;
        let proposal_supersede =
            source_lifecycle == Lifecycle::Proposal && lifecycle == Lifecycle::Proposal;
        if proposal_supersede {
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            state_change::change_state(
                &git,
                &source_id,
                "deprecated",
                &[],
                &actor,
                clock,
                &policy,
                state_change::StateChangeOptions::default(),
            )?;
        }
        println!("Created {thread_id} (supersedes {source_id})");
        if !proposal_supersede {
            eprintln!("hint: consider closing {source_id} (now superseded)");
        }
    } else {
        println!("Created {thread_id}");
    }

    let inline_nodes: [(NodeType, &[String]); 6] = [
        (NodeType::Claim, &inline.claim),
        (NodeType::Question, &inline.question),
        (NodeType::Objection, &inline.objection),
        (NodeType::Action, &inline.action),
        (NodeType::Risk, &inline.risk),
        (NodeType::Summary, &inline.summary),
    ];
    for (node_type, bodies) in &inline_nodes {
        for body_text in *bodies {
            let node_id =
                write_ops::say_node(&git, &thread_id, *node_type, body_text, &actor, clock, None)?;
            println!("Added {node_type} {}", show::short_oid(&node_id));
        }
    }
    Ok(())
}

/// Resolve a body-source flag triple (`--body` / `--body-file` / `--edit`)
/// to a concrete `Option<String>`. Returns `None` if no source was given;
/// callers that require non-empty bodies wrap with `resolve_body_required`.
pub fn resolve_thread_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    edit_hint: &str,
) -> Result<Option<String>, ForumError> {
    if edit {
        return Ok(Some(editor::edit_body(edit_hint)?));
    }
    match (body, body_file) {
        (Some(body), None) if body == "-" => {
            if std::io::stdin().is_terminal() {
                return Err(ForumError::Config(
                    "--body - requires piped input; use --body <text>, --body-file, or --edit instead".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            if buf.trim().is_empty() {
                return Err(ForumError::Config(
                    "--body - received empty input; provide non-empty content via stdin".into(),
                ));
            }
            Ok(Some(buf))
        }
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) => Ok(Some(fs::read_to_string(path)?)),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => unreachable!("clap enforces body/body-file conflicts"),
    }
}

/// Like [`resolve_thread_body`] but errors out if no body source was given.
/// Used by `say` / `revise` paths where an empty body is meaningless.
pub fn resolve_body_required(
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    edit_hint: &str,
) -> Result<String, ForumError> {
    resolve_thread_body(body, body_file, edit, edit_hint)?
        .ok_or_else(|| ForumError::Config("--body, --body-file, or --edit is required".into()))
}
