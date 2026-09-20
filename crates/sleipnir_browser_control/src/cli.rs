//! CLI entry points; registration is preview-only unless --apply is passed.
use crate::{ENDPOINT_ENV, Request, TOKEN_ENV, WINDOW_ENV, mcp, transport};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

fn configuration(executable: &Path) -> serde_json::Value {
    json!({"mcpServers":{"sleipnir-browser":{"command":executable,"args":["browser-mcp"],"env_vars":[ENDPOINT_ENV,TOKEN_ENV,WINDOW_ENV]}}})
}
fn codex_table(executable: &Path) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["command"] = toml_edit::value(executable.to_string_lossy().to_string());
    table["args"] = toml_edit::value(["browser-mcp"].into_iter().collect::<toml_edit::Array>());
    table["env_vars"] = toml_edit::value(
        [ENDPOINT_ENV, TOKEN_ENV, WINDOW_ENV]
            .into_iter()
            .collect::<toml_edit::Array>(),
    );
    table
}
fn registration_document(original: &str, executable: &Path) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = original
        .parse()
        .map_err(|e| format!("Invalid existing Codex config: {e}"))?;
    if doc
        .get("mcp_servers")
        .and_then(|v| v.get("sleipnir-browser"))
        .is_some()
    {
        return Err("sleipnir-browser already exists; no configuration was replaced".into());
    }
    if doc.get("mcp_servers").is_some_and(|v| !v.is_table()) {
        return Err("mcp_servers is not a TOML table; no changes made".into());
    }
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["mcp_servers"]["sleipnir-browser"] = toml_edit::Item::Table(codex_table(executable));
    Ok(doc.to_string())
}
fn register_codex(path: &Path, executable: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid config path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if path.is_symlink() {
        return Err("Refusing to replace a symlinked config".into());
    }
    atomic_write::with_file_lock(path, || {
        if path.is_symlink() {
            return Err(std::io::Error::other(
                "Refusing to replace a symlinked config",
            ));
        }
        let existed = path.exists();
        let original = if existed {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        let updated =
            registration_document(&original, executable).map_err(std::io::Error::other)?;
        if existed {
            // Keep an auditable, never-overwritten backup immediately beside the config.
            let backup = parent.join(format!(
                "config.toml.before-sleipnir-{}.bak",
                uuid::Uuid::new_v4().simple()
            ));
            let mut opts = std::fs::OpenOptions::new();
            opts.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            let mut out = opts.open(backup)?;
            out.write_all(original.as_bytes())?;
            out.sync_all()?;
        }
        atomic_write::save_atomic(path, updated.as_bytes())?;
        if std::fs::read_to_string(path)? != updated {
            return Err(std::io::Error::other("Config verification failed"));
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
}

pub fn run(mode: &str, args: impl Iterator<Item = String>) -> ExitCode {
    match execute(mode, args.collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
fn execute(mode: &str, args: Vec<String>) -> Result<(), String> {
    match mode {
        "browser-mcp" if args.is_empty() => mcp::run(),
        "browser-agent" => {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["config"] => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&configuration(&executable))
                            .map_err(|e| e.to_string())?
                    );
                    Ok(())
                }
                ["register", "codex"] | ["register", "codex", "--dry-run"] => {
                    let mut doc = toml_edit::DocumentMut::new();
                    doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
                    doc["mcp_servers"]["sleipnir-browser"] =
                        toml_edit::Item::Table(codex_table(&executable));
                    println!(
                        "# Preview only; use register codex --apply to write configuration.\n{doc}"
                    );
                    Ok(())
                }
                ["register", "codex", "--apply"] => {
                    let home = std::env::var_os("CODEX_HOME")
                        .map(PathBuf::from)
                        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
                        .ok_or("Cannot locate Codex config")?;
                    register_codex(&home.join("config.toml"), &executable)?;
                    println!(
                        "Registered sleipnir-browser. Restart Codex inside a new Sleipnir terminal. Enable Agent access in the browser panel."
                    );
                    Ok(())
                }
                _ => Err(
                    "Usage: sleipnir browser-agent config | register codex [--dry-run|--apply]"
                        .into(),
                ),
            }
        }
        "browser-ctl" => {
            let credentials = transport::Credentials::from_env()?;
            let request = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["list"] => Request::List,
                ["status"] => Request::Status { window: credentials.window },
                ["read-text"] => Request::ReadText { window: credentials.window },
                ["navigate", url] => Request::Navigate { window: credentials.window, url: (*url).into() },
                _ => return Err("Usage: sleipnir browser-ctl list | status | read-text | navigate <absolute-url>".into()),
            };
            let response = transport::call(&credentials, request)?;
            println!(
                "{}",
                serde_json::to_string(&response).map_err(|e| e.to_string())?
            );
            if response.is_error() {
                Err("Browser operation failed; see structured response".into())
            } else {
                Ok(())
            }
        }
        _ => Err("Invalid browser command or unsupported arguments".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_preserves_existing_settings_and_refuses_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("config.toml");
        let original = "# keep\nmodel = 'example'\n[mcp_servers.existing]\ncommand = 'keep-me'\n";
        std::fs::write(&file, original).unwrap();
        register_codex(&file, Path::new("/some path/sleipnir")).unwrap();
        let updated = std::fs::read_to_string(&file).unwrap();
        let doc: toml_edit::DocumentMut = updated.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("example"));
        assert_eq!(
            doc["mcp_servers"]["existing"]["command"].as_str(),
            Some("keep-me")
        );
        assert_eq!(
            doc["mcp_servers"]["sleipnir-browser"]["env_vars"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(!updated.contains("SLEIPNIR_BROWSER_TOKEN ="));
        assert!(updated.starts_with("# keep"));
        assert!(register_codex(&file, Path::new("/new")).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), updated);
        let backups: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|p| p.path().extension().is_some_and(|x| x == "bak"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(backups[0].path()).unwrap(),
            original
        );
    }
}
