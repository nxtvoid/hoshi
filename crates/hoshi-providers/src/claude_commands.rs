//! The slash commands and skills Claude Code offers in a folder.

use crate::claude_control;
use hoshi_core::SlashCommand;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;

const REQUEST_ID: &str = "hoshi-commands";

/// Asks a CLI started in `cwd`, so the project's own commands and skills are included.
pub(crate) fn probe(binary: &str, cwd: &Path) -> Option<Vec<SlashCommand>> {
    let request = json!({ "subtype": "initialize" });
    let init = claude_control::ask(binary, cwd, REQUEST_ID, request)?;
    Some(commands(&init))
}

/// The commands of an `initialize` answer, without the CLI's internal ones.
fn commands(init: &Value) -> Vec<SlashCommand> {
    let mut seen = HashSet::new();
    let text = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    init["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| {
            let name = text(&command["name"])?;
            if name.starts_with("__") || !seen.insert(name.to_lowercase()) {
                return None;
            }
            Some(SlashCommand {
                name,
                description: text(&command["description"]).unwrap_or_default(),
                argument_hint: text(&command["argumentHint"]),
                aliases: command["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(text)
                    .collect(),
                custom: command["builtin"] != true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIALIZE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"hoshi-commands","response":{"commands":[{"name":"code-review","description":"Review the current diff","argumentHint":"[low|high] [--fix]","builtin":true,"aliases":["review"]},{"name":"init","description":"Initialize a new CLAUDE.md file","argumentHint":"","builtin":true},{"name":"__remote-workflow","description":"Internal","argumentHint":""},{"name":"pdf","description":"Work with PDF files","argumentHint":"","aliases":["anthropic-skills:pdf"]},{"name":"Init","description":"A duplicate"},{"name":"  ","description":"No name"}],"models":[]}}}"#;

    #[test]
    fn initialize_lists_the_commands_without_internal_or_duplicate_ones() {
        let init = claude_control::response(INITIALIZE, REQUEST_ID).unwrap();

        assert_eq!(
            commands(&init),
            [
                SlashCommand {
                    name: "code-review".into(),
                    description: "Review the current diff".into(),
                    argument_hint: Some("[low|high] [--fix]".into()),
                    aliases: vec!["review".into()],
                    custom: false,
                },
                SlashCommand {
                    name: "init".into(),
                    description: "Initialize a new CLAUDE.md file".into(),
                    argument_hint: None,
                    aliases: Vec::new(),
                    custom: false,
                },
                SlashCommand {
                    name: "pdf".into(),
                    description: "Work with PDF files".into(),
                    argument_hint: None,
                    aliases: vec!["anthropic-skills:pdf".into()],
                    custom: true,
                },
            ]
        );
    }

    #[test]
    fn an_answer_to_another_request_is_ignored() {
        let other = INITIALIZE.replace("hoshi-commands", "hoshi-usage");

        assert_eq!(claude_control::response(&other, REQUEST_ID), None);
    }
}
