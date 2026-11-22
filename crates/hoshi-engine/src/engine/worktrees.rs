use crate::*;

impl Engine {
    /// Removes the thread's worktree folder to free the disk and keeps its branch; the next
    /// message creates the folder again. Refused while it has uncommitted or untracked files,
    /// while the agent works, or while a terminal is open in it. Everything stays local.
    pub fn release_worktree(&self, thread_id: &str) -> Result<(), CommandError> {
        release_worktree(&self.state, &self.terminals, thread_id)
    }

    /// Removes now the worktree folders [`merged_worktrees`] would, and tells what it did.
    pub fn delete_merged_worktrees(&self) -> MergedCleanup {
        merged_worktrees::sweep(&self.state, &self.terminals)
    }

    /// What the last sweep of merged worktrees did in this run, if one ran.
    pub fn merged_cleanup(&self) -> Option<MergedCleanup> {
        lock(&self.state).merged_worktrees.last()
    }

    /// The worktree folders of threads whose pull request merged are removed by themselves.
    pub fn set_delete_merged_worktrees(&self, on: bool) {
        lock(&self.state).merged_worktrees.set(on);
    }

    /// Once the thread's pull request is merged: removes its worktree and its local branch,
    /// and archives it. A thread in the project root on that branch first switches the root
    /// to the pull request's base. Refused when anything would be lost: uncommitted files,
    /// or commits that neither a remote branch nor the pull request holds. The forge is never
    /// touched; deleting the branch there is the forge's own business.
    pub fn clean_up_thread(&self, thread_id: &str) -> Result<(), CommandError> {
        if self.terminals.any_open_for(thread_id) {
            return Err(CommandError::TerminalsOpen {
                thread_id: thread_id.to_owned(),
            });
        }
        let switched = self.clean_up_locked(thread_id)?;
        if !switched.is_empty() {
            read_branches(&Arc::downgrade(&self.state), &switched);
        }
        Ok(())
    }

    /// The cleanup itself; returns the root threads whose branch changed under them. What
    /// git does runs outside the lock.
    pub(crate) fn clean_up_locked(&self, thread_id: &str) -> Result<Vec<String>, CommandError> {
        let (op, plan, checkpoints) = {
            let mut state = lock(&self.state);
            let (thread, root) = state.removable(thread_id)?;
            let thread = thread.clone();
            let pull_request = state
                .pull_requests
                .get(thread_id)
                .filter(|pull_request| pull_request.state == hoshi_core::PullRequestState::Merged)
                .cloned()
                .ok_or_else(|| CommandError::PullRequestNotMerged {
                    thread_id: thread_id.to_owned(),
                })?;
            let worktree = thread
                .worktree_path
                .clone()
                .filter(|_| !thread.worktree_released);
            // Only the branch the pull request merged; one the thread moved to since stays.
            let branch = thread
                .branch
                .clone()
                .filter(|branch| *branch == pull_request.head_branch)
                .and_then(|branch| Some((git::branch_tip(&root, &branch)?, branch)));
            let in_root = thread.worktree_path.is_none()
                && branch.is_some()
                && git::current_branch(&root).as_deref() == Some(pull_request.head_branch.as_str());
            let sharing_root = if in_root {
                state.root_threads_sharing(&thread)?
            } else {
                Vec::new()
            };
            let mut folders: Vec<&str> = worktree.as_deref().into_iter().collect();
            if in_root {
                folders.push(&root);
            }
            let op = state.begin_op(thread_id, &folders)?;
            state.close_lingering(thread_id);
            let plan = CleanUpPlan {
                root,
                pull_request,
                worktree,
                branch,
                in_root,
                sharing_root,
            };
            (op, plan, state.checkpoints.clone())
        };
        let cleaned = plan.run(thread_id, &checkpoints);
        let mut state = lock(&self.state);
        let committed = match cleaned {
            Ok(()) => state.decide_and_commit(Command::ThreadCleanUp {
                thread_id: thread_id.to_owned(),
            }),
            Err((worktree_removed, error)) => {
                if worktree_removed
                    && let Err(released) = state.decide_and_commit(Command::ThreadWorktreeRelease {
                        thread_id: thread_id.to_owned(),
                    })
                {
                    eprintln!(
                        "hoshi: removed the worktree of {thread_id} but didn't record it: {released}"
                    );
                }
                Err(error)
            }
        };
        let ended = state.end_op(op, None);
        drop(state);
        if committed.is_err() && !plan.sharing_root.is_empty() {
            read_branches(&Arc::downgrade(&self.state), &plan.sharing_root);
        }
        ended?;
        committed.map(|()| plan.sharing_root)
    }

    /// Creates a new thread's worktree outside the lock, then the thread in it. Messages
    /// sent to the thread meanwhile wait for it.
    pub(crate) fn create_worktree_thread(&self, command: Command) -> Result<(), CommandError> {
        let Command::ThreadCreate {
            thread_id,
            project_id,
            title,
            workspace,
            provider,
            model,
            effort,
            fast_mode,
            ultracode,
            access_mode,
            plan_mode,
        } = command
        else {
            return self.dispatch(command);
        };
        let existing = match workspace {
            ThreadWorkspace::BranchWorktree { branch } => Some(branch),
            _ => None,
        };
        let (op, plan) = {
            let mut state = lock(&self.state);
            let Some(project) = project_id.as_deref() else {
                return Err(CommandError::ChatNotLocal { thread_id });
            };
            let plan = state.plan_worktree(&thread_id, project, existing.as_deref())?;
            let path = plan.path.to_string_lossy().into_owned();
            let op = state.begin_op(&thread_id, &[&path])?;
            (op, plan)
        };
        let created = plan.create();
        let mut state = lock(&self.state);
        let workspace = match created {
            Ok(workspace) => workspace,
            Err(error) => {
                state.end_op(op, None)?;
                return Err(error);
            }
        };
        let create = Command::ThreadCreate {
            thread_id: thread_id.clone(),
            project_id,
            title,
            workspace,
            provider,
            model,
            effort,
            fast_mode,
            ultracode,
            access_mode,
            plan_mode,
        };
        let result = state.run_prepared(create);
        match &result {
            Ok(()) => state.start_worktree_setup(&thread_id),
            Err(_) => {
                thread::spawn(move || plan.discard());
            }
        }
        state.end_op(op, None)?;
        result
    }

    /// Every worktree setup since the app started, running or ended.
    pub fn worktree_setups(&self) -> Vec<hoshi_core::WorktreeSetupRun> {
        lock(&self.state).setups.runs()
    }

    /// Runs the thread's worktree setup again, with the project's command as it is now.
    pub fn rerun_worktree_setup(&self, thread_id: &str) -> Result<(), CommandError> {
        let state = lock(&self.state);
        let launch = state
            .setup_launch(thread_id)
            .or_else(|| state.setups.launch(thread_id))
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        state.run_worktree_setup(launch);
        Ok(())
    }

    /// Lets the agent start without the thread's setup, sending what it held.
    pub fn skip_worktree_setup(&self, thread_id: &str) -> Result<(), CommandError> {
        let mut state = lock(&self.state);
        for command in state.setups.stop_waiting(thread_id) {
            state.run_command(command)?;
        }
        Ok(())
    }
}
