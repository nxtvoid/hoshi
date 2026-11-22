use crate::*;

impl Engine {
    pub fn set_thread_titles(&self, settings: ThreadTitleSettings) {
        lock(&self.state).thread_titles = settings;
    }

    /// Who uses the browser and how its pages open. Agents running now keep the tools they
    /// were given until their session starts again; they are refused once access is off.
    pub fn set_browser(&self, settings: BrowserSettings) {
        lock(&self.state).browser = settings.clone();
        *hold(&self.browser_settings) = settings;
    }

    /// How agents choose the model and effort of their subagents, from their next turn on.
    pub fn set_subagent_effort(&self, policy: SubagentEffort) {
        lock(&self.state).subagent_effort = policy;
    }

    /// When idle threads with a large context compact. Threads already idle under another
    /// mode wait for the user.
    pub fn set_idle_compaction(&self, mode: IdleCompaction) {
        let mut state = lock(&self.state);
        state.idle_compaction = mode;
        if mode != IdleCompaction::BeforeCacheExpires {
            state.idle_compactions.clear();
        }
    }

    /// Names the thread again from its whole conversation, in the background. The
    /// title only changes if the user did not rename it meanwhile.
    /// Whether threads a restart cut short go on by themselves; see
    /// [`Engine::continue_cut_threads`].
    pub fn set_continue_after_restart(&self, on: bool) {
        lock(&self.state).recovery.continue_after_restart = on;
    }

    /// Whether a thread whose turn failed on its plan's usage limit goes on by itself once the
    /// limit resets.
    pub fn set_resume_after_usage_limit(&self, on: bool) {
        lock(&self.state).recovery.resume_after_usage_limit = on;
    }
}
