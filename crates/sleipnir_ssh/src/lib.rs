//! Host model shared by Sleipnir's SSH connection UI.
//!
//! The crate is deliberately free of GPUI and of the settings crate so the
//! parsing, merging, and argv-building rules can be unit tested on their own.
//! Two host sources are merged into one list:
//!
//! * [`crate::ssh_config`] — the user's `~/.ssh/config`, read only.
//! * [`AppHost`] — hosts stored in Sleipnir's own `settings.json`.
//!
//! Application hosts win on an alias collision, so an explicit entry always
//! shadows whatever the ssh config says.

pub mod askpass;
pub mod ssh_config;

use serde::{Deserialize, Serialize};

pub use ssh_config::{SshConfigHost, default_ssh_config_path, load_ssh_config, parse_ssh_config};

/// Default ssh port, used only to decide whether to show it in the UI.
pub const DEFAULT_SSH_PORT: u16 = 22;

/// A host configured inside Sleipnir (`settings.json` → `ssh.hosts`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AppHost {
    /// Connection name shown in the UI and used as the tab title.
    pub alias: String,
    /// Hostname or IP address.
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Path to a private key; `~` is expanded by `ssh` itself.
    pub identity_file: Option<String>,
    /// Jump host, passed through as `-J`.
    pub proxy_jump: Option<String>,
    /// Extra `ssh` options. They are always placed *before* the destination so
    /// they can never be mistaken for a remote command.
    pub extra_args: Vec<String>,
}

impl AppHost {
    /// Whether the entry has enough information to connect.
    pub fn is_connectable(&self) -> bool {
        !self.alias.trim().is_empty() && !self.host.trim().is_empty()
    }
}

/// Where a host entry came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostSource {
    /// Parsed from `~/.ssh/config`.
    #[default]
    SshConfig,
    /// Stored in Sleipnir settings.
    App,
}

impl HostSource {
    /// Short label for the connection list.
    pub fn label(&self) -> &'static str {
        match self {
            Self::SshConfig => "ssh config",
            Self::App => "Sleipnir",
        }
    }
}

/// A connectable host, regardless of where it was configured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshHostEntry {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub extra_args: Vec<String>,
    pub source: HostSource,
}

impl SshHostEntry {
    /// `user@hostname`, plus `:port` when it is not the default.
    pub fn display(&self) -> String {
        let mut out = String::new();
        if let Some(user) = &self.user {
            out.push_str(user);
            out.push('@');
        }
        out.push_str(&self.hostname);
        if self.port.is_some_and(|port| port != DEFAULT_SSH_PORT) {
            out.push(':');
            out.push_str(&self.port.unwrap_or(DEFAULT_SSH_PORT).to_string());
        }
        out
    }

    /// The `ssh` argv for this host (the program itself is not included).
    ///
    /// For hosts that came from `~/.ssh/config` this is just the alias: `ssh`
    /// re-reads the same file, so every option the user wrote there (including
    /// multiple `IdentityFile`s, `ProxyCommand`, `LocalForward`, …) keeps
    /// working exactly as on the command line.
    ///
    /// For application hosts the destination is always the **last** argument,
    /// after every option, so `extra_args` can never be executed as a remote
    /// command.
    pub fn argv(&self) -> Vec<String> {
        if self.source == HostSource::SshConfig {
            return vec![self.alias.clone()];
        }
        let mut argv: Vec<String> = Vec::new();
        if let Some(port) = self.port {
            argv.push("-p".into());
            argv.push(port.to_string());
        }
        if let Some(user) = &self.user {
            argv.push("-l".into());
            argv.push(user.clone());
        }
        if let Some(identity) = &self.identity_file {
            argv.push("-i".into());
            argv.push(identity.clone());
        }
        if let Some(jump) = &self.proxy_jump {
            argv.push("-J".into());
            argv.push(jump.clone());
        }
        argv.extend(self.extra_args.iter().cloned());
        argv.push(self.display());
        argv
    }
}

/// Merge the two host sources into one list.
///
/// Application hosts come first (they are the ones the user curated), then the
/// ssh-config hosts; both groups are sorted by alias and de-duplicated, with
/// application hosts winning.
pub fn resolve_hosts(
    config_hosts: &[SshConfigHost],
    app_hosts: &[AppHost],
    read_config: bool,
) -> Vec<SshHostEntry> {
    let mut entries: Vec<SshHostEntry> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let mut app: Vec<&AppHost> = app_hosts.iter().filter(|h| h.is_connectable()).collect();
    app.sort_by_key(|a| a.alias.to_lowercase());
    for host in app {
        if seen.contains(&host.alias) {
            continue;
        }
        seen.push(host.alias.clone());
        entries.push(SshHostEntry {
            alias: host.alias.trim().to_string(),
            hostname: host.host.trim().to_string(),
            user: host.user.clone(),
            port: host.port,
            identity_file: host.identity_file.clone(),
            proxy_jump: host.proxy_jump.clone(),
            extra_args: host.extra_args.clone(),
            source: HostSource::App,
        });
    }

    if read_config {
        let mut config: Vec<&SshConfigHost> = config_hosts.iter().collect();
        config.sort_by_key(|a| a.alias.to_lowercase());
        for host in config {
            if seen.contains(&host.alias) {
                continue;
            }
            seen.push(host.alias.clone());
            entries.push(SshHostEntry {
                alias: host.alias.clone(),
                hostname: host.hostname.clone().unwrap_or_else(|| host.alias.clone()),
                user: host.user.clone(),
                port: host.port,
                identity_file: host.identity_files.first().cloned(),
                proxy_jump: host.proxy_jump.clone(),
                extra_args: Vec::new(),
                source: HostSource::SshConfig,
            });
        }
    }

    entries
}

/// Rank `hosts` against `query`, returning matching indices best-first.
///
/// An empty query returns every index in the original order.
pub fn filter_hosts(hosts: &[SshHostEntry], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..hosts.len()).collect();
    }
    let needle = query.to_lowercase();
    let mut scored: Vec<(u8, usize)> = hosts
        .iter()
        .enumerate()
        .filter_map(|(index, host)| {
            let alias = host.alias.to_lowercase();
            let display = host.display().to_lowercase();
            let score = if alias == needle {
                0
            } else if alias.starts_with(&needle) {
                1
            } else if alias.contains(&needle) {
                2
            } else if display.contains(&needle) {
                3
            } else if is_subsequence(&needle, &alias) {
                4
            } else {
                return None;
            };
            Some((score, index))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| hosts[a.1].alias.cmp(&hosts[b.1].alias))
    });
    scored.into_iter().map(|(_, index)| index).collect()
}

/// Whether every character of `needle` appears in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|want| chars.any(|have| have == want))
}

/// Expand a leading `~/` in a user-supplied path.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// Convenience: load `~/.ssh/config` from its default location.
pub fn load_default_ssh_config() -> Vec<SshConfigHost> {
    load_ssh_config(&default_ssh_config_path())
}

/// Resolve the full host list from defaults in one call.
pub fn resolve_default_hosts(app_hosts: &[AppHost], read_config: bool) -> Vec<SshHostEntry> {
    let config_hosts = if read_config {
        load_default_ssh_config()
    } else {
        Vec::new()
    };
    resolve_hosts(&config_hosts, app_hosts, read_config)
}

/// Sanitize `extra_args`: only options (leading `-`) survive, and every
/// argument is kept so `-o Key=Value` style pairs stay intact.
pub fn sanitize_extra_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| !arg.trim().is_empty())
        .cloned()
        .collect()
}

/// Build the argv for an application host, applying [`sanitize_extra_args`].
pub fn app_host_argv(host: &AppHost) -> Vec<String> {
    let mut entry = entry_from_app_host(host);
    entry.extra_args = sanitize_extra_args(&entry.extra_args);
    entry.argv()
}

fn entry_from_app_host(host: &AppHost) -> SshHostEntry {
    SshHostEntry {
        alias: host.alias.trim().to_string(),
        hostname: host.host.trim().to_string(),
        user: host.user.clone(),
        port: host.port,
        identity_file: host.identity_file.as_deref().map(expand_tilde),
        proxy_jump: host.proxy_jump.clone(),
        extra_args: host.extra_args.clone(),
        source: HostSource::App,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_host(alias: &str, host: &str) -> AppHost {
        AppHost {
            alias: alias.into(),
            host: host.into(),
            ..AppHost::default()
        }
    }

    fn config_host(alias: &str, hostname: &str) -> SshConfigHost {
        SshConfigHost {
            alias: alias.into(),
            hostname: Some(hostname.into()),
            ..SshConfigHost::default()
        }
    }

    #[test]
    fn app_hosts_come_first_and_shadow_config_aliases() {
        let entries = resolve_hosts(
            &[
                config_host("web", "web.example.com"),
                config_host("zeta", "z.example.com"),
            ],
            &[
                app_host("web", "10.1.1.1"),
                app_host("alpha", "a.example.com"),
            ],
            true,
        );
        let aliases: Vec<&str> = entries.iter().map(|e| e.alias.as_str()).collect();
        assert_eq!(aliases, vec!["alpha", "web", "zeta"]);
        assert_eq!(entries[1].hostname, "10.1.1.1");
        assert_eq!(entries[1].source, HostSource::App);
        assert_eq!(entries[2].source, HostSource::SshConfig);
    }

    #[test]
    fn config_hosts_are_skipped_when_disabled() {
        let entries = resolve_hosts(&[config_host("web", "web.example.com")], &[], false);
        assert!(entries.is_empty());
    }

    #[test]
    fn blank_app_hosts_are_dropped() {
        let entries = resolve_hosts(
            &[],
            &[
                AppHost {
                    alias: "  ".into(),
                    host: "h".into(),
                    ..Default::default()
                },
                app_host("ok", "h"),
            ],
            false,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "ok");
    }

    #[test]
    fn config_host_argv_is_just_the_alias() {
        let entry = SshHostEntry {
            alias: "prod".into(),
            hostname: "10.0.0.5".into(),
            user: Some("deploy".into()),
            port: Some(2222),
            identity_file: Some("~/.ssh/id".into()),
            proxy_jump: Some("bastion".into()),
            extra_args: Vec::new(),
            source: HostSource::SshConfig,
        };
        // `ssh prod` must re-read ~/.ssh/config itself so multiple IdentityFile
        // entries and ProxyCommand keep working.
        assert_eq!(entry.argv(), vec!["prod".to_string()]);
    }

    #[test]
    fn app_host_argv_puts_the_destination_last() {
        let entry = SshHostEntry {
            alias: "prod".into(),
            hostname: "10.0.0.5".into(),
            user: Some("deploy".into()),
            port: Some(2222),
            identity_file: Some("~/.ssh/id".into()),
            proxy_jump: Some("bastion".into()),
            extra_args: vec!["-o".into(), "ServerAliveInterval=30".into()],
            source: HostSource::App,
        };
        assert_eq!(
            entry.argv(),
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
    fn display_hides_the_default_port() {
        let mut entry = SshHostEntry {
            alias: "a".into(),
            hostname: "h".into(),
            ..Default::default()
        };
        assert_eq!(entry.display(), "h");
        entry.port = Some(22);
        assert_eq!(entry.display(), "h");
        entry.port = Some(2022);
        assert_eq!(entry.display(), "h:2022");
        entry.user = Some("me".into());
        assert_eq!(entry.display(), "me@h:2022");
    }

    #[test]
    fn filter_ranks_prefix_matches_first() {
        let hosts = vec![
            SshHostEntry {
                alias: "staging".into(),
                hostname: "s".into(),
                ..Default::default()
            },
            SshHostEntry {
                alias: "production".into(),
                hostname: "p".into(),
                ..Default::default()
            },
            SshHostEntry {
                alias: "edge".into(),
                hostname: "e".into(),
                ..Default::default()
            },
        ];
        assert_eq!(filter_hosts(&hosts, "sta"), vec![0]);
        assert_eq!(filter_hosts(&hosts, "pro"), vec![1]);
        assert_eq!(filter_hosts(&hosts, ""), vec![0, 1, 2]);
        // Subsequence still matches, ranked last.
        assert_eq!(filter_hosts(&hosts, "sg"), vec![0]);
        assert!(filter_hosts(&hosts, "zzz").is_empty());
    }

    #[test]
    fn extra_args_keep_their_option_values_and_drop_blanks() {
        let args = vec![
            "-o".into(),
            "StrictHostKeyChecking=accept-new".into(),
            "  ".into(),
            "-A".into(),
        ];
        assert_eq!(
            sanitize_extra_args(&args),
            vec![
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
                "-A".to_string()
            ]
        );
    }

    #[test]
    fn app_host_argv_expands_tilde_identity() {
        let host = AppHost {
            alias: "a".into(),
            host: "h".into(),
            identity_file: Some("~/.ssh/id".into()),
            ..Default::default()
        };
        let argv = app_host_argv(&host);
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            argv,
            vec![
                "-i".to_string(),
                home.join(".ssh/id").to_string_lossy().into_owned(),
                "h".to_string()
            ]
        );
    }
}
