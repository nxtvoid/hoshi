//! Removes the worktree folders of threads whose pull request merged, while the settings ask
//! for it: soon after it is turned on, and every hour. Only the folder goes; the branch stays
//! and the next message makes the folder again. Whatever a release refuses is kept: local
//! files, a working agent, an open terminal, a shared or adopted worktree. So is a branch that
//! moved past the commit the pull request merged. The last sweep's report stays for the settings.

use crate::terminal::Terminals;
use crate::{Notification, State, git, lock, release_worktree};
use hoshi_core::{CommandError, PullRequestState};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
const CHECK_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub(crate) struct MergedWorktrees {
    on: bool,
    /// A sweep is due at the next check, as when the setting was just turned on.
    requested: bool,
    last: Option<MergedCleanup>,
}

/// Why a sweep kept a merged worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KeptReason {
    /// Uncommitted or untracked files.
    Changes,
    /// Its agent is working.
    Working,
    /// A terminal is open in it.
    Terminal,
    /// Another thread works in it, or someone locked it.
    InUse,
    /// The branch has commits after the one the pull request merged.
    NewCommits,
    /// Git failed.
    Failed,
}

impl KeptReason {
    fn of(error: &CommandError) -> Self {
        match error {
            CommandError::WorktreeDirty { .. } => Self::Changes,
            CommandError::ThreadBusy { .. } => Self::Working,
            CommandError::TerminalsOpen { .. } => Self::Terminal,
            CommandError::WorktreeShared { .. }
            | CommandError::WorktreeLocked { .. }
            | CommandError::WorktreeAdopted { .. } => Self::InUse,
            _ => Self::Failed,
        }
    }
}

/// What a sweep did, by thread id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedCleanup {
    pub at: SystemTime,
    pub removed: Vec<String>,
    pub kept: Vec<(String, KeptReason)>,
}

impl MergedWorktrees {
    pub(crate) fn set(&mut self, on: bool) {
        if on && !self.on {
            self.requested = true;
        }
        self.on = on;
    }

    /// Whether to sweep now, given when the last sweep ran.
    fn due(&mut self, last: Option<Instant>) -> bool {
        if !self.on {
            self.requested = false;
            return false;
        }
        let due = self.requested || last.is_none_or(|last| last.elapsed() >= SWEEP_EVERY);
        self.requested = false;
        due
    }

    pub(crate) fn last(&self) -> Option<MergedCleanup> {
        self.last.clone()
    }

    /// The last sweep, as a previous run left it.
    pub(crate) fn restore(&mut self, last: Option<MergedCleanup>) {
        self.last = last;
    }
}

/// Checks every little while whether a sweep is due, until the engine closes.
pub(crate) fn watch(state: Weak<Mutex<State>>, terminals: Weak<Terminals>) {
    thread::spawn(move || {
        let mut last = None;
        loop {
            thread::sleep(CHECK_EVERY);
            let (Some(shared), Some(terminals)) = (state.upgrade(), terminals.upgrade()) else {
                return;
            };
            if lock(&shared).merged_worktrees.due(last) {
                last = Some(Instant::now());
                sweep(&shared, &terminals);
            }
        }
    });
}

/// The threads with a worktree folder of hoshi's own and a merged pull request, with their
/// project root, branch and the commit the pull request merged.
fn candidates(state: &State) -> Vec<(String, String, String, String)> {
    state
        .model
        .threads()
        .filter(|thread| !thread.archived)
        .filter(|thread| {
            thread.worktree_path.is_some() && !thread.worktree_released && !thread.worktree_adopted
        })
        .filter_map(|thread| {
            let pull_request = state.pull_requests.get(&thread.id)?;
            let merged = pull_request.state == PullRequestState::Merged
                && thread.branch.as_deref() == Some(pull_request.head_branch.as_str());
            let head = pull_request.head_sha.clone().filter(|_| merged)?;
            let root = state.project_root(thread).ok()?;
            Some((
                thread.id.clone(),
                root,
                pull_request.head_branch.clone(),
                head,
            ))
        })
        .collect()
}

/// Removes every merged worktree folder that can go now, and keeps what it did.
pub(crate) fn sweep(shared: &Arc<Mutex<State>>, terminals: &Terminals) -> MergedCleanup {
    let candidates = candidates(&lock(shared));
    let mut cleanup = MergedCleanup {
        at: SystemTime::now(),
        removed: Vec::new(),
        kept: Vec::new(),
    };
    for (thread_id, root, branch, head) in candidates {
        if git::branch_tip(&root, &branch).as_deref() != Some(head.as_str()) {
            cleanup.kept.push((thread_id, KeptReason::NewCommits));
            continue;
        }
        match release_worktree(shared, terminals, &thread_id) {
            Ok(()) => cleanup.removed.push(thread_id),
            Err(error) => {
                let reason = KeptReason::of(&error);
                if reason == KeptReason::Failed {
                    eprintln!("hoshi: kept the merged worktree of {thread_id}: {error}");
                }
                cleanup.kept.push((thread_id, reason));
            }
        }
    }
    let mut state = lock(shared);
    if let Err(error) = state.store.save_merged_cleanup(&cleanup) {
        eprintln!("hoshi: failed to keep the merged worktree cleanup: {error}");
    }
    state.merged_worktrees.last = Some(cleanup.clone());
    (state.notify)(Notification::MergedCleanup(cleanup.clone()));
    cleanup
}

#[cfg(test)]
mod tests {
    use super::MergedWorktrees;
    use std::time::{Duration, Instant};

    #[test]
    fn a_sweep_runs_once_turned_on_then_every_hour() {
        let mut sweeps = MergedWorktrees::default();
        assert!(!sweeps.due(None), "off by default");

        sweeps.set(true);
        let just_now = Some(Instant::now());
        assert!(sweeps.due(just_now), "turning it on sweeps at once");
        assert!(!sweeps.due(just_now));

        let long_ago = Instant::now().checked_sub(Duration::from_secs(2 * 60 * 60));
        assert!(sweeps.due(long_ago));

        sweeps.set(false);
        assert!(!sweeps.due(long_ago));
    }
}
