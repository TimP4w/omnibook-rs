use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".config")
        });
    base.join("omnibook")
}

pub fn haptic_config_path() -> PathBuf {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("haptic_intensity")
}

pub fn presence_config_path() -> PathBuf {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("presence")
}

fn runtime_dir() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::var("HOME").map(PathBuf::from).unwrap_or_default().join(".cache"))
        .join("omnibook")
}

/// Ephemeral state file written by omnibookd and read by the GTK app.
/// Lives in XDG_RUNTIME_DIR so it is wiped on logout/reboot.
pub fn daemon_state_path() -> PathBuf {
    let dir = runtime_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("state")
}

/// Unix domain socket used for daemon ↔ UI IPC.
pub fn daemon_socket_path() -> PathBuf {
    let dir = runtime_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("omnibookd.sock")
}
