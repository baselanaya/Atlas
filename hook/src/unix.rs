//! The socket the app listens on — must match `settings::socket_path()` in the
//! app exactly. $XDG_RUNTIME_DIR (/run/user/<uid>) is per-user and 0700, so two
//! accounts on the same machine can never meet on the same socket; the role the
//! SID plays in the Windows pipe name.

use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            return dir.join("atlas.sock");
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local").join("state"));
    state.join("atlas").join("runtime").join("atlas.sock")
}
