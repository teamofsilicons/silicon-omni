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
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
};

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
    let mut config = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(home.join("config.toml"))?;
    config.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    config.write_all(MINIMAL.as_bytes())?;
    Ok(home)
}

/// Linked, not copied, so a token refresh on the real file is not lost.
fn relink(link: &Path, real: &Path) -> std::io::Result<()> {
    // A link left from a previous run may point nowhere now.
    if link.symlink_metadata().is_ok() {
        std::fs::remove_file(link)?;
    }
    if let Some(parent) = real.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Keep the link even before the first login. Codex can then create the
    // real credential file through it; otherwise a first-time login lands in
    // the disposable jail and vanishes the next time `build` refreshes it.
    std::os::unix::fs::symlink(real, link)
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

    #[test]
    fn first_login_can_create_the_real_auth_file_through_a_dangling_link() {
        let root = std::env::temp_dir().join(format!("omni-codex-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let link = root.join("jail/auth.json");
        let real = root.join("real/auth.json");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();

        relink(&link, &real).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), real);
        std::fs::write(&link, "credential").unwrap();
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "credential");

        let _ = std::fs::remove_dir_all(root);
    }
}
