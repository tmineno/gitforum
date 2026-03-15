use serde::{Deserialize, Serialize};

use super::git_ops::GitOps;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Ai,
}

/// A participant in a forum discussion — human or AI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    pub actor_id: String,
    pub kind: ActorKind,
    pub display_name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// Resolve the current actor ID.
///
/// Resolution order:
/// 1. `GIT_FORUM_ACTOR` environment variable (highest priority after `--as`)
/// 2. Git config `user.name` → `human/<slug>` (lowercased, spaces → hyphens)
/// 3. `human/user` fallback
pub fn current_actor(git: &GitOps) -> String {
    if let Ok(actor) = std::env::var("GIT_FORUM_ACTOR") {
        if !actor.is_empty() {
            return actor;
        }
    }
    match git.run(&["config", "user.name"]) {
        Ok(name) if !name.is_empty() => {
            let slug = name.to_lowercase().replace(' ', "-");
            format!("human/{slug}")
        }
        _ => "human/user".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_serialize_roundtrip() {
        let actor = Actor {
            actor_id: "human/alice".into(),
            kind: ActorKind::Human,
            display_name: "Alice".into(),
            roles: vec!["maintainer".into()],
            policy_profile: None,
            key_id: None,
        };
        let json = serde_json::to_string(&actor).unwrap();
        let back: Actor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.actor_id, "human/alice");
        assert_eq!(back.kind, ActorKind::Human);
    }
}
