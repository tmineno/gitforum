# ADR-008: fix-index Is Defense-in-Depth, Not a Fix for a Known Cause

## Context

`git-forum hook fix-index` repairs two failure modes:

1. Index entries that reference blobs missing from the local object store (re-hashes the working-tree copy).
2. HEAD-tree entries that reference blobs missing from the local object store (re-stages the working-tree copy so the next commit lands a fresh tree).

Earlier text in `doc/spec/SPEC.md:760` and `doc/MANUAL.md:1102-1105` claimed the cause was "GC in worktrees" and that the recovery exists to "prevent pre-commit framework crashes during the stash/restore cycle." Both claims were investigated under issue `@0edk3jdm` (Phase 1) and found to be unsupported.

## Decision

Document `fix-index` as defense-in-depth recovery, **not** as a known-cause fix. Drop the GC-causation language from the spec and manual. Refer readers to the `@0edk3jdm` Phase 1 investigation for the negative-result evidence rather than re-asserting the discarded explanation.

## Consequences

- Future readers (humans, AI assistants) building on the docs will not start from a discarded causal model.
- If a deterministic reproduction is later discovered, the docs need a positive update at that time. Until then, the absence of a cause is itself the documented state.
- The `fix-index` code path and the `fix_index_repairs_missing_head_tree_blob` test stay as-is. They exercise the recovery against artificially-induced corruption (`fs::remove_file` on a loose blob), which is the most we can verify until the wild mechanism is identified.

## Alternatives

- **Speculate a cause and ship a redesign.** Rejected — that is what `@0edk3jdm` originally proposed (templates redesign to "prevent" the recovery). Phase 1 showed templates were correlation, not cause; pre-commit's "stash" is a patch file (no git refs); standard cross-worktree GC does not prune HEAD-reachable blobs. Any redesign without a reproducible mechanism would be a fix-by-coincidence.
- **Delete `fix-index` outright.** Rejected — the wild incidents during v2.0.0 work were real (commits `b732222`, `293c0e6`, `fa24c13` all needed `--no-verify`). The recovery code earned its keep even if the cause is unidentified.
- **Re-derive the explanation from probes.** Rejected — Phase 1 already ran 12 scenarios across three probe scripts (cross-worktree GC, bare-flip topology, working-tree clobber, real pre-commit framework with stash dance, concurrent gc during commit, SIGINT mid-flow). All passed. Adding more speculative scenarios is unlikely to find the cause.

## Exit criteria

- `doc/spec/SPEC.md` and `doc/MANUAL.md` no longer assert a cause.
- `fix_index_blobs` and the `fix_index_repairs_missing_head_tree_blob` test remain in place.
- This ADR exists so the question doesn't get re-asked from scratch. If a future investigator does identify a deterministic cause, they should update or supersede this ADR rather than silently re-introducing causal language to the docs.

## References

- `@0edk3jdm` — original "permanent fix-index resolution" issue (rejected after Phase 1).
- `@980mt8qp` — the docs-correction issue this ADR closes.
- `src/internal/hook.rs::fix_index_blobs` — the recovery code.
- `src/internal/hook.rs::tests::fix_index_repairs_missing_head_tree_blob` — recovery test against artificial corruption.
