//! Connection planning for remote SSH panes.
//!
//! A connection is always launched as a plain `ssh` child process inside the
//! normal PTY pipeline, so every ssh feature (agent forwarding, jump hosts,
//! port forwarding, `ControlMaster`, …) keeps working exactly as it does on the
//! command line. This module only decides the argv, the tab title, and the
//! environment — in particular the askpass wiring.

pub mod askpass;

use std::path::Path;

use collections::HashMap;
use sleipnir_settings::TerminalSettings;
use sleipnir_ssh::{HostSource, SshHostEntry, sanitize_extra_args};

pub use self::askpass::AskpassServer;

use self::askpass::{askpass_env, ensure_askpass_helper};

/// Everything needed to spawn an SSH pane.
pub struct SshConnectPlan {
    /// Program to run, normally `ssh` from `settings.ssh.program`.
    pub program: String,
    /// argv *without* the program.
    pub argv: Vec<String>,
    /// Tab title override, normally the host alias.
    pub title: Option<String>,
    /// Extra environment layered over the pane's inherited environment.
    pub env: HashMap<String, String>,
    /// Askpass server for this connection, when in-app prompts are active.
    pub askpass: Option<AskpassServer>,
}

impl SshConnectPlan {
    /// Build a plan for a resolved host entry.
    ///
    /// `exe` is the running Sleipnir binary; it is only needed so the askpass
    /// helper script can exec it. Failures in the askpass setup are logged and
    /// degrade to terminal-typed input instead of failing the connection.
    pub fn for_entry(entry: SshHostEntry, settings: &TerminalSettings, exe: Option<&Path>) -> Self {
        let program = settings
            .ssh
            .program
            .clone()
            .unwrap_or_else(|| "ssh".to_string());
        let argv = argv_for(&entry);
        let title = Some(entry.alias);

        let mut env = HashMap::default();
        let mut askpass = None;
        #[cfg(unix)]
        if settings.ssh.askpass {
            if let Some(exe) = exe {
                if ensure_askpass_helper(exe) {
                    if let Some(server) = AskpassServer::bind() {
                        if let Some(socket) = server.socket_path() {
                            env.extend(askpass_env(socket));
                            askpass = Some(server);
                        }
                    }
                }
            }
        }
        // Markers inherited from the launching shell describe that shell's
        // integration, not this SSH session.
        clear_integration_markers(&mut env);

        Self {
            program,
            argv,
            title,
            env,
            askpass,
        }
    }
}

/// argv for a resolved entry. Config-sourced hosts pass just the alias so
/// `ssh` re-reads `~/.ssh/config` itself; app hosts get explicit options.
fn argv_for(entry: &SshHostEntry) -> Vec<String> {
    if entry.source == HostSource::SshConfig {
        return vec![entry.alias.clone()];
    }
    let mut argv: Vec<String> = Vec::new();
    if let Some(port) = entry.port {
        argv.push("-p".into());
        argv.push(port.to_string());
    }
    if let Some(user) = &entry.user {
        argv.push("-l".into());
        argv.push(user.clone());
    }
    if let Some(identity) = &entry.identity_file {
        argv.push("-i".into());
        argv.push(identity.clone());
    }
    if let Some(jump) = &entry.proxy_jump {
        argv.push("-J".into());
        argv.push(jump.clone());
    }
    argv.extend(sanitize_extra_args(&entry.extra_args));
    argv.push(entry.display());
    argv
}

/// Environment markers inherited from the parent shell. They describe *that*
/// shell's integration, not this SSH session, and would make a remote shell
/// skip its own setup — so they are blanked for the child.
pub(crate) const STALE_INTEGRATION_MARKERS: [&str; 5] = [
    "ITERM_SESSION_ID",
    "GHOSTTY_RESOURCES_DIR",
    "KITTY_SHELL_INTEGRATION",
    "WEZTERM_EXECUTABLE",
    "SLEIPNIR_OSC133_LOADED",
];

/// Blank the inherited integration markers in `env`.
pub(crate) fn clear_integration_markers(env: &mut HashMap<String, String>) {
    for key in STALE_INTEGRATION_MARKERS {
        env.insert(key.to_string(), String::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> TerminalSettings {
        TerminalSettings::default()
    }

    fn entry(alias: &str, host: &str, source: HostSource) -> SshHostEntry {
        SshHostEntry {
            alias: alias.into(),
            hostname: host.into(),
            user: Some("deploy".into()),
            port: Some(2222),
            identity_file: Some("~/.ssh/id".into()),
            proxy_jump: Some("bastion".into()),
            extra_args: vec!["-o".into(), "ServerAliveInterval=30".into()],
            source,
        }
    }

    #[test]
    fn config_entries_pass_only_the_alias() {
        let plan = SshConnectPlan::for_entry(
            entry("prod", "10.0.0.5", HostSource::SshConfig),
            &settings(),
            None,
        );
        assert_eq!(plan.argv, vec!["prod".to_string()]);
        assert_eq!(plan.title.as_deref(), Some("prod"));
        assert_eq!(plan.program, "ssh");
    }

    #[test]
    fn app_entries_build_explicit_options_before_the_destination() {
        let plan = SshConnectPlan::for_entry(
            entry("prod", "10.0.0.5", HostSource::App),
            &settings(),
            None,
        );
        assert_eq!(
            plan.argv,
            vec![
                "-p".to_string(),
                "2222".to_string(),
                "-l".to_string(),
                "deploy".to_string(),
                "-i".to_string(),
                "~/.ssh/id".to_string(),
                "-J".to_string(),
                "bastion".to_string(),
                "-o".to_string(),
                "ServerAliveInterval=30".to_string(),
                "deploy@10.0.0.5:2222".to_string(),
            ]
        );
    }

    #[test]
    fn custom_program_is_honoured() {
        let mut settings = settings();
        settings.ssh.program = Some("/usr/bin/ssh".into());
        let plan =
            SshConnectPlan::for_entry(entry("prod", "10.0.0.5", HostSource::App), &settings, None);
        assert_eq!(plan.program, "/usr/bin/ssh");
    }

    #[test]
    fn integration_markers_are_blanked_not_removed() {
        let mut env = HashMap::default();
        env.insert("ITERM_SESSION_ID".to_string(), "w0t1p0".into());
        clear_integration_markers(&mut env);
        assert_eq!(env.get("ITERM_SESSION_ID").map(String::as_str), Some(""));
        for key in STALE_INTEGRATION_MARKERS {
            assert!(env.contains_key(key), "{key} must be present but empty");
        }
    }

    #[test]
    fn no_askpass_server_is_created_without_a_binary() {
        let plan = SshConnectPlan::for_entry(
            entry("prod", "10.0.0.5", HostSource::App),
            &settings(),
            None,
        );
        assert!(plan.askpass.is_none());
        // Askpass variables must be absent, but the inherited integration
        // markers are still blanked so the remote shell sets itself up.
        for key in [
            "SSH_ASKPASS",
            "SSH_ASKPASS_REQUIRE",
            "SLEIPNIR_SSH_ASKPASS_SOCK",
        ] {
            assert!(!plan.env.contains_key(key), "{key} must not be set");
        }
        for key in STALE_INTEGRATION_MARKERS {
            assert_eq!(plan.env.get(key).map(String::as_str), Some(""), "{key}");
        }
    }

    #[test]
    fn askpass_can_be_disabled_in_settings() {
        let mut settings = settings();
        settings.ssh.askpass = false;
        let plan = SshConnectPlan::for_entry(
            entry("prod", "10.0.0.5", HostSource::App),
            &settings,
            Some(Path::new("/bin/true")),
        );
        assert!(plan.askpass.is_none());
        for key in [
            "SSH_ASKPASS",
            "SSH_ASKPASS_REQUIRE",
            "SLEIPNIR_SSH_ASKPASS_SOCK",
        ] {
            assert!(!plan.env.contains_key(key), "{key} must not be set");
        }
    }
}
