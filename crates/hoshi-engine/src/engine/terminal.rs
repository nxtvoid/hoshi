use crate::*;

impl Engine {
    /// Shows a thread's terminal through `sink`, starting its shell in the thread's
    /// folder (its worktree when it has one) if it is not running.
    pub fn open_terminal(
        &self,
        thread_id: &str,
        terminal_id: &str,
        size: (u16, u16),
        sink: TerminalSink,
    ) -> Result<TerminalAttach, CommandError> {
        let (cwd, project_root, worktree_path) = {
            let state = lock(&self.state);
            let not_found = || CommandError::ThreadNotFound {
                thread_id: thread_id.to_owned(),
            };
            let thread = state.model.thread(thread_id).ok_or_else(not_found)?;
            let cwd = state.cwd(thread_id).ok_or_else(not_found)?;
            let project_root = thread
                .project_id
                .as_deref()
                .and_then(|project_id| state.model.project(project_id))
                .map_or_else(|| cwd.clone(), |project| project.workspace_root.clone());
            (cwd, project_root, thread.worktree_path.clone())
        };
        let launch = Launch {
            thread_id,
            cwd: &cwd,
            project_root: &project_root,
            worktree_path: worktree_path.as_deref(),
            command: None,
        };
        self.terminals.open(terminal_id, launch, size, sink)
    }

    /// Runs `command` in a shell in the home folder, shown through `sink` like a thread's
    /// terminal; the shell exits with the command.
    pub fn run_in_terminal(
        &self,
        terminal_id: &str,
        command: &str,
        size: (u16, u16),
        sink: TerminalSink,
    ) -> Result<TerminalAttach, CommandError> {
        let home = std::env::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .to_string_lossy()
            .into_owned();
        let launch = Launch {
            thread_id: "",
            cwd: &home,
            project_root: &home,
            worktree_path: None,
            command: Some(command),
        };
        self.terminals.open(terminal_id, launch, size, sink)
    }

    pub fn write_terminal(&self, terminal_id: &str, data: &str) -> Result<(), CommandError> {
        self.terminals.write(terminal_id, data)
    }

    pub fn resize_terminal(&self, terminal_id: &str, size: (u16, u16)) -> Result<(), CommandError> {
        self.terminals.resize(terminal_id, size)
    }

    pub fn detach_terminal(&self, terminal_id: &str) {
        self.terminals.detach(terminal_id);
    }

    pub fn close_terminal(&self, terminal_id: &str) {
        self.terminals.close(terminal_id);
    }

    /// The thread's servers now; while watched, changes arrive as notifications.
    pub fn watch_servers(&self, thread_id: &str) -> Vec<hoshi_core::LocalServer> {
        self.servers.watch(thread_id)
    }

    pub fn unwatch_servers(&self, thread_id: &str) {
        self.servers.unwatch(thread_id);
    }

    /// Ends the server on `port` and what its process started; never hoshi, an agent or
    /// a shell.
    pub fn stop_server(&self, thread_id: &str, pid: u32, port: u16) -> Result<(), CommandError> {
        self.servers.stop(thread_id, pid, port)
    }

    /// The agent ends a command it is told to stop, but not always what the command
    /// started: a server run through a script keeps listening. Once the agent has taken the
    /// command as stopped, the servers it started since go too; ending them earlier would
    /// have the agent take the command as failed and go on by itself.
    pub(crate) fn end_servers_of(&self, thread_id: String, task_id: String, since: u64) {
        let (state, servers) = (Arc::downgrade(&self.state), self.servers.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + STOPPED_WITHIN;
            loop {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if !lock(&state).runs_background_command(&thread_id, &task_id) {
                    break;
                }
                if Instant::now() >= deadline {
                    return;
                }
                drop(state);
                thread::sleep(STOPPED_POLL);
            }
            servers.stop_agent_servers_since(&thread_id, since);
        });
    }
}
