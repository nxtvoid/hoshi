//! Where an agent CLI worked before, read from the session files it keeps.

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A folder the CLI ran sessions in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFolder {
    pub path: String,
    /// When a session there was last written.
    pub active_at: SystemTime,
    pub sessions: usize,
}

/// A session's first records name its folder; the rest is the conversation.
const PREFIX_BYTES: u64 = 64 * 1024;
/// Only the newest session files are read, so a long history stays quick to scan.
const MAX_SESSIONS: usize = 2000;
/// Codex keeps `sessions/<year>/<month>/<day>/`.
const CODEX_DEPTH: usize = 4;

/// Claude Code keeps each folder's sessions together in `projects/<folder>/`, so one of
/// them names the folder for all.
pub(crate) fn claude_folders(projects: &Path) -> Vec<SessionFolder> {
    let Ok(entries) = fs::read_dir(projects) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    for entry in entries.flatten() {
        let mut sessions = Vec::new();
        collect_files(&entry.path(), 0, &mut sessions);
        if sessions.is_empty() {
            continue;
        }
        sessions.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
        let Some(path) = sessions
            .iter()
            .take(3)
            .find_map(|(file, _)| json_field(&read_prefix(file), "cwd"))
        else {
            continue;
        };
        folders.push(SessionFolder {
            path,
            active_at: sessions[0].1,
            sessions: sessions.len(),
        });
    }
    folders
}

/// Codex writes one file per session, the folder in its first record. Its subagents' sessions
/// run where their parent did, so they are left out.
pub(crate) fn codex_folders(sessions: &Path) -> Vec<SessionFolder> {
    let mut files = Vec::new();
    collect_files(sessions, CODEX_DEPTH, &mut files);
    newest(&mut files);
    group(files.into_iter().filter_map(|(file, modified)| {
        let prefix = read_prefix(&file);
        let first = prefix.lines().next()?;
        if first.contains("\"subagent\":") || first.contains("\"subAgent\":") {
            return None;
        }
        Some((json_field(first, "cwd")?, modified))
    }))
}

/// Copilot keeps each session in `session-state/<id>/`, its folder in `workspace.yaml`.
pub(crate) fn copilot_folders(session_state: &Path) -> Vec<SessionFolder> {
    let Ok(entries) = fs::read_dir(session_state) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.path().join("workspace.yaml");
            let modified = fs::metadata(&file).and_then(|meta| meta.modified()).ok()?;
            Some((file, modified))
        })
        .collect();
    newest(&mut files);
    group(files.into_iter().filter_map(|(file, modified)| {
        let text = fs::read_to_string(file).ok()?;
        Some((yaml_cwd(&text)?, modified))
    }))
}

/// The `.jsonl` files in `dir`, and in its folders down to `depth` levels.
fn collect_files(dir: &Path, depth: usize, files: &mut Vec<(PathBuf, SystemTime)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth > 0 {
                collect_files(&path, depth - 1, files);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && let Ok(modified) = entry.metadata().and_then(|meta| meta.modified())
        {
            files.push((path, modified));
        }
    }
}

fn newest(files: &mut Vec<(PathBuf, SystemTime)>) {
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    files.truncate(MAX_SESSIONS);
}

fn group(sessions: impl Iterator<Item = (String, SystemTime)>) -> Vec<SessionFolder> {
    let mut folders: HashMap<String, SessionFolder> = HashMap::new();
    for (path, active_at) in sessions {
        let folder = folders
            .entry(path.clone())
            .or_insert_with(|| SessionFolder {
                path,
                active_at,
                sessions: 0,
            });
        folder.sessions += 1;
        folder.active_at = folder.active_at.max(active_at);
    }
    folders.into_values().collect()
}

fn read_prefix(file: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(file) {
        let _ = file.take(PREFIX_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The first string value of `"name"` in `text`, which may end mid-record. A key quoted inside
/// another string is escaped, so it never matches.
fn json_field(text: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\"");
    let mut from = 0;
    while let Some(found) = text[from..].find(&key) {
        let start = from + found;
        from = start + key.len();
        if text[..start].ends_with('\\') {
            continue;
        }
        let rest = text[from..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        if let Some(Ok(value)) = serde_json::Deserializer::from_str(rest)
            .into_iter::<String>()
            .next()
            && !value.trim().is_empty()
        {
            return Some(value);
        }
    }
    None
}

fn yaml_cwd(text: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("cwd:"))?
        .trim();
    let value = if value.starts_with('"') {
        serde_json::from_str::<String>(value).ok()?
    } else if let Some(quoted) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        quoted.replace("''", "'")
    } else {
        value.to_owned()
    };
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{claude_folders, codex_folders, copilot_folders, json_field, yaml_cwd};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hoshi-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_field_is_read_from_a_cut_record_but_never_from_inside_a_string() {
        assert_eq!(
            json_field(
                r#"{"text":"say \"cwd\":\"/nope\"","cwd" : "/code/app","rest":"cut"#,
                "cwd"
            ),
            Some("/code/app".to_owned())
        );
        assert_eq!(
            json_field(r#"{"cwd":"C:\\code\\app"}"#, "cwd"),
            Some(r"C:\code\app".to_owned())
        );
        assert_eq!(json_field(r#"{"cwd":""}"#, "cwd"), None);
        assert_eq!(json_field(r#"{"cwd":"/unterminated"#, "cwd"), None);
    }

    #[test]
    fn copilot_folders_come_from_plain_or_quoted_yaml() {
        assert_eq!(
            yaml_cwd("id: x\ncwd: C:\\code\\app\n").as_deref(),
            Some(r"C:\code\app")
        );
        assert_eq!(
            yaml_cwd("cwd: \"/code/my app\"\n").as_deref(),
            Some("/code/my app")
        );
        assert_eq!(yaml_cwd("cwd: '/it''s'\n").as_deref(), Some("/it's"));
        assert_eq!(yaml_cwd("id: x\n"), None);
    }

    #[test]
    fn claude_names_a_folder_once_for_all_its_sessions() {
        let dir = scratch();
        let folder = dir.join("projects").join("-code-app");
        write(
            &folder.join("a.jsonl"),
            "{\"type\":\"summary\"}\n{\"type\":\"user\",\"cwd\":\"/code/app\"}\n",
        );
        write(&folder.join("b.jsonl"), "{\"type\":\"summary\"}\n");
        write(
            &folder.join("a").join("subagents").join("agent-1.jsonl"),
            "{\"cwd\":\"/code/app\"}\n",
        );
        write(&dir.join("projects").join("empty").join("notes.txt"), "");

        let folders = claude_folders(&dir.join("projects"));
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/code/app");
        assert_eq!(folders[0].sessions, 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn codex_groups_sessions_by_folder_and_leaves_out_subagents() {
        let dir = scratch();
        let day = dir.join("sessions").join("2026").join("11").join("14");
        let meta = |cwd: &str, source: &str| {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"{cwd}\",\"source\":{source}}}}}\n"
            )
        };
        write(&day.join("rollout-1.jsonl"), &meta("/code/app", "\"cli\""));
        write(
            &day.join("rollout-2.jsonl"),
            &meta("/code/app", "\"vscode\""),
        );
        write(
            &day.join("rollout-3.jsonl"),
            &meta("/code/other", "\"cli\""),
        );
        write(
            &day.join("rollout-4.jsonl"),
            &meta("/code/child", "{\"subagent\":{\"parent\":\"x\"}}"),
        );

        let mut folders = codex_folders(&dir.join("sessions"));
        folders.sort_by(|a, b| a.path.cmp(&b.path));
        let found: Vec<_> = folders
            .iter()
            .map(|folder| (folder.path.as_str(), folder.sessions))
            .collect();
        assert_eq!(found, [("/code/app", 2), ("/code/other", 1)]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn copilot_reads_each_sessions_workspace() {
        let dir = scratch();
        let state = dir.join("session-state");
        write(&state.join("s1").join("workspace.yaml"), "cwd: /code/app\n");
        write(&state.join("s2").join("workspace.yaml"), "cwd: /code/app\n");
        write(&state.join("s3").join("events.jsonl"), "{}\n");

        let folders = copilot_folders(&state);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/code/app");
        assert_eq!(folders[0].sessions, 2);
        fs::remove_dir_all(dir).ok();
    }
}
