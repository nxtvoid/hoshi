//! Thread titles from a short, tool-less Claude Code run on a small model.

use crate::claude_models::manifest;
use crate::process::hide_console;
use crate::{GeneratedTitle, TitleRequest};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(90);
/// The quick model titles use unless the user picked another.
const DEFAULT_MODEL: &str = "haiku";
const MAX_CONTEXT_CHARS: usize = 8_000;
const TRUNCATED: &str = "[Earlier content truncated]\n\n";

const INITIAL_PROMPT: &str = "Generate a title that will help the user recognize this hoshi thread weeks later.
Return JSON with keys title and needsRefinement.
Set needsRefinement to true only if the subject is still unknown, such as an unresolved link, \"fix this\", or an unexplained attachment. Otherwise set it to false.

Before answering, silently reduce the request to:
- Subject: What system, feature, or problem is this really about?
- Outcome: What does the user ultimately want to understand or change?
- Incidental instructions: What only describes how the agent should do the work?

Title the subject and outcome. Discard incidental instructions.

Editorial rules:
- 3-8 words, fewer than 40 characters.
- Use a compact noun phrase or clear action phrase.
- Capture the umbrella goal when the request lists several symptoms or steps.
- Name the product change, not the mock, plan, report, branch, or PR used to produce it.
- Models, subagents, tools, output formats, and monitoring instructions do not belong in the title unless they are themselves the topic.
- For reviews, name what is being reviewed and the relevant concern.
- For research, name the question domain rather than the requested research process.
- Do not claim the work is complete.
- Do not copy and truncate the user's message.
- Avoid project names, quotes, labels, filler, and trailing punctuation.
- Use attached images as primary context for UI issues.
- Write the title in the language the user wrote in.";

const REGENERATE_PROMPT: &str = "Regenerate the title for an existing hoshi thread so the user can recognize it weeks later.
The previous title was {previous}.
Return JSON with keys title and needsRefinement. Set needsRefinement to false.

Determine the title in this order:
1. Read the USER messages first. Identify the latest explicit durable goal. The original subject remains the subject until the user clearly changes what the thread is about.
2. Use ASSISTANT messages to resolve vague links, unnamed code, and discovered product nouns. Do not promote one assistant finding into the thread subject unless the user adopts it as a new goal.
3. Compare that subject with the previous title. Preserve accurate scope words. Replace the previous title when it is generic, artifact-based, a completion update, or contradicted by the thread.
4. Title the durable subject and desired outcome, not the current workflow state.

Editorial rules:
- 3-8 words, fewer than 40 characters.
- Use a compact noun phrase or clear action phrase.
- Preserve the umbrella subject when later messages focus on one finding or implementation detail.
- Ignore deliverables and operations such as mocks, plans, branches, PRs, tests, CI, commits, merging, and monitoring unless they are the actual topic.
- Treat final operational follow-ups and assistant completion summaries as weak evidence of subject.
- Do not claim the work is complete.
- Do not copy and truncate a thread message.
- Avoid project names, PR numbers, quotes, labels, filler, and trailing punctuation.
- Write the title in the language the user wrote in.
- Keep the previous title unchanged if it is already accurate. Otherwise return a meaningfully improved title, not a cosmetic paraphrase.";

/// What the model must answer with.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "needsRefinement": { "type": "boolean" },
        },
        "required": ["title", "needsRefinement"],
        "additionalProperties": false,
    })
}

pub(crate) fn prompt(request: &TitleRequest) -> String {
    let mut prompt = match &request.previous_title {
        None => format!(
            "{INITIAL_PROMPT}\n\nUser message:\n{}",
            keep_start(&request.context, MAX_CONTEXT_CHARS)
        ),
        Some(previous) => format!(
            "{}\n\nThread contents:\n{}",
            REGENERATE_PROMPT.replace("{previous}", &json!(previous).to_string()),
            keep_end(&request.context, MAX_CONTEXT_CHARS)
        ),
    };
    let instructions = request.instructions.trim();
    if !instructions.is_empty() {
        prompt.push_str("\n\nAdditional instructions:\n");
        prompt.push_str(instructions);
    }
    if !request.images.is_empty() {
        prompt.push_str("\n\nAttachment metadata:");
        for image in &request.images {
            prompt.push_str("\n- ");
            prompt.push_str(image);
        }
    }
    prompt
}

fn keep_start(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

fn keep_end(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let start = text
        .char_indices()
        .nth(count - max_chars)
        .map_or(0, |(index, _)| index);
    format!("{TRUNCATED}{}", &text[start..])
}

/// The title in the CLI's `--output-format json` result; `None` for anything else.
pub(crate) fn parse(output: &str) -> Option<GeneratedTitle> {
    let value: Value = serde_json::from_str(output.trim()).ok()?;
    if value["is_error"] == true {
        return None;
    }
    let answer = match &value["structured_output"] {
        Value::Object(_) => value["structured_output"].clone(),
        _ => serde_json::from_str(value["result"].as_str()?).ok()?,
    };
    let title = answer["title"].as_str()?.trim();
    let title = title
        .trim_end_matches(['.', '!', '?', ':'])
        .trim_matches('"');
    (!title.is_empty()).then(|| GeneratedTitle {
        title: title.to_owned(),
        needs_refinement: answer["needsRefinement"] == true,
    })
}

/// Runs the CLI outside any project, without tools, hooks or a saved session.
pub(crate) fn generate(binary: &str, request: &TitleRequest) -> Option<GeneratedTitle> {
    let model = manifest().resolve(
        Some(request.model.as_deref().unwrap_or(DEFAULT_MODEL)),
        Some(hoshi_core::Effort::Low),
        false,
        false,
    );
    let mut command = Command::new(binary);
    command
        .current_dir(std::env::temp_dir())
        .args(["-p", "--output-format", "json"])
        .args(["--json-schema", &schema().to_string()])
        .args(["--model", &model.slug])
        .args([
            "--settings",
            &json!({ "disableAllHooks": true }).to_string(),
        ])
        .args(["--tools", ""])
        .arg("--disable-slash-commands")
        .arg("--no-session-persistence")
        .args(["--permission-mode", "dontAsk"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(effort) = &model.effort {
        command.args(["--effort", effort]);
    }
    hide_console(&mut command);
    let mut child = command.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt(request).as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        let _ = stdout.read_to_string(&mut output);
        let _ = sender.send(output);
    });
    let output = receiver.recv_timeout(TIMEOUT).ok();
    let _ = child.kill();
    let _ = child.wait();
    parse(&output?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(previous_title: Option<&str>) -> TitleRequest {
        TitleRequest {
            context: "fix the login redirect".into(),
            previous_title: previous_title.map(Into::into),
            images: vec!["shot.png".into()],
            model: None,
            instructions: "  Use Spanish.  ".into(),
        }
    }

    #[test]
    fn the_first_title_reads_the_message_and_the_user_instructions() {
        let prompt = prompt(&request(None));

        assert!(prompt.starts_with("Generate a title"));
        assert!(prompt.contains("User message:\nfix the login redirect"));
        assert!(prompt.contains("Additional instructions:\nUse Spanish."));
        assert!(prompt.contains("Attachment metadata:\n- shot.png"));
    }

    #[test]
    fn regenerating_names_the_previous_title_and_keeps_the_latest_messages() {
        let mut request = request(Some("Login \"bug\""));
        request.context = format!("{}USER:\nlatest ask", "x".repeat(MAX_CONTEXT_CHARS));

        let prompt = prompt(&request);

        assert!(prompt.contains(r#"The previous title was "Login \"bug\"""#));
        assert!(prompt.contains("Thread contents:\n[Earlier content truncated]"));
        assert!(prompt.contains("USER:\nlatest ask"));
    }

    #[test]
    fn the_result_carries_the_structured_title() {
        let output = r#"{"type":"result","is_error":false,"result":"{}","structured_output":{"title":"Empty TSX Snippet.","needsRefinement":true}}"#;

        assert_eq!(
            parse(output),
            Some(GeneratedTitle {
                title: "Empty TSX Snippet".into(),
                needs_refinement: true
            })
        );
    }

    #[test]
    fn a_result_without_structure_falls_back_to_its_text_and_errors_have_none() {
        let text = r#"{"type":"result","is_error":false,"result":"{\"title\":\"Login Redirect\",\"needsRefinement\":false}"}"#;
        let failed = r#"{"type":"result","is_error":true,"result":"rate limited"}"#;

        assert_eq!(
            parse(text).map(|title| title.title),
            Some("Login Redirect".into())
        );
        assert_eq!(parse(failed), None);
        assert_eq!(parse("not json"), None);
    }
}
