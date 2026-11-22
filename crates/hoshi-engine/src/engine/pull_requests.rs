use crate::*;

impl Engine {
    pub fn set_forges(&self, forges: Vec<Arc<dyn Forge>>) {
        lock(&self.state).forges = forges
            .into_iter()
            .map(|forge| (forge.kind(), forge))
            .collect();
    }

    /// Reads again the pull requests not read in the last few seconds, as when the
    /// window gets focus.
    pub fn refresh_pull_requests(&self) {
        let mut state = lock(&self.state);
        state.pull_requests.check_stale(Instant::now());
        state.wake_sync();
    }

    /// The detail of a thread's pull request as last read, answered at once. When it is
    /// missing or old, it is read again in the background and arrives as a
    /// [`Notification::PullRequestDetail`].
    pub fn pull_request_detail(&self, thread_id: &str) -> Option<PullRequestDetail> {
        let mut state = lock(&self.state);
        let version = Version::of(state.pull_requests.get(thread_id)?);
        let (detail, read) = state
            .pull_request_cache
            .detail(thread_id, &version, Instant::now());
        if read {
            read_detail(state.this.clone(), thread_id.to_owned(), version);
        }
        detail
    }

    /// The pull request a web link points to. `None` when the link is not a pull request
    /// of a forge with an account. A link read in the last minute is answered from memory.
    pub fn linked_pull_request(&self, url: &str) -> Result<Option<PullRequest>, CommandError> {
        let Some(key) = parse_pull_request_url(url) else {
            return Ok(None);
        };
        let forge = {
            let state = lock(&self.state);
            let fresh = state
                .linked_pull_requests
                .get(&key)
                .filter(|(read_at, _)| read_at.elapsed() < LINKED_PULL_REQUEST_FRESH_FOR);
            if let Some((_, pull_request)) = fresh {
                return Ok(Some(pull_request.clone()));
            }
            state.forges.get(&key.0.forge).cloned()
        };
        let Some(forge) = forge else {
            return Ok(None);
        };
        let pull_request = forge.pull_request(&key.0, key.1).map_err(forge_error)?;
        lock(&self.state)
            .linked_pull_requests
            .insert(key, (Instant::now(), pull_request.clone()));
        Ok(Some(pull_request))
    }

    /// Warms the detail and the files of a thread's pull request before they are shown,
    /// as when the pointer rests on it.
    pub fn prefetch_pull_request(&self, thread_id: &str) {
        let _ = self.pull_request_detail(thread_id);
        let mut state = lock(&self.state);
        let Some(version) = state.pull_requests.get(thread_id).map(Version::of) else {
            return;
        };
        let (_, read) = state
            .pull_request_cache
            .files(thread_id, &version, Instant::now());
        if read {
            let this = state.this.clone();
            let thread_id = thread_id.to_owned();
            thread::spawn(move || read_files(&this, &thread_id, version));
        }
    }

    /// The files a thread's pull request changes. Their patches stay here until
    /// [`Self::pull_request_file_diff`] asks for one. Waits for a read already on its way
    /// instead of starting another.
    pub fn pull_request_files(&self, thread_id: &str) -> Option<Vec<DiffFile>> {
        let waited_since = Instant::now();
        loop {
            let mut state = lock(&self.state);
            let version = Version::of(state.pull_requests.get(thread_id)?);
            let (files, read) = state
                .pull_request_cache
                .files(thread_id, &version, Instant::now());
            if let Some(files) = files {
                return Some(files.iter().map(|file| file.file.clone()).collect());
            }
            drop(state);
            if read {
                return read_files(&Arc::downgrade(&self.state), thread_id, version);
            }
            if waited_since.elapsed() > FILES_WAIT_LIMIT {
                return None;
            }
            thread::sleep(FILES_WAIT_STEP);
        }
    }

    /// One file's patch from the last [`Self::pull_request_files`] read; `None` for binaries
    /// and diffs the forge left out.
    pub fn pull_request_file_diff(&self, thread_id: &str, path: &str) -> Option<FileDiff> {
        let state = lock(&self.state);
        let file = state
            .pull_request_cache
            .current_files(thread_id)?
            .iter()
            .find(|file| file.file.path == path)?;
        file.patch.as_deref().map(diff::truncate)
    }

    /// Asks the forge to change the thread's pull request, then reads it again soon.
    pub fn run_pull_request_action(
        &self,
        thread_id: &str,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), CommandError> {
        let (forge, repository, number) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge
            .act(&repository, number, forge_id, action)
            .map_err(forge_error)?;
        {
            let mut state = lock(&self.state);
            state.pull_requests.check_soon(thread_id);
            state.pull_request_cache.invalidate(thread_id);
        }
        let _ = self.pull_request_detail(thread_id);
        Ok(())
    }

    /// Accounts the thread's pull request can ask for a review, matching `query`.
    pub fn reviewer_candidates(
        &self,
        thread_id: &str,
        query: &str,
    ) -> Result<Vec<String>, CommandError> {
        let (forge, repository, _) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge
            .reviewer_candidates(&repository, query)
            .map_err(forge_error)
    }

    /// The labels of the repository the thread's pull request belongs to.
    pub fn repository_labels(
        &self,
        thread_id: &str,
    ) -> Result<Vec<PullRequestLabel>, CommandError> {
        let (forge, repository, _) = self
            .pull_request_target(thread_id)
            .ok_or(CommandError::ForgeUnavailable)?;
        forge.labels(&repository).map_err(forge_error)
    }

    pub(crate) fn pull_request_target(
        &self,
        thread_id: &str,
    ) -> Option<(Arc<dyn Forge>, RepositoryRef, u32)> {
        let state = lock(&self.state);
        let thread = state.model.thread(thread_id)?;
        let repository = state.repositories.get(thread.project_id.as_ref()?)?.clone();
        let forge = state.forges.get(&repository.forge)?.clone();
        let number = state.pull_requests.number(thread_id)?;
        Some((forge, repository, number))
    }

    /// Checks the token and its scopes against Bitbucket, then keeps it in the system's
    /// credential store.
    pub fn connect_bitbucket(
        &self,
        email: &str,
        token: &str,
    ) -> Result<ForgeAccount, CommandError> {
        let credentials = BitbucketCredentials {
            email: email.trim().to_owned(),
            token: token.trim().to_owned(),
        };
        let access = Bitbucket::with_credentials(credentials.clone())
            .check_access()
            .map_err(forge_error)?;
        if !access.missing_scopes.is_empty() {
            return Err(CommandError::ForgeScopesMissing {
                scopes: access.missing_scopes,
            });
        }
        secrets::write(SecretKey::Bitbucket, &credentials.encode())
            .map_err(|detail| CommandError::Internal { detail })?;
        self.credentials_changed(ForgeKind::Bitbucket);
        Ok(ForgeAccount {
            forge: ForgeKind::Bitbucket,
            login: Some(access.login),
        })
    }

    /// Forgets a forge's stored credentials. GitHub's belong to its CLI and stay.
    pub fn disconnect_forge(&self, forge: ForgeKind) -> Result<(), CommandError> {
        match forge {
            ForgeKind::Github => return Ok(()),
            ForgeKind::Bitbucket => secrets::delete(SecretKey::Bitbucket)
                .map_err(|detail| CommandError::Internal { detail })?,
        }
        self.credentials_changed(forge);
        Ok(())
    }

    pub(crate) fn credentials_changed(&self, kind: ForgeKind) {
        let mut state = lock(&self.state);
        if let Some(forge) = state.forges.get(&kind) {
            forge.forget_credentials();
        }
        state.pull_requests.resume(kind);
        state.wake_sync();
    }

    /// Who each forge is signed in as now, reading the credentials again in case the account
    /// changed. Asks the forges, so it can take a moment.
    pub fn forge_accounts(&self) -> Vec<ForgeAccount> {
        let forges: Vec<_> = lock(&self.state).forges.values().cloned().collect();
        let accounts: Vec<ForgeAccount> = forges
            .into_iter()
            .map(|forge| {
                forge.forget_credentials();
                ForgeAccount {
                    forge: forge.kind(),
                    login: forge.viewer().ok(),
                }
            })
            .collect();
        let mut state = lock(&self.state);
        for account in &accounts {
            match &account.login {
                Some(login) => state.viewers.insert(account.forge, login.clone()),
                None => state.viewers.remove(&account.forge),
            };
        }
        accounts
    }

    /// Pull requests of the project's repository matching `query`, for `#` mentions.
    /// Asks the forge.
    pub fn search_pull_requests(
        &self,
        project_id: &str,
        query: &str,
    ) -> Result<Vec<PullRequest>, CommandError> {
        let (forge, repository) = {
            let state = lock(&self.state);
            let repository = state
                .repositories
                .get(project_id)
                .ok_or(CommandError::ForgeUnavailable)?
                .clone();
            let forge = state
                .forges
                .get(&repository.forge)
                .ok_or(CommandError::ForgeUnavailable)?
                .clone();
            (forge, repository)
        };
        forge
            .search_pull_requests(&repository, query)
            .map_err(forge_error)
    }
}
