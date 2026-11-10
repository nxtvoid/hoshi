//! Whether Claude Code defers MCP tools behind its ToolSearch tool, as the user's settings
//! and environment set `ENABLE_TOOL_SEARCH`.

use crate::claude_install;
use serde_json::Value;
use std::path::Path;

const KEY: &str = "ENABLE_TOOL_SEARCH";

/// How Claude Code offers MCP tools to the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ToolSearch {
    /// Behind ToolSearch, Claude Code's default.
    #[default]
    Deferred,
    /// Deferred only past a share of the context, which hoshi's few tools never reach: their
    /// schemas would go with every request.
    Auto,
    /// Never deferred, as the user chose.
    Off,
}

/// The value the first source that sets it gives, sources from the highest precedence down.
pub(crate) fn resolve(sources: &[Option<&str>]) -> ToolSearch {
    sources
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map_or(ToolSearch::Deferred, parse)
}

fn parse(value: &str) -> ToolSearch {
    let value = value.to_lowercase();
    if let Some(rest) = value.strip_prefix("auto") {
        let threshold = rest
            .strip_prefix(':')
            .and_then(|n| n.trim().parse::<u32>().ok());
        return match threshold {
            Some(percent) if percent >= 100 => ToolSearch::Off,
            _ => ToolSearch::Auto,
        };
    }
    match value.as_str() {
        "false" | "0" | "no" | "off" => ToolSearch::Off,
        _ => ToolSearch::Deferred,
    }
}

/// How a session started in `cwd` offers MCP tools: its local and project settings, the
/// user's, then the environment, as Claude Code reads them.
pub(crate) fn effective(cwd: &Path) -> ToolSearch {
    let project = cwd.join(".claude");
    let files = [
        Some(project.join("settings.local.json")),
        Some(project.join("settings.json")),
        claude_install::config_dir().map(|dir| dir.join("settings.json")),
    ];
    let mut sources: Vec<Option<String>> = files
        .iter()
        .map(|file| file.as_deref().and_then(setting))
        .collect();
    sources.push(std::env::var(KEY).ok());
    let sources: Vec<Option<&str>> = sources.iter().map(Option::as_deref).collect();
    resolve(&sources)
}

fn setting(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let settings: Value = serde_json::from_str(&text).ok()?;
    match &settings["env"][KEY] {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highest_source_that_sets_it_wins() {
        assert_eq!(resolve(&[None, None, None]), ToolSearch::Deferred);
        assert_eq!(
            resolve(&[None, Some("auto"), Some("false")]),
            ToolSearch::Auto
        );
        assert_eq!(resolve(&[Some(" "), Some("false")]), ToolSearch::Off);
        assert_eq!(resolve(&[Some("true"), Some("auto")]), ToolSearch::Deferred);
    }

    #[test]
    fn auto_and_its_thresholds_are_told_apart_from_off() {
        assert_eq!(resolve(&[Some("auto:5")]), ToolSearch::Auto);
        assert_eq!(resolve(&[Some("AUTO:50")]), ToolSearch::Auto);
        assert_eq!(resolve(&[Some("auto:100")]), ToolSearch::Off);
        assert_eq!(resolve(&[Some("0")]), ToolSearch::Off);
        assert_eq!(resolve(&[Some("1")]), ToolSearch::Deferred);
    }

    #[test]
    fn a_settings_file_s_env_is_read_as_text() {
        let dir = std::env::temp_dir().join(format!("hoshi-tool-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{ "env": { "ENABLE_TOOL_SEARCH": false } }"#).unwrap();
        assert_eq!(setting(&file).as_deref(), Some("false"));
        std::fs::write(&file, r#"{ "env": {} }"#).unwrap();
        assert_eq!(setting(&file), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
