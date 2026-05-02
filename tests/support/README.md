# Test Infrastructure

This directory contains shared support code for integration tests and E2E scenario tests.

## Directory Layout

Files under `tests/` follow the categories defined in
`doc/spec/TEST-POLICY.md`. Names mirror their owner module in
`src/internal/` so a contributor can predict where a new test
belongs.

```
tests/
  # Module integration (category 1) — drives the library API directly.
  init_test.rs                     init + commit identity
  event_storage_test.rs            write_event / read_event / load_thread_events
  thread_test.rs                   replay, list, create, resolve, timestamp
  doctor_test.rs                   doctor checks + linked-thread advisory
  index_test.rs                    index db + reindex + search + tui startup +
                                   reverse-link queries
  id_alloc_test.rs                 thread ID allocation + validation
  ls_test.rs                       ls render + kind filters
  show_test.rs                     show render + nodes + tree advisory + DEC/TASK
  node_test.rs                     say / objection / resolve / retract / revise /
                                   find_node / node-id resolution
  state_change_test.rs             transitions + guards + fast_track + approvals +
                                   RFC deprecation + DEC/TASK lifecycle
  verify_test.rs                   verify_thread guard reports + linked-thread
                                   advisory
  policy_test.rs                   policy.toml load/lint + facet-scoped guards
  evidence_test.rs                 add_evidence + commit-evidence + show section
  thread_link_test.rs              add_thread_link + show + node_show
  brief_test.rs                    brief render + JSON schema

  # CLI surface (category 2) — spawns the git-forum binary.
  cli_*_test.rs

  # Cross-module behavior (category 3).
  operation_check_test.rs          operation-check rule tables (cross-module)
  migrate_test.rs                  1.x → 2.0 storage rewrite
  hook_test.rs                     git commit-msg hook
  purge_test.rs                    purge subcommand
  github_test.rs                   github import/export

  # Output goldens (category 4).
  snapshot_test.rs

  # End-to-end scenarios (category 5).
  e2e_multiagent_test.rs           Deterministic multi-agent scenario
  e2e_live_agent_test.rs           Live-agent scenario (#[ignore])

  # Shared support (category 6).
  support/
    mod.rs                         Module declarations
    repo.rs                        TestRepo: isolated temp Git repos
    env.rs                         Environment isolation helpers
    clock.rs                       FixedClock / StepClock for tests
    ids.rs                         SequentialIdGenerator for tests
    scenario.rs                    Scenario definition structs + calculator_scenario()
    agent_adapter.rs               AgentAdapter trait + result types
    claude_adapter.rs              Claude Code subprocess adapter
    report.rs                      RFC-0003 report generation (6 sections)
    worktree.rs                    Git worktree helpers for multi-actor tests
```

## Support Modules

### repo.rs

`TestRepo` creates a fresh `git init` in a temp directory with isolated config
(`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`). Dropped on scope exit.

### clock.rs / ids.rs

Injectable `Clock` and `IdGenerator` implementations for deterministic tests.
`StepClock` increments by a fixed duration on each call. `SequentialIdGenerator`
produces predictable IDs like `human-alice-0001`.

### env.rs

`isolated_env()` builds an env map that isolates Git from the host.

### scenario.rs

Defines the data model for E2E scenarios:

- `ActorDef` — name, role, description
- `ThreadDef` — kind, title, body, creator, target status
- `NodeDef` — thread ref, node type, body, actor, should_resolve flag
- `StateTransitionDef` — thread ref, new state, actor, sign actors
- `EvidenceDef` — thread ref, evidence kind, actor
- `LinkDef` — from/to thread refs, relation, actor
- `PhaseDef` — groups of threads, nodes, transitions, evidence, links
- `ScenarioDef` — name, description, actors, phases
- `ExpectedOutcome` — thread ref, expected status, acceptable statuses, min counts

`calculator_scenario()` returns the canonical calculator project scenario with
4 actors (alice, bob, copilot, carol), 3 phases (RFC review, implementation,
contention), 7 threads, and full lifecycle coverage.

### agent_adapter.rs

`AgentAdapter` trait for executing tasks via external agents:

```rust
pub trait AgentAdapter: Send {
    fn execute_task(&self, prompt: &str) -> AgentTaskResult;
    fn shutdown(&mut self);
    fn platform_name(&self) -> &str;
}
```

`AgentTaskResult` captures stdout, stderr, exit code, duration, and success flag.
`AgentRunResult` aggregates per-actor task results.

### claude_adapter.rs

`ClaudeCodeAdapter` implements `AgentAdapter` by spawning `claude -p <prompt>`.
Key features:

- Per-worktree `--cwd` isolation
- `--max-budget-usd 0.50` budget cap
- `build_prompt()` generates goal-based prompts that require agents to discover thread IDs,
  transitions, and command sequences from live repo state
- Enforces `GIT_FORUM_AGENT_TIMEOUT` with subprocess polling and kill-on-timeout
- `is_available()` checks if `claude` CLI is on PATH

### report.rs

Builds and renders RFC-0003 compliant E2E scenario reports with 6 sections:

1. **Project summary** — thread table, actor event counts
2. **Timeline** — chronological event log
3. **Concurrency** — CAS success/retry/error counts
4. **Usability issues** — auto-detected from agent stderr (live-agent mode only)
5. **Coverage** — node types, transitions, evidence kinds exercised vs missing
6. **Recommendations** — auto-generated from findings

`build_report()` replays all threads to build the report. `render_markdown()`
produces the final markdown output. Both modes (deterministic, live-agent) use
the same report pipeline.

### worktree.rs

Git worktree helpers for multi-actor tests:

- `create_actor_worktree()` — creates a worktree + branch for one actor
- `commit_forum_config()` — commits `.forum/` so worktrees can see it
- `setup_actor_worktrees()` — batch setup for all scenario actors

## E2E Test Harness

The E2E harness has two modes, both using the same `calculator_scenario()`:

### Deterministic mode (`e2e_multiagent_test.rs`)

Drives the scenario via direct Rust library calls with fixed clocks and
sequential ID generators. Runs in CI. Phases:

1. **RFC review** — 3 RFCs with nodes, objections, resolutions, state transitions
2. **Implementation** — 4 issues with links, evidence, state transitions
3. **Verify** — policy verification on all 7 threads
4. **Contention** — concurrent writes to test CAS (thread::scope)
5. **Report** — shared report module with outcome comparisons
6. **CLI smoke** — binary invocation of ls, show, verify, kind-filtered ls

```bash
cargo test --test e2e_multiagent_test -- --nocapture
```

### Live-agent mode (`e2e_live_agent_test.rs`)

Spawns real Claude Code agents against the scenario using per-actor worktrees.
Within each phase, participating actors run concurrently and must discover the
right git-forum procedure from repo state rather than following hardcoded IDs or
state transitions.
Never runs in CI (`#[ignore]` + env var gate). Produces a usability report.

```bash
# Run live-agent test (requires claude CLI on PATH)
GIT_FORUM_LIVE_AGENT=1 cargo test --test e2e_live_agent_test -- --ignored --nocapture

# Custom timeout (default: 300s per agent task)
GIT_FORUM_AGENT_TIMEOUT=600 GIT_FORUM_LIVE_AGENT=1 \
  cargo test --test e2e_live_agent_test -- --ignored --nocapture
```

Reports are written to `./tmp/e2e_live_agent_<timestamp>.md`.

Assertions in live-agent mode are structural only:

- At least one thread was created
- All forum refs replay without error
- No duplicate event IDs
- Events come from multiple actors
- At least one thread shows multi-actor collaboration
- At least one state transition was discovered and executed

Agent behavior is non-deterministic, so no content or status assertions.

## Rules

- Do not depend on global Git config
- Do not use the network (except live-agent mode, which is `#[ignore]`)
- Do not snapshot raw commit hashes or timestamps
- Integration tests must not share state
- Each test gets a fresh `TestRepo` in a temp directory
