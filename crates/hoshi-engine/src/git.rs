use std::path::Path;
use std::process::{Command, Output, Stdio};

/// The branch checked out in `dir`, or `None` outside a repository or on a detached HEAD.
pub fn current_branch(dir: &str) -> Option<String> {
    let output = git(dir, &["branch", "--show-current"]).ok()?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !branch.is_empty()).then_some(branch)
}

pub fn is_repository(dir: &str) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| output.status.success())
}

/// Creates `branch` from the repository's current HEAD, checked out at `path`.
pub fn add_worktree(repository: &str, path: &Path, branch: &str) -> Result<(), String> {
    let path = path.to_string_lossy();
    let output = git(
        repository,
        &["worktree", "add", "-b", branch, &path, "HEAD"],
    )
    .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Removes a worktree that has no uncommitted changes. Git refuses otherwise,
/// which keeps unsaved work; the branch always stays.
pub fn remove_worktree(repository: &str, path: &str) -> Result<(), String> {
    let output =
        git(repository, &["worktree", "remove", path]).map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn git(dir: &str, args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(dir).args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.output()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throwaway repository on `main` with one commit.
    pub(crate) fn temp_repository(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hoshi-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        for args in [
            &["init", "-b", "main"][..],
            &["config", "user.email", "hoshi@example.com"],
            &["config", "user.name", "hoshi"],
            &["commit", "--allow-empty", "-m", "init"],
        ] {
            assert!(
                git(&dir_str, args).unwrap().status.success(),
                "git {args:?}"
            );
        }
        dunce::canonicalize(dir).unwrap()
    }

    #[test]
    fn reads_the_branch_and_manages_a_worktree() {
        let repo = temp_repository("worktree");
        let repo_str = repo.to_string_lossy().into_owned();
        let worktree = repo.with_extension("wt");
        let _ = std::fs::remove_dir_all(&worktree);

        assert!(is_repository(&repo_str));
        assert_eq!(current_branch(&repo_str).as_deref(), Some("main"));
        add_worktree(&repo_str, &worktree, "hoshi/test").unwrap();
        let worktree_str = worktree.to_string_lossy().into_owned();
        assert_eq!(current_branch(&worktree_str).as_deref(), Some("hoshi/test"));
        remove_worktree(&repo_str, &worktree_str).unwrap();

        assert!(!worktree.exists());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_plain_folder_has_no_branch() {
        let dir = std::env::temp_dir().join(format!("hoshi-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();

        assert!(!is_repository(&dir_str));
        assert_eq!(current_branch(&dir_str), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
