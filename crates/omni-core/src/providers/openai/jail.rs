//! A fake `CODEX_HOME`.
//!
//! Codex reads all of its settings from one folder and lets you choose which
//! one. Point it at a folder holding nothing but a link to the real credentials
//! and a config file omni wrote, and there is nothing left to auto-load: no MCP
//! servers, no hooks, no `AGENTS.md`.
//!
//! Login still works because the one file that holds it is linked through.
//! Skills are the exception — they live in a shared folder outside
//! `CODEX_HOME`, so the folder trick misses them and they get switched off over
//! the protocol instead.
//!
//! One jail for the whole machine, not one per session: its contents never
//! depend on the conversation, and a shared jail is what lets a single warm
//! app-server serve every session at once.

use std::path::{Path, PathBuf};

use crate::shared::paths;

const MINIMAL: &str = "# written by silicon omni — deliberately almost empty\n";

pub fn real_home() -> PathBuf {
    paths::dirs_home().join(".codex")
}

/// Make (or refresh) codex's jail and return it.
///
/// Always near-empty: the jail *is* how omni isolates codex, so MCP servers,
/// hooks and `AGENTS.md` never load, whether or not `disable_mcp` was asked for.
pub fn build() -> std::io::Result<PathBuf> {
    let home = paths::jail("codex");
    paths::ensure(&home)?;
    relink(&home.join("auth.json"), &real_home().join("auth.json"))?;
    std::fs::write(home.join("config.toml"), MINIMAL)?;
    Ok(home)
}

/// Linked, not copied, so a token refresh on the real file is not lost.
fn relink(link: &Path, real: &Path) -> std::io::Result<()> {
    // A link left from a previous run may point nowhere now.
    if link.symlink_metadata().is_ok() {
        std::fs::remove_file(link)?;
    }
    if real.exists() {
        std::os::unix::fs::symlink(real, link)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_home;

    #[test]
    fn the_jail_holds_only_what_omni_put_there() {
        let _home = scratch_home("codex-jail");
        let jail = build().unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&jail)
            .unwrap()
            .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();
        names.sort();
        // auth.json is only linked when there is a real one to link to.
        names.retain(|name| name != "auth.json");
        assert_eq!(names, vec!["config.toml"]);
    }

    #[test]
    fn a_stale_link_is_replaced_rather_than_left_pointing_nowhere() {
        let _home = scratch_home("codex-relink");
        let jail = paths::jail("codex");
        paths::ensure(&jail).unwrap();
        let link = jail.join("auth.json");
        std::os::unix::fs::symlink("/nowhere/at/all", &link).unwrap();
        build().unwrap();
        assert!(
            !link.exists() || link.read_link().unwrap() != Path::new("/nowhere/at/all"),
            "the dangling link did not survive"
        );
    }

    #[test]
    fn building_twice_is_the_same_as_building_once() {
        let _home = scratch_home("codex-twice");
        assert_eq!(build().unwrap(), build().unwrap());
    }
}
