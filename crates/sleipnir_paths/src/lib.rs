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
    config_path_for(cfg!(windows))
}

pub fn config_path_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join(SETTINGS_FILE)
}

pub fn plugin_dir() -> PathBuf {
    plugin_dir_for(cfg!(windows))
}

pub fn plugin_dir_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join(PLUGINS_DIR)
}

pub fn grants_path() -> PathBuf {
    grants_path_for(cfg!(windows))
}

pub fn grants_path_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join(GRANTS_FILE)
}

pub fn control_socket_path() -> PathBuf {
    control_socket_path_for(cfg!(windows))
}

pub fn control_socket_path_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join(CONTROL_SOCKET_FILE)
}

pub fn agent_control_socket_path() -> PathBuf {
    agent_control_socket_path_for(cfg!(windows))
}

pub fn agent_control_socket_path_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join(AGENT_CONTROL_SOCKET_FILE)
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
    fn derived_paths_share_one_config_dir() {
        for windows in [false, true] {
            let dir = config_dir_for(windows);
            assert_eq!(config_path_for(windows), dir.join(SETTINGS_FILE));
            assert_eq!(plugin_dir_for(windows), dir.join(PLUGINS_DIR));
            assert_eq!(grants_path_for(windows), dir.join(GRANTS_FILE));
            assert_eq!(
                control_socket_path_for(windows),
                dir.join(CONTROL_SOCKET_FILE)
            );
            assert_eq!(
                agent_control_socket_path_for(windows),
                dir.join(AGENT_CONTROL_SOCKET_FILE)
            );
        }
    }
}
