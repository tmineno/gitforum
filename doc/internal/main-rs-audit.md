# `src/main.rs` Command-Surface Audit

Source of truth for the Phase 2 extraction order. Bound by ADR-011 and
`doc/internal/3.0-removal-plan.md`.

This audit enumerates every `Commands::*` arm currently inline in
`src/main.rs` (3166 lines, 49 arms) so each Phase 2 cutover commit knows
exactly what to extract, what to delete, and which `internal::*` imports
disappear from `main.rs` as a result.

Branch tip audited: `v3.0.0-draft` after `lenm78ma` (post compat→legacy
rename). Counts and line ranges may drift; the source of truth is the
file itself — re-grep `^        Commands::` to refresh.

## Audit dimensions

For each arm:

- **Lines** — start line of `Commands::Foo {…} => {` through to the
  blank line before the next arm (or, for the last arm, the closing
  `}` of the `match`).
- **Legacy deps** — modules / types from
  `internal::{event, workflow, index, write_ops, state_change, create,
  repair*, prune, purge, timeline, reindex, github*}`, or types
  re-exported from them (`Lifecycle`, `NodeType`, `ThreadKind`,
  `EventType`). Body refs only — module-level `use` lines are tracked
  separately in §"main.rs imports" of `3.0-removal-plan.md`.
- **Disposition** — one of:
  - `EXTRACT` — arm body relocates to `internal::commands::<cmd>::run`
    (existing or new module). main.rs arm becomes a thin call site.
  - `EXTRACT (already)` — body is already a thin wrapper over an existing
    `commands::*` `run_*`; only the arm itself relocates.
  - `DELETE` — entire arm removed; the underlying module is on the
    Phase 4 DELETE list and the CLI surface is dropped per SPEC-3.0
    Appendix A. Phase 2 still removes the arm so `main.rs` no longer
    imports the doomed module.
  - `DEPRECATED` — node-shorthand alias removed per SPEC-3.0 §2.2 / ADR-006
    (only 4 canonical node types survive: Comment, Objection, Action,
    Evidence). Removed at slot 2.
- **Target** — the destination module path under
  `src/internal/commands/`. `(NEW)` flags a module not yet in the v2.0.2
  tree. Empty for `DELETE`.
- **Slot** — Phase 2 cutover order. Slots are atomic commits; an entry
  with multiple sub-letters (e.g. `7a`, `7b`) means each sub-letter is
  its own commit but they may land back-to-back since they touch
  disjoint files.

## Per-arm table

| Arm | Lines | Body legacy deps | Disposition | Target | Slot |
|---|---:|---|---|---|---|
| `Init` | 1149-1234 | `reindex::run_reindex` (after fetch); `init::*`; `hook::install_all_hooks`; `actor::actor_from_git_config` | EXTRACT (handler split — `init::*` library stays peer-level) | `commands/init.rs` (NEW) | 10a |
| `Doctor` | 1235-1326 | `doctor::*` (KEEP file, rewired Phase 1) | EXTRACT | `commands/doctor.rs` | 8 |
| `Reindex` | 1327-1341 | `reindex::run_reindex` | DELETE — `reindex.rs` is on the Phase 4 DELETE list (ADR-011 Decision 6) | — | 11 |
| `PruneOrphans` | 1342-1366 | `prune::scan`, `prune::delete` | DELETE — `prune.rs` is DELETE | — | 11 |
| `PruneStaleEvents` | 1367-1406 | `prune::scan_stale_events`, `prune::apply_stale_event_plans`; index-rebuild hint (no call) | DELETE — `prune.rs` is DELETE | — | 11 |
| `Migrate` | 1407-1429 | `migrate::run`; `reindex::run_reindex` (post-write) | EXTRACT — the only sanctioned legacy consumer (ADR-011 Decision 3) | `commands/migrate.rs` | 9 |
| `Repair` | 1430-1470 | `commands::repair_workflow::run_workflow_repair`; `repair::repair_conflicts`; `reindex::run_reindex` | DELETE — `repair.rs`, `repair_workflow.rs`, `commands/repair_workflow.rs` are all DELETE | — | 11 |
| `Purge` | 1471-1587 | `event::EventType::Say`; `purge::plan_purge_event`, `purge::purge_event`, `purge::plan_purge_actor`, `purge::purge_actor`; `reindex::run_reindex`; `thread::resolve_node_id_in_thread`, `thread::replay_thread` | DELETE — `purge.rs` is DELETE; SPEC-3.0 Appendix A: replaced by Git history rewrite guidance | — | 11 |
| `Search` | 1588-1600 | `index::open_db`, `index::search_threads`; `translate_legacy_kind_query` (helper); `ls::render_search_results` | DELETE — search becomes tree-scan in v3.0; an indexed return path is a v3.1 concern (ADR-011 Decision 6). The arm's runtime is removed; if a snapshot-tree search is wanted at v3.0.0 it is a NEW arm, not a rewrite. | — | 11 |
| `Tui` | 1601-1607 | `forum_tui::run` (KEEP, frozen) | EXTRACT | `commands/tui.rs` (NEW) | 10c |
| `Import` | 1608-1658 | `github::list_issues`; `github_import::plan_import`, `import_all`, `import_issue`; helper `print_import_plan` | DELETE — `github*.rs` are DELETE (ADR-011 Decision 7) | — | 11 |
| `Export` | 1659-1686 | `github_export::plan_export`, `export_issue`; helper `print_export_plan` | DELETE — `github*.rs` are DELETE | — | 11 |
| `Thread` (`ThreadCmd::New`) | 1687-1724 | `parse_lifecycle` (helper → `Lifecycle::parse`); calls extracted `run_canonical_thread_new` | EXTRACT (already) — arm body is a thin wrapper; only the arm relocates | `commands/thread_new.rs` | 1 |
| `New` | 1725-1776 | `preset_lookup` (helper → `workflow::SPEC`); calls extracted `run_canonical_thread_new` | EXTRACT (already) — arm wrapper relocates | `commands/thread_new.rs` | 1 |
| `Close` | 1777-1801 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Pend` | 1802-1822 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Accept` | 1823-1846 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Propose` | 1847-1867 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Deprecate` | 1868-1888 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Reject` | 1889-1910 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Withdraw` | 1911-1932 | calls extracted `run_state_shorthand` | EXTRACT (already) | `commands/state.rs` | 3 |
| `Ls` | 1933-1958 | `parse_thread_kind` (→ `ThreadKind`); `list_thread_states`; `thread::ThreadState`; `ls::render_ls` | EXTRACT | `commands/ls.rs` | 7a |
| `Shortlog` | 1959-1975 | `parse_thread_kind`, `parse_since_date`, `terminal_state_date` (helper → `event::EventType::State`); `list_thread_states`; `ls::render_shortlog` | EXTRACT — `terminal_state_date` rewires to read snapshot tip timestamp directly (no event scan) | `commands/shortlog.rs` (NEW) | 7b |
| `Show` | 1976-2010 | `thread::replay_thread`; `Policy::load`; `show::*`; `collect_implements_children` (helper → `index::*`) | EXTRACT — replay→snapshot read; `collect_implements_children` rewires to a tree scan (per Phase 4 index removal) | `commands/show.rs` | 7c |
| `Log` | 2011-2075 | `thread::replay_thread`; `event::Event` iteration; `timeline::event_display_type`, `timeline::render_markdown_refs`; `parse_since_date` | DELETE — domain-timeline view is on the Appendix A list. SPEC-3.0 §5.4: `git forum log` becomes a Git-history view (`git log` over the snapshot ref). Reintroducing the user-facing command as a Git wrapper is a separate v3.0.0 task, not a Phase 2 extraction of the current body. | — | 11 |
| `Diff` | 2076-2083 | `thread::replay_thread`; `diff::diff_body` | EXTRACT — rewires from event-replay to snapshot-tree diff | `commands/diff.rs` | 7d |
| `Status` | 2084-2099 | `thread::replay_thread`; `show::render_show` (`ShowMode::Status`) | EXTRACT | `commands/status.rs` (NEW) | 7e |
| `Node` (`NodeCmd::Show`, `NodeCmd::Add`) | 2100-2132 | `thread::find_node`; `show::render_node_show`; calls extracted `run_shorthand_say` | EXTRACT — `Show` arm gets a fresh `commands/node.rs::run_node_show`; `Add` arm is already a thin wrapper | `commands/node.rs` (NEW) | 7f |
| `Branch` (`BranchCmd::Bind`, `BranchCmd::Clear`) | 2133-2156 | `branch_ops::set_branch` (peer file moves to `commands/branch.rs`) | EXTRACT — relocate with the peer file | `commands/branch.rs` (renamed from `branch_ops.rs`) | 6 |
| `Revise` | 2157-2213 | calls extracted `revise_cmd::run_revise_body`, `run_revise_node` | EXTRACT (already) | `commands/revise.rs` | 5 |
| `Comment` | 2214-2234 | `NodeType::Comment`; calls extracted `run_shorthand_say` | EXTRACT (already) — surviving canonical type | `commands/shorthand_say.rs` | 2 |
| `Claim` | 2235-2258 | `NodeType::Claim`; `warn_legacy_node_shorthand`; `run_shorthand_say` | DEPRECATED — `Claim` is not a canonical 3.0 NodeType; arm removed | — | 2 |
| `Question` | 2259-2282 | `NodeType::Question`; `warn_legacy_node_shorthand`; `run_shorthand_say` | DEPRECATED | — | 2 |
| `Objection` | 2283-2303 | `NodeType::Objection`; `run_shorthand_say` | EXTRACT (already) — surviving canonical type | `commands/shorthand_say.rs` | 2 |
| `Summary` | 2304-2327 | `NodeType::Summary`; `warn_legacy_node_shorthand`; `run_shorthand_say` | DEPRECATED | — | 2 |
| `Action` | 2328-2348 | `NodeType::Action`; `run_shorthand_say` | EXTRACT (already) — surviving canonical type | `commands/shorthand_say.rs` | 2 |
| `Risk` | 2349-2372 | `NodeType::Risk`; `warn_legacy_node_shorthand`; `run_shorthand_say` | DEPRECATED | — | 2 |
| `Review` | 2373-2397 | `NodeType::Review`; `warn_legacy_node_shorthand`; `run_shorthand_say` | DEPRECATED | — | 2 |
| `Retract` | 2398-2410 | `event::EventType::Retract`; calls extracted `run_node_lifecycle_bulk` | EXTRACT (already) — the `EventType` literal is the only legacy bit; the run_* signature changes during Phase 1 to take a non-event-shaped tag | `commands/node_bulk.rs` | 4 |
| `Resolve` | 2411-2423 | `event::EventType::Resolve`; `run_node_lifecycle_bulk` | EXTRACT (already) | `commands/node_bulk.rs` | 4 |
| `Reopen` | 2424-2454 | `event::EventType::Reopen`; `run_node_lifecycle_bulk`; falls through to `run_state_shorthand` when `node_ids` is empty | EXTRACT (already) — composite arm; the dispatch logic stays in the arm wrapper | `commands/node_bulk.rs` + `commands/state.rs` | 4 |
| `Retype` | 2455-2495 | `event::NodeType` (parse); `thread::replay_thread`; `Policy::load`; `operation_check::check_revise`; `apply_operation_checks`; `write_ops::retype_node`; `show::short_oid` | EXTRACT — rewires from event write to snapshot mutation (overwrite `nodes/<id>.toml` `node_type` field) | `commands/retype.rs` (NEW) | 7g |
| `State` | 2496-2639 | `Policy::load`; `state_change::StateChangeOptions`, `fast_track_state`, `change_state`, `StateChangeOutcome`; `run_bulk_state_change` (extracted); `BulkSelectors`; `evidence::add_thread_link`; `thread::replay_thread`; `parse_thread_kind_filter` | EXTRACT — slot 3 already covers shorthand variants; this arm composes `run_state_shorthand` for the canonical form. Phase 1 replaces `state_change::*` with direct `thread.toml.status` writes | `commands/state.rs` | 3 |
| `Brief` | 2640-2654 | `thread::replay_thread`; `brief::*`; `read_incoming_link_counts` (helper → `index::*`); `serde_json` | EXTRACT — link-count helper rewires to either snapshot tree-scan or returns zeros (per SPEC-3.0 §9.2 the index is optional acceleration) | `commands/brief.rs` | 7h |
| `Verify` | 2655-2686 | `thread::replay_thread`; `Policy::load`; `verify::verify_thread`; `state_change::remediation_hint` | EXTRACT — `state_change::remediation_hint` becomes a method on the new policy/category surface | `commands/verify.rs` | 7i |
| `Evidence` (`EvidenceCmd::Add`) | 2687-2722 | `thread::replay_thread`; `Policy::load`; `evidence::add_evidence`; `operation_check::check_evidence`; `apply_operation_checks`; `EvidenceKind` | EXTRACT — rewires to write `evidence/<id>.toml` directly | `commands/evidence.rs` (NEW) | 7j |
| `Link` | 2723-2736 | `evidence::add_thread_link` | EXTRACT — rewires to append a row in `links.toml` | `commands/link.rs` (NEW) | 7k |
| `Hook` (`Install`, `Uninstall`, `CheckCommitMsg`, `FixIndex`, `WorktreeInit`) | 2737-2817 | `hook::*`; `init::init_forum_local`, `init::ensure_forum_refspecs`; `actor::actor_from_git_config` | EXTRACT (handler split — `hook::*` library stays peer-level then moves to `commands/hook.rs` per existing plan; sub-arm logic stays inside that file) | `commands/hook.rs` | 10b |
| `Policy` (`Show`, `Lint`, `Check`) | 2818-2864 | `Policy::load`; `policy::render_policy_show`, `lint_policy`, `check_guards`; `thread::replay_thread` | EXTRACT — Phase 1 rewires `policy::*` to category registry | `commands/policy.rs` (NEW) | 10d |

49 arms total. Top-level grouping:

- Phase 2 EXTRACT (already): 16 arms across slots 1, 3, 4, 5
  (thread/state shorthands + already-extracted `run_*` callers)
- Phase 2 EXTRACT (new modules): 14 arms across slots 6, 7a-7k, 8, 9, 10a-10d
- Phase 2 DEPRECATED (delete during slot 2): 5 arms (`Claim`, `Question`,
  `Summary`, `Risk`, `Review`)
- Phase 2 DELETE (slot 11 — drop with deleted modules): 9 arms
  (`Reindex`, `PruneOrphans`, `PruneStaleEvents`, `Repair`, `Purge`,
  `Search`, `Import`, `Export`, `Log`)
- Phase 2 EXTRACT total: 35 (49 − 5 deprecated − 9 deleted)

## Helper functions in main.rs

Free functions defined after `fn main()` (lines 2870-3166) that the
arms call:

| Helper | Lines | Used by | Disposition |
|---|---:|---|---|
| `preset_lookup` | 2877-2879 | `New`, `parse_thread_kind`, `translate_legacy_kind_query` | Move to `commands/thread_new.rs` (slot 1). After Phase 1's category rewrite the kind preset table itself disappears; the helper goes with it. |
| `valid_preset_names` | 2881-2887 | `parse_thread_kind` (error message) | Same fate as `preset_lookup` |
| `parse_thread_kind` | 2889-2896 | `Ls`, `Shortlog`, `parse_thread_kind_filter` | Move to `commands/shared.rs` (slot 7a, with first user). Removed when `kind` filter is replaced by `category` (Phase 1 model rewrite). |
| `parse_lifecycle` | 2898-2904 | `Thread` arm | Move to `commands/thread_new.rs` (slot 1). Removed in Phase 1 (lifecycle → category). |
| `translate_legacy_kind_query` | 2920-2946 | `Search` (DELETE) | Goes with `Search` arm at slot 11. |
| `parse_since_date` | 2948-2963 | `Log` (DELETE), `Shortlog` | Move to `commands/shortlog.rs` (slot 7b). Stays at v3.0.0 — date parsing is generic. |
| `terminal_state_date` | 2965-2982 | `Shortlog` | Move to `commands/shortlog.rs` (slot 7b). Body rewires to read snapshot tip timestamp instead of scanning `event::EventType::State`. |
| `parse_thread_kind_filter` | 2984-2986 | `State` (bulk) | Move to `commands/shared.rs` with `parse_thread_kind` (slot 3). |
| `parse_unrecognized_subcommand` | 2989-2995 | `main` error path | Stays in `main.rs` — clap-error glue. |
| `subcommand_hint` | 3003-3027 | `main` error path | Stays in `main.rs`. |
| `print_import_plan` | 3029-3043 | `Import` (DELETE) | Goes with `Import` at slot 11. |
| `collect_implements_children` | 3057-3093 | `Show` (`--tree`) | Move to `commands/show.rs` (slot 7c). Phase 4 (index deletion) requires it to switch from `index::find_incoming_links` to a tree scan over thread snapshots. |
| `read_incoming_link_counts` | 3101-3120 | `Brief` | Move to `commands/brief.rs` (slot 7h). Same Phase 4 rewire — return zeros if no index, or scan snapshots. |
| `fallback_scan_implements` | 3126-3149 | `collect_implements_children` | Move with `collect_implements_children`; promoted to the primary path once `index.rs` is deleted. |
| `print_export_plan` | 3151-3166 | `Export` (DELETE) | Goes with `Export` at slot 11. |

## Phase 2 cutover order (consolidated)

| Slot | Commit subject | What it touches |
|---:|---|---|
| 1 | `Thread::New`, `New` arms relocate into `commands/thread_new.rs` | `parse_lifecycle`, `preset_lookup`, `valid_preset_names` move with them. main.rs drops `Lifecycle`, `KindPreset`, `SPEC` line-2875 use. |
| 2 | shorthand_say arms cleaned up | Relocate `Comment`, `Objection`, `Action` arm bodies into `commands/shorthand_say.rs`. Delete `Claim`, `Question`, `Summary`, `Risk`, `Review` arms (and their `Commands::*` enum variants + the `warn_legacy_node_shorthand` helper). main.rs drops `NodeType` for shorthand types. |
| 3 | state arms relocate into `commands/state.rs` | `Close`, `Pend`, `Accept`, `Propose`, `Deprecate`, `Reject`, `Withdraw`, `State`, `Reopen` (empty-node-ids branch). Move `parse_thread_kind`, `parse_thread_kind_filter` to `commands/shared.rs`. main.rs drops `state_change::*`, `BulkSelectors`. |
| 4 | node_bulk arms relocate | `Retract`, `Resolve`, `Reopen` (with-nodes branch) into `commands/node_bulk.rs`. main.rs drops `event::EventType::{Retract, Resolve, Reopen}`. |
| 5 | revise arm relocates into `commands/revise.rs` | Already a thin wrapper; mechanical move only. |
| 6 | branch arms relocate; `branch_ops.rs` → `commands/branch.rs` | Both peer-file rename and arm extraction in one commit. |
| 7a | ls → `commands/ls.rs` | `parse_thread_kind` already at `commands/shared.rs` after slot 3. |
| 7b | shortlog → `commands/shortlog.rs` (NEW) | `parse_since_date`, `terminal_state_date` move with it. `terminal_state_date` rewires to snapshot tip timestamp. |
| 7c | show → `commands/show.rs` | `collect_implements_children`, `fallback_scan_implements` move with it. Phase 1 cutover replaces `thread::replay_thread` with snapshot read. |
| 7d | diff → `commands/diff.rs` | Snapshot-tree diff replaces event-body diff. |
| 7e | status → `commands/status.rs` (NEW) | Tiny wrapper over `show::render_show(ShowMode::Status)`. |
| 7f | node → `commands/node.rs` (NEW) | `NodeCmd::Show`. `NodeCmd::Add` already a thin wrapper. |
| 7g | retype → `commands/retype.rs` (NEW) | Rewires from `write_ops::retype_node` (event write) to snapshot field overwrite. main.rs drops `write_ops`. |
| 7h | brief → `commands/brief.rs` | `read_incoming_link_counts` moves with it. |
| 7i | verify → `commands/verify.rs` | Drops `state_change::remediation_hint` — moves into `commands/verify.rs` or onto the policy/category surface. |
| 7j | evidence → `commands/evidence.rs` (NEW) | Rewires to TOML write. |
| 7k | link → `commands/link.rs` (NEW) | Rewires to `links.toml` row append. |
| 8 | doctor → `commands/doctor.rs` | Internal checks rewired around snapshot validation. |
| 9 | migrate → `commands/migrate.rs` | The single sanctioned legacy consumer; imports `internal::legacy::*`. |
| 10a | init handler → `commands/init.rs` (NEW) | Library code (`init.rs`) stays peer-level — used by hook worktree-init too. |
| 10b | hook → `commands/hook.rs` | Library code stays peer-level. Hook script rewritten for `Refs:` trailer (SPEC-3.0 §2.5). |
| 10c | tui → `commands/tui.rs` (NEW) | Trivial wrapper. |
| 10d | policy → `commands/policy.rs` (NEW) | Phase 1 already rewrote `policy.rs` to the category registry. |
| 10e | help handler → `commands/help.rs` (NEW) | Library code (`help.rs`) stays peer-level. Vocabulary updated to 3.0 categories. |
| 11 | drop the `internal::*` deletes' arms | `Reindex`, `PruneOrphans`, `PruneStaleEvents`, `Repair`, `Purge`, `Search`, `Import`, `Export`, `Log` — remove `Commands::*` variants, arm bodies, and corresponding helpers (`translate_legacy_kind_query`, `print_import_plan`, `print_export_plan`). main.rs drops `index`, `prune`, `purge`, `repair`, `reindex`, `timeline`, `github`, `github_export`, `github_import`. The peer modules themselves are removed in Phase 4 by `git rm` per `3.0-removal-plan.md`. |

After slot 11, `main.rs` import block contains only:

```
clap, config, error::ForumError, git_ops::GitOps, lint_emit::{self, LintEmitter},
commands::{shared::*, every per-command run_* fn}
```

with no references to the modules listed in the acceptance criteria.

## Verification at end of Phase 2

The build-time gate (task `3dx6szoh`) checks `main.rs` against the same
forbidden module list `internal::commands/*` files use, with one exception
documented in `3.0-removal-plan.md` §"Heuristic for is this KEEP file
done?": main.rs is allowed to import `internal::commands::migrate`.
That module reaches `internal::legacy::*` internally; main.rs does not.

A grep contract for the post-Phase-2 state:

```
grep -E "use git_forum::internal::(event|workflow|index|write_ops|state_change|create|repair|repair_workflow|prune|purge|timeline|reindex|github)" src/main.rs
# expected: zero matches
```

This is the regression check the gate enforces.
