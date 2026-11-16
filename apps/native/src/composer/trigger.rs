//! What the cursor is typing: an `@path`, a `/command` the prompt starts with, a
//! `#pull-request` or a `$skill`.

use hoshi_core::SlashCommand;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    Path,
    Command,
    PullRequest,
    Skill,
}

/// The token being typed: its byte range in the text, from its sigil up to the cursor, and
/// what follows the sigil.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub range: Range<usize>,
    pub query: String,
}

fn pull_request_query(query: &str) -> bool {
    let mut chars = query.chars();
    match chars.next() {
        None => true,
        Some(first) => {
            first.is_alphanumeric()
                && chars.all(|char| char.is_alphanumeric() || char == '_' || char == '-')
        }
    }
}

/// Not `$5` or `$12.50`, which are prices.
fn skill_query(query: &str) -> bool {
    let mut chars = query.chars();
    match chars.next() {
        None => true,
        Some(first) => {
            first.is_ascii_alphabetic()
                && chars.all(|char| char.is_alphanumeric() || "_:-".contains(char))
        }
    }
}

/// The token the cursor ends, outside any chip.
pub fn detect_trigger(text: &str, cursor: usize, tokens: &[Range<usize>]) -> Option<Trigger> {
    let before = text.get(..cursor)?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, char)| char.is_whitespace())
        .map_or(0, |(index, char)| index + char.len_utf8());
    if tokens
        .iter()
        .any(|range| range.start < cursor && start < range.end)
    {
        return None;
    }
    let token = &before[start..];
    let mut chars = token.chars();
    let sigil = chars.next()?;
    let query = chars.as_str();
    let kind = match sigil {
        '@' => TriggerKind::Path,
        '/' if !query.contains('/') && before[..start].trim().is_empty() => TriggerKind::Command,
        '#' if pull_request_query(query) => TriggerKind::PullRequest,
        '$' if skill_query(query) => TriggerKind::Skill,
        _ => return None,
    };
    let query = match kind {
        TriggerKind::Path => query.strip_prefix('"').unwrap_or(query),
        _ => query,
    };
    Some(Trigger {
        kind,
        range: start..cursor,
        query: query.to_owned(),
    })
}

const BOUNDARIES: &[char] = &['-', '_', ':'];

/// Lower is better; `None` when the command does not match.
fn score(command: &SlashCommand, query: &str) -> Option<u32> {
    let name = command.name.to_lowercase();
    let aliases: Vec<String> = command
        .aliases
        .iter()
        .map(|alias| alias.to_lowercase())
        .collect();
    if name == query {
        return Some(0);
    }
    if aliases.iter().any(|alias| alias == query) {
        return Some(1);
    }
    if name.starts_with(query) {
        return Some(2);
    }
    if aliases.iter().any(|alias| alias.starts_with(query)) {
        return Some(3);
    }
    if let Some(at) = name.find(query).filter(|at| *at > 0) {
        let boundary = name[..at].ends_with(BOUNDARIES);
        return Some(if boundary { 4 } else { 5 });
    }
    if command.description.to_lowercase().contains(query) {
        return Some(20);
    }
    let mut wanted = query.chars().peekable();
    for char in name.chars() {
        if wanted.peek() == Some(&char) {
            wanted.next();
        }
    }
    wanted.peek().is_none().then_some(50)
}

/// The items whose command matches `query`, best first; all of them, in order, when it is
/// empty.
pub fn rank_commands<T>(
    items: Vec<T>,
    query: &str,
    command_of: impl Fn(&T) -> &SlashCommand,
) -> Vec<T> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return items;
    }
    let mut scored: Vec<(u32, usize, T)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            score(command_of(&item), &needle).map(|score| (score, index, item))
        })
        .collect();
    scored.sort_by_key(|(score, index, _)| (*score, *index));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::{Trigger, TriggerKind, detect_trigger, rank_commands};
    use hoshi_core::SlashCommand;

    fn trigger(kind: TriggerKind, range: std::ops::Range<usize>, query: &str) -> Option<Trigger> {
        Some(Trigger {
            kind,
            range,
            query: query.into(),
        })
    }

    #[test]
    fn each_sigil_opens_its_menu() {
        assert_eq!(
            detect_trigger("look at @src/ma", 15, &[]),
            trigger(TriggerKind::Path, 8..15, "src/ma")
        );
        assert_eq!(
            detect_trigger("@", 1, &[]),
            trigger(TriggerKind::Path, 0..1, "")
        );
        assert_eq!(
            detect_trigger("  /rev", 6, &[]),
            trigger(TriggerKind::Command, 2..6, "rev")
        );
        assert_eq!(
            detect_trigger("fix #12", 7, &[]),
            trigger(TriggerKind::PullRequest, 4..7, "12")
        );
        assert_eq!(
            detect_trigger("use $re", 7, &[]),
            trigger(TriggerKind::Skill, 4..7, "re")
        );
        assert_eq!(
            detect_trigger("@\"my", 4, &[]),
            trigger(TriggerKind::Path, 0..4, "my")
        );
    }

    #[test]
    fn emails_prices_paths_and_chips_open_nothing() {
        assert_eq!(detect_trigger("mail me@host", 12, &[]), None);
        assert_eq!(detect_trigger("@src done", 9, &[]), None);
        assert_eq!(detect_trigger("costs $5", 8, &[]), None);
        assert_eq!(detect_trigger("then /review", 12, &[]), None);
        assert_eq!(detect_trigger("/usr/bin", 8, &[]), None);
        assert_eq!(detect_trigger("see #a.b", 8, &[]), None);
        let chip = 0..12;
        assert_eq!(
            detect_trigger("@src/main.rs", 12, std::slice::from_ref(&chip)),
            None
        );
    }

    fn command(name: &str, description: &str, aliases: &[&str]) -> SlashCommand {
        SlashCommand {
            name: name.into(),
            description: description.into(),
            argument_hint: None,
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            custom: false,
        }
    }

    #[test]
    fn commands_rank_exact_then_prefix_then_inside_then_description() {
        let commands = vec![
            command("security-review", "", &[]),
            command("compact", "shrink the context", &[]),
            command("review", "", &[]),
            command("pr-comments", "review comments", &[]),
            command("clear", "", &["reset"]),
        ];
        let ranked = rank_commands(commands.clone(), "review", |command| command);
        let names: Vec<&str> = ranked.iter().map(|command| command.name.as_str()).collect();
        assert_eq!(names, ["review", "security-review", "pr-comments"]);

        let by_alias = rank_commands(commands.clone(), "res", |command| command);
        assert_eq!(by_alias[0].name, "clear");

        assert_eq!(rank_commands(commands, " ", |command| command).len(), 5);
    }
}
