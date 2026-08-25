//! Bundled Streamline Core Line icons for known coding-agent tab marks.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// GPUI asset source for the agent-mark SVGs under `icons/`.
pub struct AgentAssets;

impl AgentAssets {
    const ALL: &'static [&'static str] = &[
        "icons/aider.svg",
        "icons/amp.svg",
        "icons/claude.svg",
        "icons/codex.svg",
        "icons/copilot.svg",
        "icons/crush.svg",
        "icons/cursor.svg",
        "icons/gemini.svg",
        "icons/goose.svg",
        "icons/grok.svg",
        "icons/opencode.svg",
        "icons/pi.svg",
    ];
}

impl AssetSource for AgentAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Some(Cow::Borrowed(match path {
            "icons/aider.svg" => include_bytes!("../icons/aider.svg").as_slice(),
            "icons/amp.svg" => include_bytes!("../icons/amp.svg").as_slice(),
            "icons/claude.svg" => include_bytes!("../icons/claude.svg").as_slice(),
            "icons/codex.svg" => include_bytes!("../icons/codex.svg").as_slice(),
            "icons/copilot.svg" => include_bytes!("../icons/copilot.svg").as_slice(),
            "icons/crush.svg" => include_bytes!("../icons/crush.svg").as_slice(),
            "icons/cursor.svg" => include_bytes!("../icons/cursor.svg").as_slice(),
            "icons/gemini.svg" => include_bytes!("../icons/gemini.svg").as_slice(),
            "icons/goose.svg" => include_bytes!("../icons/goose.svg").as_slice(),
            "icons/grok.svg" => include_bytes!("../icons/grok.svg").as_slice(),
            "icons/opencode.svg" => include_bytes!("../icons/opencode.svg").as_slice(),
            "icons/pi.svg" => include_bytes!("../icons/pi.svg").as_slice(),
            _ => return Ok(None),
        })))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = path.trim_end_matches('/');
        Ok(Self::ALL
            .iter()
            .copied()
            .filter(|p| prefix.is_empty() || *p == prefix || p.starts_with(&format!("{prefix}/")))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_every_bundled_agent_icon() {
        let assets = AgentAssets;
        for path in AgentAssets::ALL {
            let bytes = assets
                .load(path)
                .unwrap()
                .unwrap_or_else(|| panic!("missing asset {path}"));
            assert!(
                bytes.starts_with(b"<svg"),
                "{path} is not an SVG ({} bytes)",
                bytes.len()
            );
        }
    }

    #[test]
    fn unknown_paths_are_none() {
        assert!(AgentAssets.load("icons/missing.svg").unwrap().is_none());
    }
}
