use crate::*;

impl Engine {
    /// The files a thread changed, compared as `scope` says.
    pub fn thread_diff_files(
        &self,
        thread_id: &str,
        scope: DiffScope,
    ) -> Result<Vec<DiffFile>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        diff::files(&cwd, scope).map_err(|detail| CommandError::Internal { detail })
    }

    pub fn thread_file_diff(
        &self,
        thread_id: &str,
        scope: DiffScope,
        file: &DiffFile,
    ) -> Result<FileDiff, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        diff::file(&cwd, scope, file).map_err(|detail| CommandError::Internal { detail })
    }

    /// Takes the thread back to before `message_id`: the transcript from there on goes,
    /// and the next turn resumes the conversation as it was. With `revert_files`, the
    /// working tree goes back to how it was before that message's turn too.
    pub fn rewind_thread(
        &self,
        thread_id: &str,
        message_id: &str,
        revert_files: bool,
    ) -> Result<(), CommandError> {
        let cannot = || CommandError::CannotRewind {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
        };
        let rewind_files = |cwd: &str| {
            let before = checkpoint::reference(thread_id, message_id, Moment::Before);
            if !checkpoint::exists(cwd, &before) || !checkpoint::taken_in(cwd, &before) {
                return Err(CommandError::NoCheckpoint {
                    thread_id: thread_id.to_owned(),
                    message_id: message_id.to_owned(),
                });
            }
            checkpoint::restore(cwd, &before).map_err(|detail| CommandError::GitFailed { detail })
        };
        let mut state = lock(&self.state);
        let thread = state
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
        let item = state
            .store
            .thread_item(thread_id, message_id)
            .map_err(internal)?
            .ok_or_else(cannot)?;
        let ItemContent::UserMessage {
            rewind: Some(rewind),
            ..
        } = item.content
        else {
            return Err(cannot());
        };
        let command = Command::ThreadRewind {
            thread_id: thread_id.to_owned(),
            message_id: message_id.to_owned(),
            rewind,
        };
        if !revert_files {
            return state.decide_and_commit(command);
        }
        let cwd = state
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if let Some(busy) = state.working_in(&cwd, thread_id) {
            return Err(CommandError::ThreadBusy { thread_id: busy });
        }
        let op = state.begin_op(thread_id, &[&cwd])?;
        let checkpoints = state.checkpoints.clone();
        drop(state);
        checkpoints.settle(&cwd);
        let reverted = rewind_files(&cwd);
        let mut state = lock(&self.state);
        let rewound = reverted.and_then(|()| state.decide_and_commit(command));
        state.end_op(op, None)?;
        rewound
    }

    /// The files a finished turn changed, read from its checkpoints.
    pub fn turn_diff_files(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Vec<DiffFile>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let before = checkpoint::reference(thread_id, turn_id, Moment::Before);
        let after = checkpoint::reference(thread_id, turn_id, Moment::After);
        checkpoint::changes(&cwd, &before, &after)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// How one file listed in a turn's changes changed during that turn.
    pub fn turn_file_diff(
        &self,
        thread_id: &str,
        turn_id: &str,
        file: &DiffFile,
    ) -> Result<FileDiff, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let before = checkpoint::reference(thread_id, turn_id, Moment::Before);
        let after = checkpoint::reference(thread_id, turn_id, Moment::After);
        checkpoint::file_diff(&cwd, &before, &after, file)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// The code editors installed on this machine; found once, then answered at once.
    pub fn editors(&self) -> Vec<Editor> {
        editors::installed()
    }

    /// The local branches of a thread's repository.
    pub fn thread_branches(&self, thread_id: &str) -> Result<Vec<String>, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        Ok(git::local_branches(&cwd))
    }

    /// What the thread's working tree changed and has not committed.
    pub fn thread_changes(&self, thread_id: &str) -> Result<ChangesSummary, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let files = diff::files(&cwd, DiffScope::Uncommitted)
            .map_err(|detail| CommandError::Internal { detail })?;
        Ok(ChangesSummary {
            files: files.len() as u32,
            additions: files.iter().filter_map(|file| file.additions).sum(),
            deletions: files.iter().filter_map(|file| file.deletions).sum(),
        })
    }

    /// Where the thread's agent runs; for a chat, its own folder.
    pub fn thread_folder(&self, thread_id: &str) -> Option<String> {
        lock(&self.state).cwd(thread_id)
    }

    pub fn open_in_editor(&self, thread_id: &str, editor: Editor) -> Result<(), CommandError> {
        let cwd = lock(&self.state)
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        editors::open(editor, &cwd).map_err(|detail| CommandError::Internal { detail })
    }

    /// Opens a project's folder, for a thread not started yet.
    pub fn open_project_in_editor(
        &self,
        project_id: &str,
        editor: Editor,
    ) -> Result<(), CommandError> {
        let root = lock(&self.state)
            .model
            .project(project_id)
            .map(|project| project.workspace_root.clone())
            .ok_or_else(|| CommandError::ProjectNotFound {
                project_id: project_id.to_owned(),
            })?;
        if !Path::new(&root).is_dir() {
            return Err(CommandError::WorkspaceRootNotFound { path: root });
        }
        editors::open(editor, &root).map_err(|detail| CommandError::Internal { detail })
    }

    /// Checks out another local branch in the thread's folder. Refused while a turn runs,
    /// so the agent never finds its branch changed under it.
    pub fn switch_branch(&self, thread_id: &str, branch: &str) -> Result<(), CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        let op = {
            let mut state = lock(&self.state);
            let working = state
                .model
                .thread(thread_id)
                .is_some_and(|thread| thread.status == ThreadStatus::Working);
            if working {
                return Err(CommandError::ThreadBusy {
                    thread_id: thread_id.to_owned(),
                });
            }
            state.begin_op(thread_id, &[&cwd])?
        };
        let switched =
            git::switch(&cwd, branch).map_err(|detail| CommandError::GitFailed { detail });
        lock(&self.state).end_op(op, None)?;
        switched?;
        let state = Arc::downgrade(&self.state);
        read_branches(&state, &[thread_id.to_owned()]);
        let project_id = lock(&self.state)
            .model
            .thread(thread_id)
            .filter(|thread| thread.worktree_path.is_none())
            .and_then(|thread| thread.project_id.clone());
        if let Some(project_id) = project_id {
            read_root_branches(&state, &[project_id]);
        }
        Ok(())
    }

    pub(crate) fn repository_cwd(&self, thread_id: &str) -> Result<String, CommandError> {
        let cwd = lock(&self.state)
            .cwd(thread_id)
            .ok_or_else(|| CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            })?;
        if !git::is_repository(&cwd) {
            return Err(CommandError::NotAGitRepository { path: cwd });
        }
        Ok(cwd)
    }
}
