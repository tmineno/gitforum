#![allow(dead_code)]
use std::collections::HashMap;

/// Build an environment map that isolates tests from the host.
///
/// Sets `GIT_CONFIG_NOSYSTEM`, overrides `HOME` and `XDG_CONFIG_HOME`
/// to a provided temp path so that no global dotfiles leak in.
pub fn isolated_env(tmp_home: &std::path::Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    env.insert("HOME".into(), tmp_home.display().to_string());
    env.insert(
        "XDG_CONFIG_HOME".into(),
        tmp_home.join(".config").display().to_string(),
    );
    env
}
