use crate::*;

impl State {
    /// Asks the provider for a title in the background, retrying a first title twice.
    /// It replaces `current` only if the thread still has that title by then.
    pub(crate) fn name_thread(
        &mut self,
        thread_id: &str,
        current: &str,
        mut request: TitleRequest,
    ) {
        let provider = self
            .thread_titles
            .provider
            .as_deref()
            .unwrap_or(DEFAULT_PROVIDER);
        let Some(adapter) = self.adapter_for(provider) else {
            return;
        };
        let forced = request.previous_title.is_some();
        if !self.thread_titles.generate && !forced {
            return;
        }
        request.model.clone_from(&self.thread_titles.model);
        request
            .instructions
            .clone_from(&self.thread_titles.instructions);
        let state = self.this.clone();
        let (thread_id, current) = (thread_id.to_owned(), current.to_owned());
        thread::spawn(move || {
            let attempts = if forced { 1 } else { TITLE_ATTEMPTS };
            let mut delay = TITLE_RETRY_DELAY;
            for attempt in 1..=attempts {
                if let Some(generated) = adapter.generate_title(&request) {
                    let Some(shared) = state.upgrade() else {
                        return;
                    };
                    let mut state = lock(&shared);
                    let events = record_title(&state.model, &thread_id, &current, &generated.title);
                    if let Err(error) = state.commit(&events, &now()) {
                        eprintln!("hoshi: failed to record a thread title: {error}");
                    }
                    if !events.is_empty() && !generated.needs_refinement {
                        state.name_branch(&thread_id, &generated.title);
                    }
                    if generated.needs_refinement && request.previous_title.is_none() {
                        state.titles_to_refine.insert(thread_id);
                    }
                    return;
                }
                if attempt < attempts {
                    thread::sleep(delay);
                    delay *= 2;
                }
            }
        });
    }

    /// Names the branch of a worktree hoshi made after the thread's title, as t3code does.
    /// Only the `hoshi/<id>` it started on, and only before it was ever pushed.
    pub(crate) fn name_branch(&self, thread_id: &str, title: &str) {
        let Some(thread) = self.model.thread(thread_id) else {
            return;
        };
        let placeholder = format!("hoshi/{}", thread_id.chars().take(8).collect::<String>());
        let (Some(path), Some(branch)) = (&thread.worktree_path, &thread.branch) else {
            return;
        };
        if thread.worktree_adopted || thread.worktree_released || *branch != placeholder {
            return;
        }
        let Some(name) = worktree::branch_name(title) else {
            return;
        };
        let (path, state, thread_id) = (path.clone(), self.this.clone(), thread_id.to_owned());
        thread::spawn(move || {
            let pushed = git::branch_config(&path, &placeholder, "remote").is_some();
            if pushed || git::current_branch(&path).as_deref() != Some(placeholder.as_str()) {
                return;
            }
            let Some(free) = (1..=20)
                .map(|n| match n {
                    1 => format!("hoshi/{name}"),
                    _ => format!("hoshi/{name}-{n}"),
                })
                .find(|candidate| git::branch_tip(&path, candidate).is_none())
            else {
                return;
            };
            if let Err(error) = git::rename_branch(&path, &placeholder, &free) {
                eprintln!("hoshi: kept branch {placeholder}: {error}");
                return;
            }
            read_branches(&state, &[thread_id]);
        });
    }

    /// Names the thread from its messages. `forced` regenerates even when the user
    /// turned generated titles off.
    pub(crate) fn rename_from_conversation(&mut self, thread_id: &str, forced: bool) {
        if !forced && !self.thread_titles.generate {
            return;
        }
        let Some(current) = self
            .model
            .thread(thread_id)
            .map(|thread| thread.title.clone())
        else {
            return;
        };
        let items = self.store.thread_items(thread_id).unwrap_or_default();
        let (context, images) = titles::conversation(&items);
        if context.is_empty() {
            return;
        }
        let request = TitleRequest {
            context,
            previous_title: Some(current.clone()),
            images,
            model: None,
            instructions: String::new(),
        };
        self.name_thread(thread_id, &current, request);
    }
}
