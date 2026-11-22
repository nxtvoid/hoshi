use crate::*;

impl State {
    /// Creates again the worktree a released thread had, before a message runs in it.
    /// Left to the git work that runs in its folder, if any, which starts the turn after.
    pub(crate) fn restore_released_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        let released = self.model.thread(thread_id).is_some_and(|thread| {
            thread.worktree_released
                && !thread
                    .worktree_path
                    .as_deref()
                    .is_some_and(|path| self.folder_ops.contains(&git::normalize(path)))
        });
        if !released {
            return Ok(());
        }
        self.restore_worktree(thread_id)
    }

    /// Creates the thread's worktree again from its branch when its folder is gone.
    pub(crate) fn restore_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        let Some(thread) = self.model.thread(thread_id) else {
            return Ok(());
        };
        let Some(path) = thread
            .worktree_path
            .as_deref()
            .filter(|_| !thread.worktree_adopted)
        else {
            return Ok(());
        };
        if Path::new(path).is_dir() {
            return Ok(());
        }
        let failed = |detail: String| CommandError::WorktreeFailed { detail };
        let branch = thread
            .branch
            .as_deref()
            .ok_or_else(|| failed("the thread has no branch to check out".into()))?;
        let root = self.project_root(thread)?;
        git::restore_worktree(&root, path, branch).map_err(failed)
    }

    /// The thread, its project root, and whether it is free to have its workspace removed.
    pub(crate) fn removable(
        &self,
        thread_id: &str,
    ) -> Result<(&hoshi_core::Thread, String), CommandError> {
        let thread = self
            .model
            .thread(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if thread.active_turn_id.is_some() {
            return Err(CommandError::ThreadBusy {
                thread_id: thread_id.to_owned(),
            });
        }
        if thread.worktree_adopted {
            return Err(CommandError::WorktreeAdopted {
                thread_id: thread_id.to_owned(),
            });
        }
        if let Some(other) = self.sharing_worktree(thread, &[]) {
            return Err(CommandError::WorktreeShared {
                thread_id: thread_id.to_owned(),
                other_thread_id: other.to_owned(),
            });
        }
        let root = self.project_root(thread)?;
        Ok((thread, root))
    }

    /// Another thread that works in `thread`'s worktree as its own, such as a fork, besides
    /// those in `leaving`.
    pub(crate) fn sharing_worktree(
        &self,
        thread: &hoshi_core::Thread,
        leaving: &[String],
    ) -> Option<&str> {
        let path = git::normalize(thread.worktree_path.as_deref()?);
        self.model
            .threads()
            .filter(|other| other.id != thread.id && !other.worktree_adopted)
            .filter(|other| !leaving.contains(&other.id))
            .find(|other| {
                other
                    .worktree_path
                    .as_deref()
                    .is_some_and(|other| git::normalize(other) == path)
            })
            .map(|other| other.id.as_str())
    }

    /// The other threads that run in `thread`'s project root, and so change branch with it.
    /// Refused while one of them works.
    pub(crate) fn root_threads_sharing(
        &self,
        thread: &hoshi_core::Thread,
    ) -> Result<Vec<String>, CommandError> {
        let sharing = self.model.threads().filter(|other| {
            other.id != thread.id
                && other.project_id == thread.project_id
                && other.worktree_path.is_none()
        });
        let mut ids = Vec::new();
        for other in sharing {
            if other.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy {
                    thread_id: other.id.clone(),
                });
            }
            ids.push(other.id.clone());
        }
        Ok(ids)
    }

    /// A thread other than `thread_id` whose turn runs in `folder`.
    pub(crate) fn working_in(&self, folder: &str, thread_id: &str) -> Option<String> {
        let folder = git::normalize(folder);
        self.model
            .threads()
            .filter(|other| other.id != thread_id && other.active_turn_id.is_some())
            .find(|other| {
                self.model
                    .thread_cwd(&other.id)
                    .is_some_and(|cwd| git::normalize(cwd) == folder)
            })
            .map(|other| other.id.clone())
    }

    /// The root of the thread's project; a chat has none, so it has no worktree either.
    pub(crate) fn project_root(&self, thread: &hoshi_core::Thread) -> Result<String, CommandError> {
        let project_id =
            thread
                .project_id
                .as_deref()
                .ok_or_else(|| CommandError::NothingToClean {
                    thread_id: thread.id.clone(),
                })?;
        self.model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
            .ok_or_else(|| CommandError::ProjectNotFound {
                project_id: project_id.to_owned(),
            })
    }

    /// Removes a worktree folder, refusing when that would lose uncommitted work. A folder
    /// already gone only leaves git's record of it, which is pruned.
    pub(crate) fn remove_clean_worktree(
        root: &str,
        path: &str,
        thread_id: &str,
    ) -> Result<(), CommandError> {
        if !Path::new(path).is_dir() {
            git::prune_worktrees(root);
            return Ok(());
        }
        let git_failed = |detail: String| CommandError::GitFailed { detail };
        if !git::is_clean(path).map_err(git_failed)? {
            return Err(CommandError::WorktreeDirty {
                thread_id: thread_id.to_owned(),
            });
        }
        if let Some(reason) = git::lock_reason(root, path) {
            // Claude Code locks the worktrees it enters and leaves the lock behind.
            if !worktree::is_stale_agent_lock(&reason) {
                return Err(CommandError::WorktreeLocked {
                    thread_id: thread_id.to_owned(),
                    reason,
                });
            }
            git::unlock_worktree(root, path).map_err(git_failed)?;
        }
        git::remove_worktree(root, path).map_err(git_failed)
    }

    /// A worktree for a new thread: on a new `hoshi/<id>` branch, or on `existing`.
    pub(crate) fn create_worktree(
        &self,
        thread_id: &str,
        project_id: &str,
        existing: Option<&str>,
    ) -> Result<ThreadWorkspace, CommandError> {
        self.plan_worktree(thread_id, project_id, existing)?
            .create()
    }

    /// Where a new thread's worktree goes; [`WorktreePlan::create`] makes it.
    pub(crate) fn plan_worktree(
        &self,
        thread_id: &str,
        project_id: &str,
        existing: Option<&str>,
    ) -> Result<WorktreePlan, CommandError> {
        let project =
            self.model
                .project(project_id)
                .ok_or_else(|| CommandError::ProjectNotFound {
                    project_id: project_id.to_owned(),
                })?;
        let root = project.workspace_root.clone();
        if !git::is_repository(&root) {
            return Err(CommandError::NotAGitRepository { path: root });
        }
        let short_id: String = thread_id.chars().take(8).collect();
        let branch = existing.map_or_else(|| format!("hoshi/{short_id}"), str::to_owned);
        let folder = format!(
            "{}-{}",
            slug(&project.title),
            project_id.chars().take(8).collect::<String>()
        );
        Ok(WorktreePlan {
            path: self.worktrees_dir.join(folder).join(&short_id),
            root,
            branch,
            existing: existing.is_some(),
        })
    }

    /// Whether the thread's worktree was released and its folder is gone.
    pub(crate) fn worktree_missing(&self, thread_id: &str) -> bool {
        self.model.thread(thread_id).is_some_and(|thread| {
            thread.worktree_released
                && thread
                    .worktree_path
                    .as_deref()
                    .is_some_and(|path| !Path::new(path).is_dir())
        })
    }

    /// Runs the project's setup in the thread's worktree, if it has one.
    pub(crate) fn start_worktree_setup(&self, thread_id: &str) {
        if let Some(launch) = self.setup_launch(thread_id) {
            self.run_worktree_setup(launch);
        }
    }

    pub(crate) fn setup_launch(&self, thread_id: &str) -> Option<SetupLaunch> {
        let thread = self.model.thread(thread_id)?;
        let worktree_path = thread.worktree_path.clone()?;
        let project = self.model.project(thread.project_id.as_deref()?)?;
        let setup = project.worktree_setup.clone()?;
        Some(SetupLaunch {
            thread_id: thread_id.to_owned(),
            project_root: project.workspace_root.clone(),
            worktree_path,
            files: setup.files,
            command: setup.command,
            wait: setup.wait,
        })
    }

    /// The messages a setup held are sent apart from its terminal, whose lock it ends under.
    pub(crate) fn run_worktree_setup(&self, launch: SetupLaunch) {
        let this = self.this.clone();
        self.setups.start(launch, move |held| {
            let this = this.clone();
            thread::spawn(move || {
                let Some(state) = this.upgrade() else {
                    return;
                };
                let mut state = lock(&state);
                for command in held {
                    if let Err(error) = state.run_command(command) {
                        eprintln!("hoshi: failed to send a message held for setup: {error}");
                    }
                }
            });
        })
    }

    /// Records the branch read in `cwd` off the lock. `false` when the thread moved to
    /// another folder meanwhile, so the read says nothing about where it is now.
    pub(crate) fn record_branch_read(
        &mut self,
        thread_id: &str,
        cwd: &str,
        branch: Option<String>,
    ) -> bool {
        if self.model.thread_cwd(thread_id) != Some(cwd) {
            return false;
        }
        // What git does there may have removed the folder this read found no branch in.
        if self.thread_ops.contains_key(thread_id) || self.folder_ops.contains(&git::normalize(cwd))
        {
            return true;
        }
        let events = record_branch(&self.model, thread_id, branch);
        if !events.is_empty() {
            self.pull_requests.check_soon(thread_id);
        }
        if let Err(error) = self.commit(&events, &now()) {
            eprintln!("hoshi: failed to record a branch: {error}");
        }
        true
    }

    pub(crate) fn worktree_check(
        &self,
        thread_id: &str,
        turn_id: &str,
        workspace: TurnWorkspace,
    ) -> WorktreeCheck {
        let claimed = self
            .model
            .threads()
            .filter(|thread| thread.id != thread_id)
            .filter_map(|thread| thread.worktree_path.clone())
            .collect();
        WorktreeCheck {
            turn_id: turn_id.to_owned(),
            workspace,
            claimed,
        }
    }
}
