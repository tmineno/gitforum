//! `git forum repair --workflow-violations` orchestration (#uu9wxn1d).

use crate::internal::clock::Clock;
use crate::internal::error::ForumResult;
use crate::internal::repair_workflow::{self, RepairAction, RepairPlan};

use super::shared::{discover_repo_with_init_warning, resolve_actor};

pub fn run_workflow_repair(
    apply: bool,
    as_actor: Option<String>,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let plans = repair_workflow::plan(&git)?;

    if plans.is_empty() {
        println!("No workflow violations found.");
        return Ok(());
    }

    print_plans(&plans, apply);

    if apply {
        let actor = resolve_actor(as_actor, &git);
        let written = repair_workflow::apply(&git, &plans, &actor, clock)?;
        println!();
        println!("Wrote {written} corrective event(s).");
        let unfixable: Vec<&RepairPlan> = plans
            .iter()
            .filter(|p| matches!(p.action, RepairAction::Unfixable { .. }))
            .collect();
        if !unfixable.is_empty() {
            println!();
            println!(
                "{} thread(s) remain flagged after repair (sink terminals; cannot be fixed via append-only):",
                unfixable.len()
            );
            for plan in unfixable {
                println!(
                    "  {} (terminal '{}')",
                    plan.thread_id, plan.current_terminal
                );
            }
        }
    } else {
        println!();
        println!("Dry-run only. Re-run with `--apply` to write the corrective events.");
    }
    Ok(())
}

fn print_plans(plans: &[RepairPlan], apply: bool) {
    let header = if apply { "REPAIR PLAN" } else { "DRY-RUN PLAN" };
    println!("{header} — {} thread(s) flagged", plans.len());
    println!();
    for plan in plans {
        println!(
            "  {} ({} lifecycle, terminal '{}')",
            plan.thread_id, plan.lifecycle, plan.current_terminal
        );
        println!(
            "    illegal edge: '{}' -> '{}' on event {}",
            plan.illegal_from, plan.illegal_to, plan.offending_event_id
        );
        match &plan.action {
            RepairAction::Append { steps } => {
                let walk: Vec<String> = steps.iter().map(|s| format!("`{s}`")).collect();
                println!("    action:      append state events: {}", walk.join(" → "));
            }
            RepairAction::AlreadyRepaired => {
                println!("    action:      already repaired (chain self-heals)");
            }
            RepairAction::Unfixable { reason } => {
                println!("    action:      UNFIXABLE — {reason}");
            }
        }
        println!();
    }
}
