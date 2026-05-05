//! `git forum new <preset>` and `git forum thread new --lifecycle ...`
//! orchestration.
//!
//! Owns the clap subcommand enum `ThreadCmd` (moved from `main.rs` by
//! task `t8o3vnt6`), the kind-preset / lifecycle helpers (relocated
//! from `main.rs` and `commands/shared.rs` by RFC `7ymtc4b2` Phase 2
//! slot 1), and the snapshot-write entry point.
//!
//! Phase 2 slot 1: write path emits a SPEC-3.0 snapshot tree
//! (`thread.toml` + optional `body.md` / `nodes/` / `links.toml` /
//! `evidence.toml`) at `refs/forum/threads/<id>` directly, via
//! [`crate::internal::snapshot::store::write_snapshot`]. The legacy
//! `internal::create::create_thread_with_branch` event-write path is
//! no longer invoked here.

use std::fs;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use chrono::Utc;
use clap::Subcommand;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::evidence::{EvidenceFile, EvidenceKind, EvidenceRecord};
use crate::internal::git_ops::GitOps;
use crate::internal::id_alloc;
use crate::internal::node::{NodeKind, NodeRecord, NodeStatus};
use crate::internal::policy::{self, CategoryPreset, Lifecycle, Policy};
use crate::internal::snapshot::{
    self, store::write_snapshot, Link, Links, NodeWithBody, ThreadDocument,
};
use crate::internal::thread::ThreadSnapshot;

use crate::internal::operation_check;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};

/// Canonical thread sub-commands per SPEC-2.0 §9.1.
///
/// Power-user / scripting interface keyed on `--lifecycle` and `--tag` rather
/// than the `new <kind>` preset. The kind presets at the top level
/// (`git forum new rfc`, etc.) are the everyday surface; this `thread`
/// namespace exists so scripts can set arbitrary lifecycle/tag combinations
/// without depending on the preset table.
#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum ThreadCmd {
    /// Create a new thread with explicit lifecycle and tag values
    New {
        /// Thread title (omit when using --from-commit)
        #[arg(
            allow_hyphen_values = true,
            required_unless_present_any = ["from_commit", "from_thread"]
        )]
        title: Option<String>,
        /// Lifecycle facet (proposal | execution | record). SPEC-2.0 §2.3.4.
        #[arg(long, value_name = "LIFECYCLE")]
        lifecycle: String,
        /// Tag(s) to attach via the create-time facet_set (may be repeated). SPEC-2.0 §2.3.5.
        #[arg(long, value_name = "TAG")]
        tag: Vec<String>,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long = "from-commit", value_name = "REV")]
        from_commit: Option<String>,
        #[arg(long = "from-thread", value_name = "THREAD_ID")]
        from_thread: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
}

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

/// Args for `commands::thread_new::run` — covers both the `new <preset>`
/// surface (kind preset → lifecycle/tags via the registry) and the
/// canonical `thread new --lifecycle <X> --tag <Y>` form.
pub struct ThreadNewArgs {
    pub title: Option<String>,
    pub body: Option<String>,
    pub body_file: Option<PathBuf>,
    pub edit: bool,
    pub branch: Option<String>,
    pub link_to: Vec<String>,
    pub rel: Option<String>,
    pub as_actor: Option<String>,
    pub from_commit: Option<String>,
    pub from_thread: Option<String>,
    pub inline: ThreadNewInline,
    pub force: bool,
    pub lifecycle: Lifecycle,
    pub tags: Vec<String>,
}

/// Uniform entry point per task `t8o3vnt6` — bundles the existing
/// canonical thread-create call with [`Context`].
pub fn run(args: ThreadNewArgs, ctx: &Context) -> Result<(), ForumError> {
    run_canonical_thread_new(
        args.title,
        args.body,
        args.body_file,
        args.edit,
        args.branch,
        args.link_to,
        args.rel,
        args.as_actor,
        args.from_commit,
        args.from_thread,
        args.inline,
        args.force,
        args.lifecycle,
        args.tags,
        ctx.clock.as_ref(),
    )
}

/// Lifecycle/tag-keyed thread creation, the single dispatch shared by both
/// the `git forum new <preset>` everyday surface and the canonical
/// `git forum thread new --lifecycle ... --tag ...` power-user form.
///
/// Phase 2 slot 1 (RFC `7ymtc4b2`): writes a SPEC-3.0 snapshot tree
/// directly via [`write_snapshot`]. The lifecycle/tag CLI surface
/// stays in place and is mapped to a 3.0 `category` (`rfc`/`task`)
/// internally; the kind-preset surface (`git forum new rfc`) maps the
/// preset name to the same category set.
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
    mut tags: Vec<String>,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);

    let category = lifecycle_to_category(lifecycle).to_string();
    // SPEC-3.0 §8.3: preserve dec/record classification with a
    // canonical `decision` tag when collapsing to the task category.
    augment_tags_for_lifecycle(lifecycle, &mut tags);
    // SPEC-3.0 §3.1: honour `[categories.X] initial_status = ...` overrides
    // from policy.toml so a repo that re-pins the rfc/task initial state
    // (or defines a custom category) gets the configured starting point.
    let registry = policy.effective_registry();
    let initial_status = registry
        .get(&category)
        .map(|d| d.initial_status.clone())
        .unwrap_or_else(|| "open".into());

    let edit_hint = format!("Compose body for new {lifecycle} thread");

    let (effective_title, effective_body, commit_ref, source_thread) = if let Some(ref source_id) =
        from_thread
    {
        let source_id = &resolve_tid(&git, source_id)?;
        // ADR-011 Decision 3: non-migrate paths must NOT consume legacy
        // event chains. Use the snapshot reader so a legacy source
        // surfaces `LegacyEventChain` *before* any write happens — no
        // partial-state risk if the source needs migration.
        let source = snapshot::read_snapshot(&git, source_id)?;
        let source_lifecycle =
            legacy_lifecycle_for_category(&source.snapshot.category, &source.snapshot.tags);
        // SPEC-2.0 §9.3 lifecycle-keyed restatement of 1.x §9.2: an
        // execution thread cannot supersede a proposal. Use
        // `link --rel implements` instead.
        if source_lifecycle == Lifecycle::Proposal && lifecycle == Lifecycle::Execution {
            return Err(ForumError::Config(
                "cannot create an execution thread --from-thread a proposal thread; \
                     an execution thread does not supersede a proposal. \
                     Use `git forum link <NEW> <SOURCE> --rel implements` instead."
                    .into(),
            ));
        }
        let t = title.unwrap_or_else(|| format!("v2: {}", source.snapshot.title));
        let b = resolve_thread_body(body, body_file, edit, &edit_hint)?.or(source.body.clone());
        (t, b, None, Some((source_id.clone(), source_lifecycle)))
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

    // SPEC-3.0 §3.3 creation rules — keyed on the resolved category.
    let violations = operation_check::check_op(
        &policy,
        operation_check::Op::Create {
            category: &category,
            body: effective_body.as_deref(),
        },
    );
    let _ = &effective_title; // Title is not consulted by §3.3 rules.
    let _ = &tags; // 3.0 has no tag-scoped creation rules (§3.1).
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let now = clock.now();
    let timestamp_str = now.to_rfc3339();
    let thread_id = id_alloc::alloc_bare_thread_id(&actor, &effective_title, &timestamp_str);

    // Optional --branch: validate the branch exists before writing.
    if let Some(b) = branch.as_deref() {
        let refname = format!("refs/heads/{b}");
        if git.resolve_ref(&refname)?.is_none() {
            return Err(ForumError::Repo(format!(
                "branch '{b}' does not exist in this repository"
            )));
        }
    }

    let mut nodes: Vec<NodeWithBody> = Vec::new();
    push_inline_nodes(
        &mut nodes,
        &inline.objection,
        NodeKind::Objection,
        None,
        &actor,
        now,
    );
    push_inline_nodes(
        &mut nodes,
        &inline.action,
        NodeKind::Action,
        None,
        &actor,
        now,
    );
    push_inline_nodes(
        &mut nodes,
        &inline.claim,
        NodeKind::Comment,
        Some("claim"),
        &actor,
        now,
    );
    push_inline_nodes(
        &mut nodes,
        &inline.question,
        NodeKind::Comment,
        Some("question"),
        &actor,
        now,
    );
    push_inline_nodes(
        &mut nodes,
        &inline.risk,
        NodeKind::Comment,
        Some("risk"),
        &actor,
        now,
    );
    push_inline_nodes(
        &mut nodes,
        &inline.summary,
        NodeKind::Comment,
        Some("summary"),
        &actor,
        now,
    );

    let mut links_entries: Vec<Link> = Vec::new();
    if !link_to.is_empty() {
        let rel = rel
            .as_deref()
            .ok_or_else(|| ForumError::Config("--rel is required when --link-to is used".into()))?;
        for target in &link_to {
            let resolved_target = resolve_tid(&git, target)?;
            links_entries.push(Link {
                target: resolved_target,
                rel: rel.into(),
                created_at: now,
                created_by: actor.clone(),
            });
        }
    }
    if let Some((ref source_id, _)) = source_thread {
        links_entries.push(Link {
            target: source_id.clone(),
            rel: "supersedes".into(),
            created_at: now,
            created_by: actor.clone(),
        });
    }

    let mut evidence_entries: Vec<EvidenceRecord> = Vec::new();
    if let Some(sha) = commit_ref.clone() {
        evidence_entries.push(EvidenceRecord {
            id: short_evidence_id(&sha),
            kind: EvidenceKind::Commit,
            ref_target: sha,
            created_at: now,
            created_by: actor.clone(),
        });
    }

    let snapshot_doc = ThreadDocument {
        snapshot: ThreadSnapshot {
            schema_version: ThreadSnapshot::SCHEMA_VERSION,
            id: thread_id.clone(),
            title: effective_title.clone(),
            category: category.clone(),
            status: initial_status.clone(),
            tags: tags.clone(),
            created_at: now,
            created_by: actor.clone(),
            updated_at: now,
            updated_by: actor.clone(),
            branch: branch.clone(),
            supersedes: source_thread
                .as_ref()
                .map(|(sid, _)| vec![sid.clone()])
                .unwrap_or_default(),
        },
        body: effective_body,
        nodes,
        links: Links {
            entries: links_entries,
        },
        evidence: EvidenceFile {
            entries: evidence_entries,
        },
    };

    write_snapshot(
        &git,
        &thread_id,
        &snapshot_doc,
        &format!("create {thread_id}"),
    )?;

    // Bidirectional supersede link on the SOURCE side. Only proposal-
    // supersede needs the auto-deprecate; for now we surface the link
    // and leave deprecation to the operator (slot 3 will rewire
    // state-change to snapshot writes; until then, calling the
    // event-write `state_change::*` from the snapshot path would
    // break the cli regression tests by mixing storage shapes on
    // the source ref).
    if let Some((source_id, _source_lifecycle)) = source_thread.as_ref() {
        write_back_supersede_link(&git, source_id, &thread_id, &actor, now)?;
        println!("Created {thread_id} (supersedes {source_id})");
        eprintln!("hint: consider closing {source_id} (now superseded)");
    } else {
        println!("Created {thread_id}");
    }

    // Echo a line for each inline node so the existing CLI output
    // shape (a `Created …` line followed by `Added …` lines) stays
    // stable across the cutover.
    for n in &snapshot_doc.nodes {
        let label = n
            .record
            .legacy_label
            .as_deref()
            .unwrap_or(node_kind_label(n.record.kind));
        println!("Added {label} {}", short_node_id(&n.record.id));
    }

    Ok(())
}

/// Append the symmetric `superseded-by` edge to the SOURCE thread.
///
/// The source has already been pre-flighted via `snapshot::read_snapshot`
/// in [`run_canonical_thread_new`], so a legacy event chain on the
/// source surfaces `LegacyEventChain` *before* any write. By the time
/// we reach this fn, the source is known to be a SPEC-3.0 snapshot.
fn write_back_supersede_link(
    git: &GitOps,
    source_id: &str,
    new_thread_id: &str,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), ForumError> {
    let mut source = snapshot::read_snapshot(git, source_id)?;
    source.links.entries.push(Link {
        target: new_thread_id.into(),
        rel: "superseded-by".into(),
        created_at: now,
        created_by: actor.into(),
    });
    source.snapshot.updated_at = now;
    source.snapshot.updated_by = actor.into();
    write_snapshot(
        git,
        source_id,
        &source,
        &format!("link superseded-by {new_thread_id}"),
    )?;
    Ok(())
}

fn push_inline_nodes(
    out: &mut Vec<NodeWithBody>,
    bodies: &[String],
    kind: NodeKind,
    legacy_label: Option<&'static str>,
    actor: &str,
    now: chrono::DateTime<Utc>,
) {
    for body in bodies {
        let id = id_alloc::alloc_bare_thread_id(actor, body, &now.to_rfc3339());
        out.push(NodeWithBody {
            record: NodeRecord {
                id,
                kind,
                status: NodeStatus::Open,
                created_at: now,
                created_by: actor.into(),
                updated_at: None,
                updated_by: None,
                reply_to: None,
                legacy_label: legacy_label.map(String::from),
            },
            body: body.clone(),
        });
    }
}

fn node_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Comment => "comment",
        NodeKind::Approval => "approval",
        NodeKind::Objection => "objection",
        NodeKind::Action => "action",
    }
}

fn short_node_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn short_evidence_id(sha: &str) -> String {
    format!("c-{}", sha.chars().take(8).collect::<String>())
}

/// Map a v2-shape [`Lifecycle`] onto a SPEC-3.0 built-in category.
///
/// Per SPEC-3.0 §8.3 (category mapping table):
/// - `Proposal` → `rfc` (drafted-then-accepted)
/// - `Execution` → `task` (open-then-done)
/// - `Record` → `task` (decisions ride the task category; the
///   classification is preserved via the canonical `decision` tag,
///   not via a separate category).
pub fn lifecycle_to_category(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Proposal => "rfc",
        Lifecycle::Execution | Lifecycle::Record => "task",
    }
}

/// SPEC-3.0 §8.3 canonical-tag augmentation. Adds the
/// classification tags that the category collapse would otherwise
/// erase, without overriding tags the caller has already set.
///
/// - `Lifecycle::Record` → ensures `decision` is in `tags`
///
/// `Lifecycle::Execution` is split further by the kind preset
/// (`issue`/`bug` get tag `bug` from the preset row, `task`/`job`
/// get `task`); no augmentation needed here.
pub fn augment_tags_for_lifecycle(lifecycle: Lifecycle, tags: &mut Vec<String>) {
    let extra: Option<&'static str> = match lifecycle {
        Lifecycle::Record => Some("decision"),
        Lifecycle::Proposal | Lifecycle::Execution => None,
    };
    if let Some(t) = extra {
        if !tags.iter().any(|x| x == t) {
            tags.push(t.into());
        }
    }
}

/// Inverse of [`lifecycle_to_category`] for the few read paths that
/// still need a `Lifecycle` value (e.g. the supersede-direction guard
/// in `--from-thread`).
///
/// Per SPEC-3.0 §8.3 the `task` category covers both Execution and
/// Record; the `decision` tag is what distinguishes the two on read.
pub fn legacy_lifecycle_for_category(category: &str, tags: &[String]) -> Lifecycle {
    match category {
        "task" => {
            if tags.iter().any(|t| t == "decision") {
                Lifecycle::Record
            } else {
                Lifecycle::Execution
            }
        }
        _ => Lifecycle::Proposal,
    }
}

/// Parse a kind preset name into the corresponding 3.0-native
/// [`CategoryPreset`]. Used by `Commands::New { kind, ... }` to map
/// `git forum new rfc` → preset row → `(category, tags)`.
pub fn preset_lookup(name: &str) -> Option<&'static CategoryPreset> {
    policy::preset_lookup(name)
}

/// Comma-separated list of valid preset names for error messages.
pub fn valid_preset_names() -> String {
    policy::presets()
        .iter()
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a lifecycle string (`proposal` / `execution` / `record`) into
/// its enum.
pub fn parse_lifecycle(s: &str) -> Result<Lifecycle, ForumError> {
    Lifecycle::parse(s).ok_or_else(|| {
        ForumError::Config(format!(
            "unknown lifecycle '{s}'; valid: proposal, execution, record"
        ))
    })
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
        return Ok(Some(crate::internal::editor::edit_body(edit_hint)?));
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
