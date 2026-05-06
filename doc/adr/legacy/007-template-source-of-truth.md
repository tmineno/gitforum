# task `96u6zxmc`: Template Source of Truth

## Context

git-forum's body templates (`issue.md`, `rfc.md`, `dec.md`, `task.md`) lived in two places at once:

1. **Inline string constants** in `src/internal/init.rs` (`TEMPLATE_ISSUE`, `TEMPLATE_RFC`, `TEMPLATE_DEC`, `TEMPLATE_TASK`).
2. **Committed files** under `.forum/templates/*.md`.

`init_forum` ran `write_if_missing(.forum/templates/<name>.md, TEMPLATE_<KIND>)` from both `git forum init` and the `worktree-init` post-checkout hook. There was no enforcement keeping the two sources in sync, and they had drifted in HEAD: the committed `rfc.md` was a 1-line `# {title}` stub while the constant carried a multi-section `Goal / Non-goals / Context / Proposal` scaffold. Users who looked at the committed file to learn what an RFC body should look like got the misleading stub.

The drift was discovered during the investigation of thread `0edk3jdm` and tracked as a focused follow-up in thread `96u6zxmc`.

## Decision

**Committed files are the single source of truth.** The inline `TEMPLATE_*` constants are removed; `init_forum` embeds the templates at compile time via `include_str!("../../.forum/templates/<name>.md")`. The same physical file backs both git-forum's own forum (the maintainer-facing role of `.forum/templates/`) and the seed shipped to user repos via `git forum init`.

`init_forum` is split into two entry points:

- `init_forum(paths)` — full init, writes both shared `.forum/` content and per-worktree `.git/forum/` content. Used by `git forum init`.
- `init_forum_local(paths)` — per-worktree only, writes `.git/forum/logs` and the local git alias. Used by the `worktree-init` post-checkout hook.

`worktree-init` no longer touches `.forum/`. Tracked template files arrive in a worktree via checkout, never via the hook.

The drifted `rfc.md` is regenerated to match the multi-section scaffold and committed in the same change.

## Consequences

- One source of truth for template content. A `git diff` of `.forum/templates/rfc.md` shows exactly what changed for users.
- `git forum init` on a brand-new repo still produces working templates — the bytes are embedded by `include_str!` at build time, so the binary is self-contained.
- `worktree-init` on an existing repo is now a strict no-op for shared content. Templates either exist via checkout (tracked) or they don't (untracked branch); the hook doesn't synthesize either way.
- A new test asserts that the post-init `rfc.md` contains its scaffold sections (`## Goal`, `## Non-goals`, `## Context`, `## Proposal`), so the drift cannot silently return.
- Doctor's existing template-existence check in `src/internal/doctor.rs:70-95` is unchanged.

## Alternatives

- **(a) Compiled-in constants only; remove `.forum/templates/` from tracking.** Rejected: users who learn git-forum by reading repo source would lose the `.forum/templates/*.md` examples, and `doctor`'s template-existence check would have to be replaced with a constant-validation check. The committed files have community-facing value beyond the binary's seed.
- **Keep both sources, add a build-time equality assertion.** Rejected: solves the drift but keeps two copies. `include_str!` from the committed file gives equality by construction with no extra machinery.
- **Generate templates from a separate `templates/` source dir.** Rejected: duplicates `.forum/templates/`. The dogfood role and the seed role can share one file.

## Exit criteria

- Source code contains no inline template body constants.
- `cargo test` passes including the new `init_creates_non_trivial_rfc_template` test.
- `.forum/templates/rfc.md` matches the embedded content by virtue of `include_str!`.
- thread `96u6zxmc` closed.
