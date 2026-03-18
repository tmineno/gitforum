#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use git_forum::internal::event::EventType;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::thread;

use super::agent_adapter::AgentRunResult;
use super::scenario::ExpectedOutcome;

// ---------------------------------------------------------------------------
// Data model (RFC-0003 §1–§6)
// ---------------------------------------------------------------------------

/// Full scenario report containing all 6 RFC-0003 sections.
pub struct ScenarioReport {
    pub mode: String,
    pub run_config: RunConfig,
    pub project_summary: ProjectSummary,
    pub timeline: Vec<TimelineEntry>,
    pub contention: Option<ContentionReport>,
    pub usability_issues: Vec<UsabilityIssue>,
    pub coverage: CoverageReport,
    pub recommendations: Vec<String>,
    pub ai_usability_analysis: Option<String>,
    pub outcome_comparisons: Vec<OutcomeComparison>,
    pub agent_results: Vec<AgentRunResult>,
}

/// Run configuration metadata.
pub struct RunConfig {
    pub agents: Vec<AgentConfig>,
}

/// Per-agent configuration recorded in the report.
pub struct AgentConfig {
    pub actor_name: String,
    pub model: String,
    pub command_args: Vec<String>,
}

/// §1 — Project summary.
pub struct ProjectSummary {
    pub threads: Vec<ThreadReport>,
    pub total_nodes: usize,
    pub total_evidence: usize,
    pub actor_event_counts: Vec<(String, usize)>,
}

pub struct ThreadReport {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub node_count: usize,
    pub link_count: usize,
    pub evidence_count: usize,
}

/// §2 — Timeline of actor actions.
pub struct TimelineEntry {
    pub timestamp: String,
    pub actor: String,
    pub thread_id: String,
    pub event_type: String,
    pub summary: String,
}

/// §3 — Concurrency incidents.
pub struct ContentionReport {
    pub success_count: usize,
    pub retry_count: usize,
    pub conflict_errors: Vec<String>,
}

/// §4 — Usability issues.
pub struct UsabilityIssue {
    pub severity: Severity,
    pub category: String,
    pub description: String,
    pub evidence: String,
    pub actor: String,
    pub phase: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "ERROR"),
            Self::Warning => write!(f, "WARNING"),
            Self::Info => write!(f, "INFO"),
        }
    }
}

/// §5 — Coverage.
pub struct CoverageReport {
    pub node_types_used: Vec<String>,
    pub node_types_missing: Vec<String>,
    pub transitions_used: Vec<String>,
    pub transitions_missing: Vec<String>,
    pub evidence_kinds_used: Vec<String>,
    pub link_rels_used: Vec<String>,
    pub cli_commands_used: Vec<String>,
    pub features_exercised: Vec<String>,
}

/// Outcome comparison for expected vs actual thread state.
pub struct OutcomeComparison {
    pub thread_ref: String,
    pub expected_status: String,
    pub actual_status: String,
    pub passed: bool,
    pub node_count: usize,
    pub evidence_count: usize,
    pub link_count: usize,
}

// ---------------------------------------------------------------------------
// Known feature sets (from spec / state_machine.rs)
// ---------------------------------------------------------------------------

fn all_node_types() -> Vec<&'static str> {
    vec![
        "claim",
        "question",
        "objection",
        "alternative",
        "evidence",
        "summary",
        "action",
        "risk",
        "assumption",
        "review",
    ]
}

fn all_transitions() -> Vec<(&'static str, &'static str)> {
    vec![
        // Issue
        ("open", "closed"),
        ("open", "rejected"),
        ("closed", "open"),
        ("rejected", "open"),
        // RFC
        ("draft", "proposed"),
        ("draft", "rejected"),
        ("proposed", "under-review"),
        ("proposed", "draft"),
        ("under-review", "accepted"),
        ("under-review", "rejected"),
        ("under-review", "draft"),
        ("accepted", "deprecated"),
        ("rejected", "deprecated"),
    ]
}

fn all_evidence_kinds() -> Vec<&'static str> {
    vec![
        "commit",
        "file",
        "hunk",
        "test",
        "benchmark",
        "doc",
        "thread",
        "external",
    ]
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a report from thread data, expected outcomes, and optional agent results.
pub fn build_report(
    git: &GitOps,
    expected: &[ExpectedOutcome],
    agent_results: &[AgentRunResult],
    contention: Option<ContentionReport>,
) -> ScenarioReport {
    let mode = if agent_results.is_empty() {
        "deterministic"
    } else {
        "live-agent"
    };

    let thread_ids = thread::list_thread_ids(git).unwrap_or_default();

    // Build project summary (§1)
    let mut threads = Vec::new();
    let mut total_nodes = 0usize;
    let mut total_evidence = 0usize;
    let mut actor_map: HashMap<String, usize> = HashMap::new();
    let mut timeline = Vec::new();
    let mut node_types_seen: HashSet<String> = HashSet::new();
    let mut transitions_seen: HashSet<String> = HashSet::new();
    let mut evidence_kinds_seen: HashSet<String> = HashSet::new();
    let mut link_rels_seen: HashSet<String> = HashSet::new();
    let mut features: HashSet<String> = HashSet::new();

    for id in &thread_ids {
        let state = match thread::replay_thread(git, id) {
            Ok(s) => s,
            Err(_) => continue,
        };

        threads.push(ThreadReport {
            id: state.id.clone(),
            kind: format!("{:?}", state.kind),
            status: state.status.clone(),
            title: state.title.clone(),
            node_count: state.nodes.len(),
            link_count: state.links.len(),
            evidence_count: state.evidence_items.len(),
        });

        total_nodes += state.nodes.len();
        total_evidence += state.evidence_items.len();

        // Collect node types
        for node in &state.nodes {
            node_types_seen.insert(node.node_type.to_string());
        }

        // Collect evidence kinds
        for ev in &state.evidence_items {
            evidence_kinds_seen.insert(ev.kind.to_string());
            features.insert("evidence".to_string());
        }

        // Collect link rels
        for link in &state.links {
            link_rels_seen.insert(link.rel.clone());
            features.insert("thread-links".to_string());
        }

        // Build timeline and actor counts from events
        let mut prev_status: Option<String> = None;
        for ev in &state.events {
            *actor_map.entry(ev.actor.clone()).or_insert(0) += 1;

            let event_desc = match ev.event_type {
                EventType::Create => {
                    format!("create:{}", state.kind)
                }
                EventType::Say => {
                    let nt = ev
                        .node_type
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "node".to_string());
                    format!("say:{nt}")
                }
                EventType::State => {
                    let ns = ev.new_state.as_deref().unwrap_or("?");
                    // Track transition
                    if let Some(ref prev) = prev_status {
                        let trans = format!("{prev}->{ns}");
                        transitions_seen.insert(trans);
                    }
                    prev_status = ev.new_state.clone();
                    format!("state:{ns}")
                }
                EventType::Link => {
                    if ev.evidence.is_some() {
                        "evidence".to_string()
                    } else {
                        let rel = ev.link_rel.as_deref().unwrap_or("link");
                        format!("link:{rel}")
                    }
                }
                EventType::Resolve => "resolve".to_string(),
                EventType::Reopen => "reopen".to_string(),
                EventType::Retract => "retract".to_string(),
                EventType::Edit => "edit".to_string(),
                EventType::ReviseBody => "revise-body".to_string(),
                _ => format!("{}", ev.event_type),
            };

            // Track status for transition detection
            if ev.event_type == EventType::Create {
                prev_status = Some(state.kind.initial_status().to_string());
            }

            let summary_text = match ev.event_type {
                EventType::Create => format!("Created {} \"{}\"", state.kind, state.title),
                EventType::Say => {
                    let nt = ev
                        .node_type
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "node".to_string());
                    let body_preview = ev
                        .body
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>();
                    format!("Added {nt}: {body_preview}")
                }
                EventType::State => {
                    let ns = ev.new_state.as_deref().unwrap_or("?");
                    format!("Changed state to {ns}")
                }
                EventType::Resolve => "Resolved node".to_string(),
                EventType::Link => {
                    if ev.evidence.is_some() {
                        "Attached evidence".to_string()
                    } else {
                        let rel = ev.link_rel.as_deref().unwrap_or("link");
                        format!("Linked ({rel})")
                    }
                }
                _ => format!("{}", ev.event_type),
            };

            timeline.push(TimelineEntry {
                timestamp: ev.created_at.to_rfc3339(),
                actor: ev.actor.clone(),
                thread_id: state.id.clone(),
                event_type: event_desc,
                summary: summary_text,
            });
        }
    }

    // Sort timeline chronologically
    timeline.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    let mut actor_event_counts: Vec<(String, usize)> = actor_map.into_iter().collect();
    actor_event_counts.sort_by(|a, b| b.1.cmp(&a.1));

    let project_summary = ProjectSummary {
        threads,
        total_nodes,
        total_evidence,
        actor_event_counts,
    };

    // Build coverage (§5)
    let all_nt: Vec<&str> = all_node_types();
    let node_types_used: Vec<String> = all_nt
        .iter()
        .filter(|nt| node_types_seen.contains(**nt))
        .map(|s| s.to_string())
        .collect();
    let node_types_missing: Vec<String> = all_nt
        .iter()
        .filter(|nt| !node_types_seen.contains(**nt))
        .map(|s| s.to_string())
        .collect();

    let all_trans = all_transitions();
    let transitions_used: Vec<String> = all_trans
        .iter()
        .filter(|(from, to)| transitions_seen.contains(&format!("{from}->{to}")))
        .map(|(from, to)| format!("{from}->{to}"))
        .collect();
    let transitions_missing: Vec<String> = all_trans
        .iter()
        .filter(|(from, to)| !transitions_seen.contains(&format!("{from}->{to}")))
        .map(|(from, to)| format!("{from}->{to}"))
        .collect();

    let all_ek = all_evidence_kinds();
    let evidence_kinds_used: Vec<String> = all_ek
        .iter()
        .filter(|ek| evidence_kinds_seen.contains(**ek))
        .map(|s| s.to_string())
        .collect();

    let link_rels_used: Vec<String> = link_rels_seen.into_iter().collect();

    // CLI commands used (live-agent only: parse from stdout)
    let mut cli_commands_used: Vec<String> = Vec::new();
    for ar in agent_results {
        for task in &ar.tasks {
            for line in task.stdout.lines() {
                if line.contains("git-forum") || line.contains("git forum") {
                    let cmd = line.trim().to_string();
                    if !cli_commands_used.contains(&cmd) {
                        cli_commands_used.push(cmd);
                    }
                }
            }
        }
    }

    if contention.is_some() {
        features.insert("contention".to_string());
    }

    let features_exercised: Vec<String> = features.into_iter().collect();

    let coverage = CoverageReport {
        node_types_used,
        node_types_missing,
        transitions_used,
        transitions_missing,
        evidence_kinds_used,
        link_rels_used,
        cli_commands_used,
        features_exercised,
    };

    // Build run config from agent results
    let mut seen_actors: Vec<String> = Vec::new();
    let mut agent_configs = Vec::new();
    for ar in agent_results {
        if !seen_actors.contains(&ar.actor_name) {
            seen_actors.push(ar.actor_name.clone());
            agent_configs.push(AgentConfig {
                actor_name: ar.actor_name.clone(),
                model: ar.model.clone(),
                command_args: ar.command_args.clone(),
            });
        }
    }
    let run_config = RunConfig {
        agents: agent_configs,
    };

    // Build usability issues (§4) — from agent results
    let usability_issues = detect_usability_issues(agent_results);

    // Build outcome comparisons
    let outcome_comparisons = build_outcome_comparisons(git, expected);

    // Build recommendations (§6)
    let mut report = ScenarioReport {
        mode: mode.to_string(),
        run_config,
        project_summary,
        timeline,
        contention,
        usability_issues,
        coverage,
        recommendations: vec![],
        ai_usability_analysis: None,
        outcome_comparisons,
        agent_results: vec![],
    };

    report.recommendations = generate_recommendations(&report);
    report
}

// ---------------------------------------------------------------------------
// Usability issue detection (§4)
// ---------------------------------------------------------------------------

fn detect_usability_issues(agent_results: &[AgentRunResult]) -> Vec<UsabilityIssue> {
    let mut issues = Vec::new();

    for ar in agent_results {
        for task in &ar.tasks {
            // Non-zero exit code
            if let Some(code) = task.exit_code {
                if code != 0 {
                    issues.push(UsabilityIssue {
                        severity: Severity::Error,
                        category: "confusing-error".to_string(),
                        description: format!("git-forum exited with code {code}"),
                        evidence: truncate_output(&task.stderr, 200),
                        actor: ar.actor_name.clone(),
                        phase: String::new(),
                    });
                }
            }

            // Stderr contains error indicators
            for line in task.stderr.lines() {
                let lower = line.to_lowercase();
                if lower.contains("error:") || lower.contains("fail") {
                    issues.push(UsabilityIssue {
                        severity: Severity::Error,
                        category: "confusing-error".to_string(),
                        description: "Error detected in agent stderr".to_string(),
                        evidence: line.to_string(),
                        actor: ar.actor_name.clone(),
                        phase: String::new(),
                    });
                }
            }

            // Agent used --help mid-task
            if task.stdout.contains("--help") || task.stdout.contains("--help-llm") {
                issues.push(UsabilityIssue {
                    severity: Severity::Info,
                    category: "missing-affordance".to_string(),
                    description: "Agent consulted help documentation mid-task".to_string(),
                    evidence: String::new(),
                    actor: ar.actor_name.clone(),
                    phase: String::new(),
                });
            }

            // Timeout
            if !task.success && task.exit_code.is_none() {
                issues.push(UsabilityIssue {
                    severity: Severity::Error,
                    category: "timeout".to_string(),
                    description: "Agent process timed out".to_string(),
                    evidence: String::new(),
                    actor: ar.actor_name.clone(),
                    phase: String::new(),
                });
            }
        }
    }

    issues
}

fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

// ---------------------------------------------------------------------------
// Outcome comparisons
// ---------------------------------------------------------------------------

fn build_outcome_comparisons(git: &GitOps, expected: &[ExpectedOutcome]) -> Vec<OutcomeComparison> {
    let mut results = Vec::new();

    for exp in expected {
        match thread::replay_thread(git, &exp.thread_ref) {
            Ok(state) => {
                let passed = state.status == exp.expected_status
                    && state.nodes.len() >= exp.min_nodes
                    && state.evidence_items.len() >= exp.expected_evidence_count
                    && state.links.len() >= exp.expected_link_count;
                results.push(OutcomeComparison {
                    thread_ref: exp.thread_ref.clone(),
                    expected_status: exp.expected_status.clone(),
                    actual_status: state.status.clone(),
                    passed,
                    node_count: state.nodes.len(),
                    evidence_count: state.evidence_items.len(),
                    link_count: state.links.len(),
                });
            }
            Err(_) => {
                results.push(OutcomeComparison {
                    thread_ref: exp.thread_ref.clone(),
                    expected_status: exp.expected_status.clone(),
                    actual_status: "NOT_FOUND".to_string(),
                    passed: false,
                    node_count: 0,
                    evidence_count: 0,
                    link_count: 0,
                });
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Recommendations (§6)
// ---------------------------------------------------------------------------

fn generate_recommendations(report: &ScenarioReport) -> Vec<String> {
    let mut recs = Vec::new();

    // Failed outcome comparisons — describe the actual failing criterion
    for oc in &report.outcome_comparisons {
        if !oc.passed {
            if oc.actual_status == "NOT_FOUND" {
                recs.push(format!(
                    "Thread {} was never created (expected status '{}').",
                    oc.thread_ref, oc.expected_status
                ));
            } else if oc.actual_status != oc.expected_status {
                recs.push(format!(
                    "Thread {} ended in '{}' instead of expected '{}'.",
                    oc.thread_ref, oc.actual_status, oc.expected_status
                ));
            } else {
                // Status matches but a count criterion failed
                recs.push(format!(
                    "Thread {} status matches ('{}') but failed a count check \
                     (nodes={}, evidence={}, links={}). \
                     Check min_nodes/expected_evidence_count/expected_link_count in scenario.",
                    oc.thread_ref,
                    oc.actual_status,
                    oc.node_count,
                    oc.evidence_count,
                    oc.link_count
                ));
            }
        }
    }

    // Missing transitions
    if !report.coverage.transitions_missing.is_empty() {
        recs.push(format!(
            "State transitions not exercised: {}. Consider adding scenario steps.",
            report.coverage.transitions_missing.join(", ")
        ));
    }

    // Error-severity usability issues (deduplicate by description)
    let mut seen_errors: Vec<String> = Vec::new();
    for issue in &report.usability_issues {
        if issue.severity == Severity::Error && !seen_errors.contains(&issue.description) {
            seen_errors.push(issue.description.clone());
            recs.push(format!(
                "Agent encountered error: {}. Consider improving error message or CLI affordance.",
                issue.description
            ));
        }
    }

    // High retry count
    if let Some(ref c) = report.contention {
        if c.retry_count > 3 {
            recs.push(format!(
                "High retry count ({}) suggests CAS contention. Consider semantic merge.",
                c.retry_count
            ));
        }
    }

    // Missing node types
    if !report.coverage.node_types_missing.is_empty() {
        recs.push(format!(
            "Node types not exercised: {}.",
            report.coverage.node_types_missing.join(", ")
        ));
    }

    // --- git-forum usability recommendations (live-agent mode) ---
    if report.mode == "live-agent" {
        // Check if agents needed help docs
        let help_count = report
            .usability_issues
            .iter()
            .filter(|i| i.category == "missing-affordance")
            .count();
        if help_count > 0 {
            recs.push(format!(
                "Agents consulted --help {help_count} time(s). CLI discoverability may need improvement (clearer error messages, subcommand suggestions)."
            ));
        }

        // Check for agents that failed to create expected threads
        let failed_outcomes = report
            .outcome_comparisons
            .iter()
            .filter(|oc| oc.actual_status == "NOT_FOUND")
            .count();
        if failed_outcomes > 0 {
            recs.push(format!(
                "{failed_outcomes} expected thread(s) were never created. Agents may struggle with the thread creation workflow. Consider improving `--help-llm` examples or error guidance."
            ));
        }

        // Check if agents produced wrong states (thread exists but wrong status)
        let wrong_states: Vec<&OutcomeComparison> = report
            .outcome_comparisons
            .iter()
            .filter(|oc| {
                !oc.passed
                    && oc.actual_status != "NOT_FOUND"
                    && oc.actual_status != oc.expected_status
            })
            .collect();
        if !wrong_states.is_empty() {
            let examples: Vec<String> = wrong_states
                .iter()
                .map(|oc| {
                    format!(
                        "{} (expected {}, got {})",
                        oc.thread_ref, oc.expected_status, oc.actual_status
                    )
                })
                .collect();
            recs.push(format!(
                "Thread(s) reached wrong state: {}. Consider clearer guard violation messages or multi-step transition shortcuts.",
                examples.join("; ")
            ));
        }

        // Overall success rate
        let total = report.outcome_comparisons.len();
        let passed = report
            .outcome_comparisons
            .iter()
            .filter(|oc| oc.passed)
            .count();
        if total > 0 {
            let pct = (passed * 100) / total;
            recs.push(format!(
                "Overall outcome accuracy: {passed}/{total} ({pct}%). This reflects how well agents can drive git-forum end-to-end."
            ));
        }
    }

    recs
}

// ---------------------------------------------------------------------------
// Markdown renderer
// ---------------------------------------------------------------------------

/// Render a ScenarioReport as RFC-0003 compliant markdown.
pub fn render_markdown(report: &ScenarioReport) -> String {
    let mut out = String::new();

    out.push_str(&format!("# E2E Scenario Report ({})\n\n", report.mode));

    // Run configuration
    if !report.run_config.agents.is_empty() {
        out.push_str("## Run Configuration\n\n");
        out.push_str("| Actor | Model | Command |\n");
        out.push_str("|---|---|---|\n");
        for ac in &report.run_config.agents {
            let cmd = if ac.command_args.is_empty() {
                "(deterministic — library calls)".to_string()
            } else {
                format!("`{}`", ac.command_args.join(" "))
            };
            out.push_str(&format!("| {} | {} | {} |\n", ac.actor_name, ac.model, cmd));
        }
        out.push('\n');
    }

    // §1 — Project Summary
    out.push_str("## 1. Project Summary\n\n");
    out.push_str("| ID | Kind | Status | Title | Nodes | Links | Evidence |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    for t in &report.project_summary.threads {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            t.id, t.kind, t.status, t.title, t.node_count, t.link_count, t.evidence_count
        ));
    }
    out.push_str(&format!(
        "\nTotal nodes: {}, Total evidence: {}\n\n",
        report.project_summary.total_nodes, report.project_summary.total_evidence
    ));
    out.push_str("Actor activity:\n");
    for (actor, count) in &report.project_summary.actor_event_counts {
        out.push_str(&format!("- {actor}: {count} events\n"));
    }
    out.push('\n');

    // §2 — Timeline
    out.push_str("## 2. Timeline\n\n");
    out.push_str("| Time | Actor | Thread | Event | Summary |\n");
    out.push_str("|---|---|---|---|---|\n");
    for entry in &report.timeline {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            entry.timestamp, entry.actor, entry.thread_id, entry.event_type, entry.summary
        ));
    }
    out.push('\n');

    // §3 — Concurrency
    out.push_str("## 3. Concurrency\n\n");
    if let Some(ref c) = report.contention {
        out.push_str(&format!(
            "Successes: {}, Retries: {}\n",
            c.success_count, c.retry_count
        ));
        if !c.conflict_errors.is_empty() {
            out.push_str("\nConflict errors:\n");
            for err in &c.conflict_errors {
                out.push_str(&format!("- {err}\n"));
            }
        }
    } else {
        out.push_str("No contention testing performed.\n");
    }
    out.push('\n');

    // §4 — Usability Issues
    out.push_str("## 4. Usability Issues\n\n");
    if report.usability_issues.is_empty() {
        out.push_str("No usability issues detected.\n");
    } else {
        out.push_str("| Severity | Category | Actor | Phase | Description |\n");
        out.push_str("|---|---|---|---|---|\n");
        for issue in &report.usability_issues {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                issue.severity, issue.category, issue.actor, issue.phase, issue.description
            ));
        }
    }
    out.push('\n');

    // §5 — Coverage
    out.push_str("## 5. Coverage\n\n");
    let total_nt = all_node_types().len();
    let total_trans = all_transitions().len();
    out.push_str(&format!(
        "Node types: {} / {}\n",
        report.coverage.node_types_used.len(),
        total_nt
    ));
    out.push_str(&format!(
        "Transitions: {} / {}\n",
        report.coverage.transitions_used.len(),
        total_trans
    ));
    if !report.coverage.node_types_missing.is_empty() {
        out.push_str(&format!(
            "Missing node types: {}\n",
            report.coverage.node_types_missing.join(", ")
        ));
    }
    if !report.coverage.transitions_missing.is_empty() {
        out.push_str(&format!(
            "Missing transitions: {}\n",
            report.coverage.transitions_missing.join(", ")
        ));
    }
    if !report.coverage.evidence_kinds_used.is_empty() {
        out.push_str(&format!(
            "Evidence kinds: {}\n",
            report.coverage.evidence_kinds_used.join(", ")
        ));
    }
    if !report.coverage.link_rels_used.is_empty() {
        out.push_str(&format!(
            "Link rels: {}\n",
            report.coverage.link_rels_used.join(", ")
        ));
    }
    if !report.coverage.cli_commands_used.is_empty() {
        out.push_str(&format!(
            "CLI commands observed: {}\n",
            report.coverage.cli_commands_used.len()
        ));
    }
    if !report.coverage.features_exercised.is_empty() {
        out.push_str(&format!(
            "Features exercised: {}\n",
            report.coverage.features_exercised.join(", ")
        ));
    }
    out.push('\n');

    // Outcome comparisons
    if !report.outcome_comparisons.is_empty() {
        out.push_str("### Outcome Comparisons\n\n");
        out.push_str("| Thread | Expected | Actual | Nodes | Evidence | Links | Pass |\n");
        out.push_str("|---|---|---|---|---|---|---|\n");
        for oc in &report.outcome_comparisons {
            let pass_str = if oc.passed { "PASS" } else { "FAIL" };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                oc.thread_ref,
                oc.expected_status,
                oc.actual_status,
                oc.node_count,
                oc.evidence_count,
                oc.link_count,
                pass_str
            ));
        }
        out.push('\n');
    }

    // §6 — Recommendations
    out.push_str("## 6. Recommendations\n\n");
    if report.recommendations.is_empty() && report.ai_usability_analysis.is_none() {
        out.push_str("No recommendations.\n");
    } else {
        if !report.recommendations.is_empty() {
            out.push_str("### Structured Findings\n\n");
            for rec in &report.recommendations {
                out.push_str(&format!("- {rec}\n"));
            }
            out.push('\n');
        }
        if let Some(ref analysis) = report.ai_usability_analysis {
            out.push_str("### AI Usability Analysis\n\n");
            out.push_str(analysis);
            out.push('\n');
        }
    }

    out
}

// ---------------------------------------------------------------------------
// AI-generated usability analysis
// ---------------------------------------------------------------------------

/// Build a context summary from the report for the AI to analyze.
fn build_analysis_context(report: &ScenarioReport) -> String {
    let mut ctx = String::new();

    ctx.push_str("You are analyzing the results of an E2E test where AI agents used a CLI tool called `git-forum` to manage structured discussions (RFCs, issues) in a Git repository.\n\n");

    ctx.push_str("## Agent Configuration\n\n");
    for ac in &report.run_config.agents {
        ctx.push_str(&format!("- {} (model: {})\n", ac.actor_name, ac.model));
    }
    ctx.push('\n');

    ctx.push_str("## Outcome Comparisons\n\n");
    for oc in &report.outcome_comparisons {
        let status = if oc.passed { "PASS" } else { "FAIL" };
        ctx.push_str(&format!(
            "- {} expected={} actual={} nodes={} evidence={} links={} [{}]\n",
            oc.thread_ref,
            oc.expected_status,
            oc.actual_status,
            oc.node_count,
            oc.evidence_count,
            oc.link_count,
            status,
        ));
    }
    ctx.push('\n');

    ctx.push_str("## Detected Issues\n\n");
    if report.usability_issues.is_empty() {
        ctx.push_str("No automated usability issues detected.\n");
    } else {
        for issue in &report.usability_issues {
            ctx.push_str(&format!(
                "- [{}] {} (actor: {}, category: {})\n",
                issue.severity, issue.description, issue.actor, issue.category
            ));
            if !issue.evidence.is_empty() {
                ctx.push_str(&format!("  evidence: {}\n", issue.evidence));
            }
        }
    }
    ctx.push('\n');

    ctx.push_str("## Coverage\n\n");
    ctx.push_str(&format!(
        "Node types used: {}/{}\n",
        report.coverage.node_types_used.len(),
        report.coverage.node_types_used.len() + report.coverage.node_types_missing.len()
    ));
    ctx.push_str(&format!(
        "Transitions used: {}/{}\n",
        report.coverage.transitions_used.len(),
        report.coverage.transitions_used.len() + report.coverage.transitions_missing.len()
    ));
    if !report.coverage.node_types_missing.is_empty() {
        ctx.push_str(&format!(
            "Missing node types: {}\n",
            report.coverage.node_types_missing.join(", ")
        ));
    }
    if !report.coverage.transitions_missing.is_empty() {
        ctx.push_str(&format!(
            "Missing transitions: {}\n",
            report.coverage.transitions_missing.join(", ")
        ));
    }
    ctx.push('\n');

    if let Some(ref c) = report.contention {
        ctx.push_str("## Concurrency\n\n");
        ctx.push_str(&format!(
            "Successes: {}, Retries: {}\n",
            c.success_count, c.retry_count
        ));
        if !c.conflict_errors.is_empty() {
            ctx.push_str(&format!("Conflict errors: {}\n", c.conflict_errors.len()));
        }
        ctx.push('\n');
    }

    ctx.push_str("## Agent Outputs (excerpts)\n\n");
    for ar in &report.agent_results {
        ctx.push_str(&format!("### {} (model: {})\n", ar.actor_name, ar.model));
        for (i, task) in ar.tasks.iter().enumerate() {
            ctx.push_str(&format!(
                "Task {}: success={} exit={:?} duration={:.1}s\n",
                i + 1,
                task.success,
                task.exit_code,
                task.duration.as_secs_f64()
            ));
            // Include truncated stdout for context
            let stdout_preview: String = task.stdout.chars().take(1000).collect();
            if !stdout_preview.is_empty() {
                ctx.push_str(&format!("stdout:\n```\n{stdout_preview}\n```\n"));
            }
            let stderr_preview: String = task.stderr.chars().take(500).collect();
            if !stderr_preview.is_empty() {
                ctx.push_str(&format!("stderr:\n```\n{stderr_preview}\n```\n"));
            }
        }
        ctx.push('\n');
    }

    ctx
}

/// Call an AI model to generate a usability analysis of the E2E test results.
///
/// Returns None if the claude CLI is unavailable or the call fails.
pub fn generate_ai_usability_analysis(report: &ScenarioReport, model: &str) -> Option<String> {
    let context = build_analysis_context(report);

    let prompt = format!(
        "{context}\n\
        ---\n\n\
        Based on the E2E test results above, write a usability analysis of `git-forum` as a CLI tool for AI agents. \
        Address these questions:\n\n\
        1. **Discoverability**: How easily could agents figure out the right commands? Did any agents struggle with command syntax or subcommand structure?\n\
        2. **Error messages**: Were error messages clear enough for agents to self-correct? Note any confusing errors.\n\
        3. **Workflow friction**: Were there multi-step sequences that could be simplified? Any commands that required awkward workarounds?\n\
        4. **Missing affordances**: What CLI features would help agents succeed more reliably?\n\
        5. **Overall assessment**: Rate the CLI's agent-friendliness and suggest the top 3 improvements.\n\n\
        Be specific and reference actual agent behavior from the outputs. Keep the analysis concise (under 500 words). \
        Write in markdown."
    );

    let output = std::process::Command::new("claude")
        .args(["-p", &prompt, "--model", model, "--max-budget-usd", "0.50"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let analysis = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if analysis.is_empty() {
        None
    } else {
        Some(analysis)
    }
}
