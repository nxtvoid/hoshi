use std::path::{Path, PathBuf};
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

/// Checks out an existing `branch` in a new worktree at `path`: the local branch when there
/// is one, otherwise origin's, fetched first and tracked.
pub fn add_branch_worktree(repository: &str, path: &Path, branch: &str) -> Result<(), String> {
    let path = path.to_string_lossy();
    let _ = stdout(repository, &["worktree", "prune"]);
    let local = format!("refs/heads/{branch}");
    if stdout(repository, &["rev-parse", "--verify", "--quiet", &local]).is_ok() {
        return stdout(repository, &["worktree", "add", &path, branch]).map(|_| ());
    }
    let remote = format!("refs/remotes/origin/{branch}");
    stdout(
        repository,
        &["fetch", "origin", &format!("+refs/heads/{branch}:{remote}")],
    )?;
    stdout(
        repository,
        &["worktree", "add", "--track", "-b", branch, &path, &remote],
    )
    .map(|_| ())
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

/// Every worktree of the repository at `dir`, the main one included, as normalized paths.
pub fn worktrees(dir: &str) -> Option<Vec<String>> {
    let output = git(dir, &["worktree", "list", "--porcelain"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(normalize)
        .collect();
    Some(paths)
}

/// The local branches of the repository at `dir`, by name.
pub fn local_branches(dir: &str) -> Vec<String> {
    stdout(dir, &["branch", "--format=%(refname:short)"])
        .map(|output| {
            output
                .lines()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Checks out an existing `branch` in a new worktree at `path`, as it was before it was
/// removed.
pub fn restore_worktree(repository: &str, path: &str, branch: &str) -> Result<(), String> {
    // A folder deleted by hand leaves an entry that would block the path.
    let _ = stdout(repository, &["worktree", "prune"]);
    stdout(repository, &["worktree", "add", path, branch]).map(|_| ())
}

/// Why the worktree at `path` is locked: `Some` with git's reason (maybe empty) when it is.
pub fn lock_reason(repository: &str, path: &str) -> Option<String> {
    let listing = stdout(repository, &["worktree", "list", "--porcelain"]).ok()?;
    let wanted = normalize(path);
    listing.split("\n\n").find_map(|entry| {
        let mut lines = entry.lines();
        let worktree = lines.next()?.strip_prefix("worktree ")?;
        if normalize(worktree) != wanted {
            return None;
        }
        lines.find_map(|line| {
            (line == "locked")
                .then(String::new)
                .or_else(|| line.strip_prefix("locked ").map(str::to_owned))
        })
    })
}

pub fn unlock_worktree(repository: &str, path: &str) -> Result<(), String> {
    stdout(repository, &["worktree", "unlock", path]).map(|_| ())
}

/// Forgets worktrees whose folder is gone.
pub fn prune_worktrees(repository: &str) {
    let _ = stdout(repository, &["worktree", "prune"]);
}

/// Whether `dir` has no uncommitted changes and no untracked files.
pub fn is_clean(dir: &str) -> Result<bool, String> {
    stdout(dir, &["status", "--porcelain", "--untracked-files=normal"])
        .map(|output| output.trim().is_empty())
}

/// The commit a local branch points to; `None` when there is no such branch.
pub fn branch_tip(repository: &str, branch: &str) -> Option<String> {
    let reference = format!("refs/heads/{branch}");
    stdout(
        repository,
        &["rev-parse", "--verify", "--quiet", &reference],
    )
    .ok()
    .map(|output| output.trim().to_owned())
    .filter(|sha| !sha.is_empty())
}

/// Whether some remote-tracking branch already holds `commit`.
pub fn on_a_remote(repository: &str, commit: &str) -> bool {
    stdout(repository, &["branch", "--remotes", "--contains", commit])
        .is_ok_and(|output| !output.trim().is_empty())
}

/// Whether `ancestor` is `descendant` or one of its parents. False when either is unknown here.
pub fn is_ancestor(repository: &str, ancestor: &str, descendant: &str) -> bool {
    stdout(
        repository,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )
    .is_ok()
}

/// Deletes a local branch, merged or not. Remote branches are never touched.
pub fn delete_branch(repository: &str, branch: &str) -> Result<(), String> {
    stdout(repository, &["branch", "-D", "--", branch]).map(|_| ())
}

/// Checks out `branch` in `dir`; git refuses when uncommitted changes would be lost.
pub fn switch(dir: &str, branch: &str) -> Result<(), String> {
    stdout(dir, &["switch", branch]).map(|_| ())
}

/// Where `origin` points, as configured.
pub fn origin_url(dir: &str) -> Option<String> {
    let output = git(dir, &["remote", "get-url", "origin"]).ok()?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !url.is_empty()).then_some(url)
}

/// The `.git` folder shared by the repository at `dir` and all its worktrees.
pub fn common_dir(dir: &str) -> Option<PathBuf> {
    let output = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !path.is_empty()).then(|| PathBuf::from(normalize(&path)))
}

/// Git prints forward slashes on Windows; paths compare equal once canonical.
pub fn normalize(path: &str) -> String {
    dunce::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned())
}

/// Runs git and returns what it printed; fails with git's own message.
pub fn stdout(dir: &str, args: &[&str]) -> Result<String, String> {
    run(dir, args, &[0])
}

/// For `git diff --no-index`, which exits with 1 when the files differ.
pub fn stdout_allowing_differences(dir: &str, args: &[&str]) -> Result<String, String> {
    run(dir, args, &[0, 1])
}

fn run(dir: &str, args: &[&str], success_codes: &[i32]) -> Result<String, String> {
    let output = git(dir, args).map_err(|error| error.to_string())?;
    if output
        .status
        .code()
        .is_some_and(|code| success_codes.contains(&code))
    {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Makes `dir` a repository; a repository already there stays as it is.
pub fn init(dir: &str) -> Result<(), String> {
    stdout(dir, &["init"]).map(|_| ())
}

/// Clones `url` into `destination`, whose parent must exist. Never asks for credentials
/// on a terminal: a repository that needs them and has none fails with git's message.
pub fn clone(url: &str, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", destination.display()))?;
    let mut command = command(&parent.to_string_lossy(), &["clone", "--", url]);
    command.arg(destination).env("GIT_TERMINAL_PROMPT", "0");
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn git(dir: &str, args: &[&str]) -> std::io::Result<Output> {
    command(dir, args).output()
}

fn command(dir: &str, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir).args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
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
        assert_eq!(
            worktrees(&repo_str).unwrap(),
            vec![repo_str.clone(), normalize(&worktree_str)]
        );
        assert_eq!(common_dir(&worktree_str), Some(repo.join(".git")));
        remove_worktree(&repo_str, &worktree_str).unwrap();

        assert!(!worktree.exists());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn lists_local_branches_and_switches_between_them() {
        let repo = temp_repository("switch");
        let dir = repo.to_string_lossy().into_owned();
        stdout(&dir, &["branch", "feat/x"]).unwrap();

        assert_eq!(local_branches(&dir), ["feat/x", "main"]);
        switch(&dir, "feat/x").unwrap();
        assert_eq!(current_branch(&dir).as_deref(), Some("feat/x"));
        assert!(switch(&dir, "missing").is_err());
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
