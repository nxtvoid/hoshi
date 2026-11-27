use crate::*;

impl State {
    pub(crate) fn wake_sync(&self) {
        if let Some(wake) = &self.sync_wake {
            let _ = wake.send(());
        }
    }

    /// Reads the `origin` of the thread's project now when the read at start has not come to
    /// it yet, so the instructions of the turns its process runs agree on its forge.
    pub(crate) fn read_origin(&mut self, thread_id: &str) {
        let Some(project) = self
            .model
            .thread(thread_id)
            .and_then(|thread| thread.project_id.clone())
            .filter(|project| !self.origins_read.contains(project))
        else {
            return;
        };
        let Some(root) = self
            .model
            .project(&project)
            .map(|project| project.workspace_root.clone())
        else {
            return;
        };
        if let Some(repository) = git::origin_url(&root).and_then(|url| parse_remote(&url)) {
            self.repositories.insert(project.clone(), repository);
        }
        self.origins_read.insert(project);
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
