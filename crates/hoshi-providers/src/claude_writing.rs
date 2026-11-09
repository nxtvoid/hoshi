//! Commit messages and pull request texts from a short, tool-less Claude Code run.

use crate::claude_titles::{answer, run_quick};
use crate::{CommitMessage, CommitMessageRequest, PullRequestText, PullRequestTextRequest};
use serde_json::{Value, json};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(120);
const MODEL: &str = "haiku";
const MAX_SUBJECT_CHARS: usize = 72;
const MAX_FILES_CHARS: usize = 6_000;
const MAX_PATCH_CHARS: usize = 40_000;
const MAX_LOG_CHARS: usize = 12_000;
const MAX_TEMPLATE_CHARS: usize = 8_000;

const STYLE: &str = "Follow the style of the repository's recent commit subjects listed below: \
their prefixes, casing, scope and language. When they show no clear convention, start with a \
Conventional Commits prefix such as feat, fix, chore, refactor, docs, test or perf.";

const COMMIT_PROMPT: &str = "You write concise git commit messages.
Return JSON with keys subject and body{branch_key}.
- subject: imperative mood, at most 72 characters, no trailing period.
- body: empty, or a few short \"- \" bullets only when the change needs explaining.
- Never add co-author trailers, signatures or any attribution.{branch_rule}";

const BRANCH_RULE: &str = "
- branch: a short kebab-case name for a branch holding this change, such as fix-login-redirect, without any prefix.";

const PULL_REQUEST_PROMPT: &str = "You write pull request titles and descriptions.
Return JSON with keys title and body.
- title: like a commit subject in the repository's style, at most 72 characters, no trailing period.
- body: Markdown. Without a template, a \"## Summary\" section of short bullets on what changed and why, then a \"## Testing\" section saying what was run, or \"Not run\".
- With a template, fill it in instead, keeping its headings.
- Never add signatures or any attribution.";

fn commit_schema(wants_branch: bool) -> Value {
    let mut properties = json!({
        "subject": { "type": "string" },
        "body": { "type": "string" },
    });
    let mut required = vec!["subject", "body"];
    if wants_branch {
        properties["branch"] = json!({ "type": "string" });
        required.push("branch");
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn pull_request_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "body": { "type": "string" },
        },
        "required": ["title", "body"],
        "additionalProperties": false,
    })
}

fn recent(subjects: &[String]) -> String {
    if subjects.is_empty() {
        return "(none yet)".to_owned();
    }
    subjects
        .iter()
        .map(|subject| format!("- {subject}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn commit_prompt(request: &CommitMessageRequest) -> String {
    let (branch_key, branch_rule) = if request.wants_branch {
        (" and branch", BRANCH_RULE)
    } else {
        ("", "")
    };
    format!(
        "{}\n\n{STYLE}\n\nRecent commit subjects:\n{}\n\nBranch: {}\n\nStaged files:\n{}\n\nStaged patch:\n{}",
        COMMIT_PROMPT
            .replace("{branch_key}", branch_key)
            .replace("{branch_rule}", branch_rule),
        recent(&request.recent_subjects),
        request.branch,
        cut(&request.files, MAX_FILES_CHARS),
        cut(&request.patch, MAX_PATCH_CHARS),
    )
}

pub(crate) fn pull_request_prompt(request: &PullRequestTextRequest) -> String {
    let mut prompt = format!(
        "{PULL_REQUEST_PROMPT}\n\n{STYLE}\n\nRecent commit subjects:\n{}\n\nBase branch: {}\nHead branch: {}\n\nCommits:\n{}\n\nDiff stat:\n{}\n\nPatch:\n{}",
        recent(&request.recent_subjects),
        request.base,
        request.head,
        cut(&request.commits, MAX_LOG_CHARS),
        cut(&request.stat, MAX_LOG_CHARS),
        cut(&request.patch, MAX_PATCH_CHARS),
    );
    if let Some(template) = &request.template {
        prompt.push_str("\n\nPull request template:\n");
        prompt.push_str(&cut(template, MAX_TEMPLATE_CHARS));
    }
    prompt
}

fn cut(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}\n[truncated]", &text[..index]),
        None => text.to_owned(),
    }
}

/// The first line, without a trailing period, at most [`MAX_SUBJECT_CHARS`].
fn subject_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.trim_end_matches('.').trim();
    let line: String = line.chars().take(MAX_SUBJECT_CHARS).collect();
    (!line.is_empty()).then(|| line.trim_end().to_owned())
}

pub(crate) fn parse_commit(output: &str) -> Option<CommitMessage> {
    let answer = answer(output)?;
    Some(CommitMessage {
        subject: subject_line(answer["subject"].as_str()?)?,
        body: answer["body"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        branch: answer["branch"]
            .as_str()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .map(str::to_owned),
    })
}

pub(crate) fn parse_pull_request(output: &str) -> Option<PullRequestText> {
    let answer = answer(output)?;
    Some(PullRequestText {
        title: subject_line(answer["title"].as_str()?)?,
        body: answer["body"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned(),
    })
}

pub(crate) fn write_commit(binary: &str, request: &CommitMessageRequest) -> Option<CommitMessage> {
    let schema = commit_schema(request.wants_branch);
    parse_commit(&run_quick(
        binary,
        MODEL,
        &schema,
        &commit_prompt(request),
        TIMEOUT,
    )?)
}

pub(crate) fn write_pull_request(
    binary: &str,
    request: &PullRequestTextRequest,
) -> Option<PullRequestText> {
    parse_pull_request(&run_quick(
        binary,
        MODEL,
        &pull_request_schema(),
        &pull_request_prompt(request),
        TIMEOUT,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit_request(wants_branch: bool) -> CommitMessageRequest {
        CommitMessageRequest {
            branch: "hoshi/login".into(),
            files: "M\tsrc/login.ts".into(),
            patch: "+fix".into(),
            recent_subjects: vec!["fix: keep the session".into(), "feat: add export".into()],
            wants_branch,
        }
    }

    #[test]
    fn the_commit_prompt_shows_the_repository_s_style_and_the_staged_change() {
        let prompt = commit_prompt(&commit_request(false));

        assert!(prompt.contains("- fix: keep the session\n- feat: add export"));
        assert!(prompt.contains("Conventional Commits"));
        assert!(prompt.contains("Staged files:\nM\tsrc/login.ts"));
        assert!(!prompt.contains("branch:"));
        assert!(commit_prompt(&commit_request(true)).contains("- branch: a short kebab-case"));
    }

    #[test]
    fn a_commit_answer_keeps_one_short_subject_line() {
        let long = "x".repeat(100);
        let output = json!({
            "type": "result",
            "is_error": false,
            "structured_output": {
                "subject": "fix: stop the login loop.\nextra",
                "body": "  - keep the token  ",
                "branch": "fix-login-loop",
            },
        })
        .to_string();
        let long_output = json!({
            "type": "result",
            "is_error": false,
            "structured_output": { "subject": long, "body": "" },
        })
        .to_string();

        assert_eq!(
            parse_commit(&output),
            Some(CommitMessage {
                subject: "fix: stop the login loop".into(),
                body: "- keep the token".into(),
                branch: Some("fix-login-loop".into()),
            })
        );
        let cut = parse_commit(&long_output).unwrap();
        assert_eq!(cut.subject.chars().count(), MAX_SUBJECT_CHARS);
        assert_eq!(cut.branch, None);
    }

    #[test]
    fn the_pull_request_prompt_carries_the_range_and_its_template() {
        let request = PullRequestTextRequest {
            base: "main".into(),
            head: "hoshi/login".into(),
            commits: "abc fix: stop the login loop".into(),
            stat: " src/login.ts | 4 ++--".into(),
            patch: "+fix".into(),
            template: Some("## What\n## Why".into()),
            recent_subjects: Vec::new(),
        };

        let prompt = pull_request_prompt(&request);

        assert!(prompt.contains("Base branch: main\nHead branch: hoshi/login"));
        assert!(prompt.contains("Recent commit subjects:\n(none yet)"));
        assert!(prompt.ends_with("Pull request template:\n## What\n## Why"));
    }
}
