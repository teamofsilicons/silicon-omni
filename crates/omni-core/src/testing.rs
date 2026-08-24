//! Scaffolding omni's own tests share. Not part of the public engine.
//!
//! Every test that touches disk needs its own `~/.omni`, and the home is
//! process-wide, so tests that move it take a turn rather than racing. Holding
//! the guard is what reserves the turn; dropping it puts the tree back.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::shared::paths;

static TURN: Mutex<()> = Mutex::new(());

pub struct Home {
    pub path: PathBuf,
    _turn: MutexGuard<'static, ()>,
}

/// A private `~/.omni` for one test, deleted when the guard goes out of scope.
pub fn scratch_home(name: &str) -> Home {
    let turn = TURN.lock().unwrap_or_else(|poison| poison.into_inner());
    let path = std::env::temp_dir().join(format!("omni-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::create_dir_all(&path);
    paths::set_home(Some(path.clone()));
    Home { path, _turn: turn }
}

impl Drop for Home {
    fn drop(&mut self) {
        paths::set_home(None);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
