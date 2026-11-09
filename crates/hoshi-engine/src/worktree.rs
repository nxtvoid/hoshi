use std::path::Path;
use std::process::{Command, Stdio};

/// What a turn saw of its repository, to tell which worktree it created.
#[derive(Clone)]
pub struct TurnWorkspace {
    pub repository: String,
    pub worktrees_before: Vec<String>,
    /// Inputs of the turn's tool calls, the evidence when several turns ran at once.
    pub tool_inputs: Vec<String>,
    /// The agent reported moving its session, which says where it went better than guessing.
    pub moved: bool,
}

const MAX_TOOL_INPUTS: usize = 200;

impl TurnWorkspace {
    pub fn record_tool_input(&mut self, input: &str) {
        if self.tool_inputs.len() < MAX_TOOL_INPUTS {
            self.tool_inputs.push(input.to_lowercase());
        }
    }
}

/// The worktree a turn moved to: one that appeared since the turn started and no
/// other thread owns. A worktree its tool calls named wins; without one, a single new
/// worktree counts only if no other turn of the project was running.
pub fn created_by_turn(
    workspace: &TurnWorkspace,
    worktrees_after: &[String],
    claimed: &[String],
    alone: bool,
) -> Option<String> {
    let new: Vec<&String> = worktrees_after
        .iter()
        .filter(|path| !workspace.worktrees_before.contains(path) && !claimed.contains(path))
        .collect();
    let named: Vec<&String> = new
        .iter()
        .copied()
        .filter(|path| {
            let folder = folder_name(path);
            !folder.is_empty()
                && workspace
                    .tool_inputs
                    .iter()
                    .any(|input| input.contains(&folder))
        })
        .collect();
    match (named.as_slice(), new.as_slice()) {
        ([path], _) => Some((*path).clone()),
        ([], [path]) if alone => Some((*path).clone()),
        _ => None,
    }
}

const BRANCH_NAME_MAX_CHARS: usize = 40;

/// A branch name from a thread's title, as words joined by `-`: "Fix the login bug" gives
/// `fix-the-login-bug`. `None` when no word is left, as for a title in another script.
pub fn branch_name(title: &str) -> Option<String> {
    let plain: String = title
        .chars()
        .map(|c| match c.to_lowercase().next().unwrap_or(c) {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    let mut name = String::new();
    for word in plain.split_whitespace() {
        let next = if name.is_empty() { 0 } else { 1 } + word.len();
        if !name.is_empty() && name.len() + next > BRANCH_NAME_MAX_CHARS {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(word);
    }
    name.truncate(BRANCH_NAME_MAX_CHARS);
    let name = name.trim_matches('-').to_owned();
    (!name.is_empty()).then_some(name)
}

/// The process an agent named when it locked a worktree, as Claude Code does:
/// `claude session <name> (pid 21452)`.
pub fn locking_pid(reason: &str) -> Option<u32> {
    let start = reason.rfind("(pid ")? + "(pid ".len();
    let rest = &reason[start..];
    rest[..rest.find(')')?].trim().parse().ok()
}

/// A lock left by an agent whose process is gone, which nothing holds anymore.
pub fn is_stale_agent_lock(reason: &str) -> bool {
    locking_pid(reason).is_some_and(|pid| !process_running(pid))
}

fn process_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let filter = format!("PID eq {pid}");
        Command::new("tasklist")
            .args(["/FI", &filter, "/NH", "/FO", "CSV"])
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(true)
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(true)
    }
}

fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_becomes_a_short_branch_name() {
        assert_eq!(
            branch_name("Fix the login bug").as_deref(),
            Some("fix-the-login-bug")
        );
        assert_eq!(
            branch_name("Arreglar el diseño: botón \"Guardar\"").as_deref(),
            Some("arreglar-el-diseno-boton-guardar")
        );
        assert_eq!(
            branch_name("Add CSV export to the reports page with filters and totals").as_deref(),
            Some("add-csv-export-to-the-reports-page-with")
        );
        assert_eq!(branch_name("修正"), None);
    }

    #[test]
    fn reads_the_pid_an_agent_left_in_its_lock() {
        assert_eq!(
            locking_pid("claude session add-readme (pid 21452)"),
            Some(21452)
        );
        assert_eq!(locking_pid("kept by hand"), None);
        assert_eq!(locking_pid("(pid x)"), None);
        assert!(!is_stale_agent_lock("kept by hand"));
        assert!(!is_stale_agent_lock(&format!(
            "claude session x (pid {})",
            std::process::id()
        )));
    }

    fn workspace(before: &[&str], tool_inputs: &[&str]) -> TurnWorkspace {
        let mut workspace = TurnWorkspace {
            repository: "/repo".into(),
            worktrees_before: before.iter().map(|path| path.to_string()).collect(),
            tool_inputs: Vec::new(),
            moved: false,
        };
        for input in tool_inputs {
            workspace.record_tool_input(input);
        }
        workspace
    }

    fn paths(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn a_single_new_worktree_belongs_to_a_turn_that_ran_alone() {
        let workspace = workspace(&["/repo"], &[]);
        let after = paths(&["/repo", "/wt/csv-export"]);

        assert_eq!(
            created_by_turn(&workspace, &after, &[], true).as_deref(),
            Some("/wt/csv-export")
        );
        assert_eq!(created_by_turn(&workspace, &after, &[], false), None);
    }

    #[test]
    fn the_worktree_named_in_a_tool_call_wins_over_others() {
        let workspace = workspace(
            &["/repo"],
            &[r#"{"command":"git worktree add ../CSV-Export -b feat/csv"}"#],
        );
        let after = paths(&["/repo", "/wt/csv-export", "/wt/other"]);

        assert_eq!(
            created_by_turn(&workspace, &after, &[], false).as_deref(),
            Some("/wt/csv-export")
        );
    }

    #[test]
    fn worktrees_owned_by_other_threads_are_ignored() {
        let workspace = workspace(&["/repo"], &[]);
        let after = paths(&["/repo", "/wt/a", "/wt/b"]);

        assert_eq!(
            created_by_turn(&workspace, &after, &paths(&["/wt/a"]), true).as_deref(),
            Some("/wt/b")
        );
    }

    #[test]
    fn several_unnamed_worktrees_stay_unassigned() {
        let workspace = workspace(&["/repo"], &[]);
        let after = paths(&["/repo", "/wt/a", "/wt/b"]);

        assert_eq!(created_by_turn(&workspace, &after, &[], true), None);
    }
}
