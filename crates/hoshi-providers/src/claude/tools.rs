//! What Claude Code's tools did, read from the structured result the CLI sends with each
//! tool result (`tool_use_result`).

use crate::truncate;
use hoshi_core::{Task, TaskStatus, ToolDetail};
use serde_json::Value;
use std::path::Path;

/// A diff stays readable in a transcript row; the file itself has the rest.
const DIFF_MAX_CHARS: usize = 32_000;

pub(crate) fn detail(result: &Value) -> Option<ToolDetail> {
    file_change(result)
}

/// Edit, MultiEdit and Write report their change as hunks; a new file only as its content.
fn file_change(result: &Value) -> Option<ToolDetail> {
    let path = result["filePath"].as_str()?;
    let hunks = result["structuredPatch"].as_array();
    let mut diff = String::new();
    let (mut additions, mut deletions) = (0, 0);
    match hunks.filter(|hunks| !hunks.is_empty()) {
        Some(hunks) => {
            for hunk in hunks {
                let number = |key: &str| hunk[key].as_u64().unwrap_or(0);
                diff.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    number("oldStart"),
                    number("oldLines"),
                    number("newStart"),
                    number("newLines")
                ));
                for line in hunk["lines"].as_array().into_iter().flatten() {
                    let line = line.as_str().unwrap_or_default();
                    match line.chars().next() {
                        Some('+') => additions += 1,
                        Some('-') => deletions += 1,
                        _ => {}
                    }
                    diff.push_str(line);
                    diff.push('\n');
                }
            }
        }
        None if result["type"] == "create" => {
            let content = result["content"].as_str()?;
            let lines: Vec<_> = content.lines().collect();
            additions = lines.len() as u32;
            diff.push_str(&format!("@@ -0,0 +1,{additions} @@\n"));
            for line in lines {
                diff.push('+');
                diff.push_str(line);
                diff.push('\n');
            }
        }
        None => return None,
    }
    Some(ToolDetail::FileChange {
        path: path.to_owned(),
        additions,
        deletions,
        diff: Some(truncate(diff.trim_end_matches('\n'), DIFF_MAX_CHARS)),
    })
}

/// The list `TodoWrite` sets, which older Claude Code versions use.
pub(crate) fn todo_list(input: &Value) -> Option<Vec<Task>> {
    let todos = input["todos"].as_array()?;
    Some(
        todos
            .iter()
            .filter_map(|todo| {
                Some(Task {
                    subject: todo["content"].as_str()?.to_owned(),
                    active_form: todo["activeForm"].as_str().map(str::to_owned),
                    status: task_status(&todo["status"]),
                })
            })
            .collect(),
    )
}

/// The list `TaskCreate` and `TaskUpdate` keep in `dir`, one numbered JSON file per task.
pub(crate) fn saved_task_list(dir: &Path) -> Option<Vec<Task>> {
    let mut files: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let number = path.file_stem()?.to_str()?.parse().ok()?;
            (path.extension()? == "json").then_some((number, path))
        })
        .collect();
    files.sort();
    Some(
        files
            .into_iter()
            .filter_map(|(_, path)| {
                let task: Value =
                    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
                if task["metadata"]["_internal"] == true {
                    return None;
                }
                Some(Task {
                    subject: task["subject"].as_str()?.to_owned(),
                    active_form: task["activeForm"].as_str().map(str::to_owned),
                    status: task_status(&task["status"]),
                })
            })
            .collect(),
    )
}

fn task_status(status: &Value) -> TaskStatus {
    match status.as_str() {
        Some("in_progress") => TaskStatus::InProgress,
        Some("completed") => TaskStatus::Completed,
        _ => TaskStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_edit_becomes_its_hunks() {
        let result = json!({
            "filePath": "/repo/notes.txt",
            "oldString": "two",
            "newString": "TWO",
            "structuredPatch": [{ "oldStart": 1, "oldLines": 3, "newStart": 1, "newLines": 3,
                                  "lines": [" one", "-two", "+TWO", " three"] }],
        });

        assert_eq!(
            detail(&result),
            Some(ToolDetail::FileChange {
                path: "/repo/notes.txt".into(),
                additions: 1,
                deletions: 1,
                diff: Some("@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three".into()),
            })
        );
    }

    #[test]
    fn a_new_file_is_all_additions() {
        let result = json!({
            "type": "create",
            "filePath": "/repo/notes.txt",
            "content": "one\ntwo\n",
            "structuredPatch": [],
        });

        assert_eq!(
            detail(&result),
            Some(ToolDetail::FileChange {
                path: "/repo/notes.txt".into(),
                additions: 2,
                deletions: 0,
                diff: Some("@@ -0,0 +1,2 @@\n+one\n+two".into()),
            })
        );
    }

    #[test]
    fn the_saved_task_list_reads_in_number_order() {
        let dir = std::env::temp_dir().join(format!("hoshi-tasks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, task: Value| {
            std::fs::write(dir.join(name), task.to_string()).unwrap();
        };
        write("10.json", json!({ "subject": "Ship", "status": "pending" }));
        write(
            "2.json",
            json!({ "subject": "Test", "activeForm": "Testing", "status": "in_progress" }),
        );
        write(
            "1.json",
            json!({ "subject": "Write", "status": "completed" }),
        );
        write(
            "3.json",
            json!({ "subject": "Hidden", "status": "pending", "metadata": { "_internal": true } }),
        );
        write("notes.txt", json!({}));

        let tasks = saved_task_list(&dir).unwrap();

        let subjects: Vec<_> = tasks.iter().map(|task| task.subject.as_str()).collect();
        assert_eq!(subjects, ["Write", "Test", "Ship"]);
        assert_eq!(tasks[1].status, TaskStatus::InProgress);
        assert_eq!(tasks[1].active_form.as_deref(), Some("Testing"));
        assert_eq!(saved_task_list(&dir.join("missing")), None);
    }

    #[test]
    fn todo_write_sets_the_whole_list() {
        let input = json!({ "todos": [
            { "content": "Write", "activeForm": "Writing", "status": "completed" },
            { "content": "Test", "activeForm": "Testing", "status": "pending" }
        ] });

        let tasks = todo_list(&input).unwrap();

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(tasks[1].active_form.as_deref(), Some("Testing"));
    }

    #[test]
    fn other_results_have_no_detail() {
        let read = json!({ "type": "text", "file": { "filePath": "/repo/a.txt", "content": "x" } });
        assert_eq!(detail(&read), None);
        assert_eq!(detail(&json!({ "stdout": "hi", "stderr": "" })), None);
        assert_eq!(detail(&Value::Null), None);
    }
}
