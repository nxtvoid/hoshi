use crate::*;

impl State {
    pub(crate) fn wake_sync(&self) {
        if let Some(wake) = &self.sync_wake {
            let _ = wake.send(());
        }
    }

    /// The forge and repository of a thread's project.
    pub(crate) fn pull_request_target_of(
        &self,
        thread_id: &str,
    ) -> Option<(Arc<dyn Forge>, RepositoryRef)> {
        let thread = self.model.thread(thread_id)?;
        let repository = self.repositories.get(thread.project_id.as_ref()?)?.clone();
        let forge = self.forges.get(&repository.forge)?.clone();
        Some((forge, repository))
    }

    pub(crate) fn due_pull_request_jobs(&self) -> Vec<PullRequestJob> {
        let now = Instant::now();
        self.model
            .threads()
            .filter(|thread| !thread.archived || self.pull_requests.has_open(&thread.id))
            .filter_map(|thread| {
                let branch = thread.branch.clone()?;
                let repository = self.repositories.get(thread.project_id.as_ref()?)?.clone();
                let forge = self.forges.get(&repository.forge)?.clone();
                self.pull_requests
                    .is_due(&thread.id, repository.forge, now)
                    .then(|| PullRequestJob {
                        thread_id: thread.id.clone(),
                        branch,
                        repository,
                        forge,
                    })
            })
            .collect()
    }

    pub(crate) fn record_pull_request(
        &mut self,
        job: &PullRequestJob,
        result: Result<Option<hoshi_core::PullRequest>, hoshi_forge::ForgeError>,
    ) {
        let Some(thread) = self.model.thread(&job.thread_id) else {
            return;
        };
        // The branch moved while the forge answered; the next read covers the new one.
        if thread.branch.as_deref() != Some(job.branch.as_str()) {
            return;
        }
        let working = thread.status == ThreadStatus::Working || thread.watching_pull_request;
        let changed = self.pull_requests.record(
            &job.thread_id,
            job.repository.forge,
            result,
            working,
            Instant::now(),
            Utc::now(),
        );
        let Some(pull_request) = changed else {
            return;
        };
        let update = ThreadPullRequest {
            thread_id: job.thread_id.clone(),
            pull_request,
        };
        if let Err(error) = self.store.save_pull_request(&update) {
            eprintln!("hoshi: failed to cache a pull request: {error}");
        }
        (self.notify)(Notification::PullRequest(update));
    }
}
