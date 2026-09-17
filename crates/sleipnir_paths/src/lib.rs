//! Canonical on-disk layout for Sleipnir.
//!
//! On Unix/macOS this is `~/.config/sleipnir`, **not** `dirs::config_dir()`
//! (`~/Library/Application Support` on macOS). Every crate that needs a path
//! under that directory must go through this module so settings, plugins,
//! grants, and sockets cannot drift.

use std::path::PathBuf;

pub const SETTINGS_FILE: &str = "settings.json";
pub const PLUGINS_DIR: &str = "plugins";
pub const GRANTS_FILE: &str = "plugin-grants.json";
pub const CONTROL_SOCKET_FILE: &str = "control.sock";
pub const AGENT_CONTROL_SOCKET_FILE: &str = "agent-control.sock";

/// Directory that holds `settings.json` and local plugin configuration.
pub fn config_dir() -> PathBuf {
    config_dir_for(cfg!(windows))
}

/// Config directory for a given OS family.
pub fn config_dir_for(windows: bool) -> PathBuf {
    if windows {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir")
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

pub fn plugin_dir() -> PathBuf {
    config_dir().join(PLUGINS_DIR)
}

pub fn grants_path() -> PathBuf {
    config_dir().join(GRANTS_FILE)
}

pub fn control_socket_path() -> PathBuf {
    config_dir().join(CONTROL_SOCKET_FILE)
}

pub fn agent_control_socket_path() -> PathBuf {
    config_dir().join(AGENT_CONTROL_SOCKET_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_config_dir_is_dot_config_sleipnir() {
        let unix = config_dir_for(false);
        assert!(
            unix.ends_with(".config/sleipnir"),
            "unix config dir should be ~/.config/sleipnir, got {unix:?}"
        );
    }

    #[test]
    fn windows_config_dir_is_os_config_dir_join_sleipnir() {
        let win = config_dir_for(true);
        assert_eq!(win.file_name().and_then(|s| s.to_str()), Some("sleipnir"));
        if let Some(config) = dirs::config_dir() {
            assert_eq!(win, config.join("sleipnir"));
        }
    }

    #[test]
    fn host_paths_hang_off_config_dir() {
        let dir = config_dir();
        assert_eq!(config_path(), dir.join(SETTINGS_FILE));
        assert_eq!(plugin_dir(), dir.join(PLUGINS_DIR));
        assert_eq!(grants_path(), dir.join(GRANTS_FILE));
        assert_eq!(control_socket_path(), dir.join(CONTROL_SOCKET_FILE));
        assert_eq!(
            agent_control_socket_path(),
            dir.join(AGENT_CONTROL_SOCKET_FILE)
        );
    }
}
