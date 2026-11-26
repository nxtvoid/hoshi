//! Commit messages and pull request texts from a short, tool-less Claude Code run.

use super::titles::run_quick;
use crate::writing::{commit_prompt, parse_commit, parse_pull_request, pull_request_prompt};
use crate::{CommitMessage, CommitMessageRequest, PullRequestText, PullRequestTextRequest};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(120);
const MODEL: &str = "haiku";

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

pub(crate) fn write_commit(binary: &Path, request: &CommitMessageRequest) -> Option<CommitMessage> {
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
    binary: &Path,
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
