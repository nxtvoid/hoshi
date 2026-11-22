use crate::*;

impl Engine {
    /// Checks the provider's CLI again when the last check is a few minutes old, as when
    /// the window gets focus.
    pub fn refresh_provider_version(&self) {
        let stale = lock(&self.state)
            .provider_version_checked_at
            .is_none_or(|at| at.elapsed() >= PROVIDER_VERSION_FRESH_FOR);
        if stale {
            self.check_provider_version();
        }
    }

    /// Also reads the plan's limits in the background, so they are ready when asked for.
    /// Also reads the plan's limits and checks the CLI against its releases in the background.
    pub fn set_adapter(&self, adapter: Option<Arc<dyn ProviderAdapter>>) {
        self.set_adapters(adapter.into_iter().collect());
    }

    /// The enabled providers' adapters, one per provider. Each that reports them also reads
    /// its plan's limits in the background, and the CLIs are checked against their releases.
    pub fn set_adapters(&self, adapters: Vec<Arc<dyn ProviderAdapter>>) {
        let adapter = adapters
            .iter()
            .find(|adapter| adapter.provider() == DEFAULT_PROVIDER)
            .cloned();
        let enabled = adapter.is_some();
        let any_enabled = !adapters.is_empty();
        let reporting: Vec<String> = adapters
            .iter()
            .filter(|adapter| adapter.reports_usage_limits())
            .map(|adapter| adapter.provider().to_owned())
            .collect();
        {
            let mut state = lock(&self.state);
            let mut gone: Vec<String> = state
                .adapters
                .iter()
                .map(|adapter| adapter.provider().to_owned())
                .filter(|provider| !adapters.iter().any(|kept| kept.provider() == provider))
                .collect();
            if !enabled && !gone.iter().any(|provider| provider == DEFAULT_PROVIDER) {
                gone.push(DEFAULT_PROVIDER.to_owned());
            }
            state.adapter = adapter;
            state.adapters = adapters;
            for provider in gone {
                state.forget_provider(provider);
            }
        }
        self.slash_commands.clear();
        for provider in reporting {
            let state = Arc::downgrade(&self.state);
            let probes = self.usage_probes.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_usage_limits(&state, &probes, &provider, false);
                }
            });
        }
        if any_enabled {
            let state = Arc::downgrade(&self.state);
            let maintenance = self.maintenance.clone();
            thread::spawn(move || {
                if let Some(state) = state.upgrade() {
                    check_provider_versions(&state, &maintenance, None);
                }
            });
        }
    }

    /// The default provider's CLI as last checked, without checking it.
    pub fn provider_version(&self) -> Option<ProviderVersion> {
        self.provider_version_of(DEFAULT_PROVIDER)
    }

    /// Provider `provider`'s CLI as last checked, without checking it.
    pub fn provider_version_of(&self, provider: &str) -> Option<ProviderVersion> {
        lock(&self.state).provider_versions.get(provider).cloned()
    }

    /// Checks the default provider's CLI against its latest release. While an update or
    /// another check runs, answers with what is known.
    pub fn check_provider_version(&self) -> Option<ProviderVersion> {
        check_provider_versions(&self.state, &self.maintenance, Some(DEFAULT_PROVIDER));
        self.provider_version()
    }

    /// Whether the default provider's CLI is installed and signed in. Runs the CLI.
    pub fn provider_account_status(&self) -> ProviderAccountStatus {
        let adapter = lock(&self.state).adapter.clone();
        adapter.map_or(ProviderAccountStatus::Unknown, |adapter| {
            adapter.account_status()
        })
    }

    /// Every enabled provider's CLI and account as last checked, without checking them.
    pub fn providers_status(&self) -> ProvidersStatus {
        lock(&self.state).providers_status()
    }

    /// Checks every enabled provider's CLI against its latest release, and whether it is
    /// installed and signed in. Runs each CLI, so it can take a few seconds.
    pub fn check_providers(&self) -> ProvidersStatus {
        let adapters = lock(&self.state).adapters.clone();
        let accounts: Vec<(String, ProviderAccountStatus, Option<String>)> =
            thread::scope(|scope| {
                let accounts: Vec<_> = adapters
                    .iter()
                    .map(|adapter| {
                        scope.spawn(|| {
                            let status = adapter.account_status();
                            (
                                adapter.provider().to_owned(),
                                status,
                                adapter.signed_in_as(),
                            )
                        })
                    })
                    .collect();
                check_provider_versions(&self.state, &self.maintenance, None);
                accounts
                    .into_iter()
                    .filter_map(|account| account.join().ok())
                    .collect()
            });
        let codex = lock(&self.state).codex.clone();
        let codex_account = codex.and_then(|codex| codex_account(codex.home.as_deref()));
        let mut state = lock(&self.state);
        state.provider_logins.clear();
        for (provider, account, login) in accounts {
            if state.adapter_for(&provider).is_some() {
                if let Some(login) = login {
                    state.provider_logins.insert(provider.clone(), login);
                }
                state.provider_accounts.insert(provider, account);
            }
        }
        state.codex_account = codex_account.filter(|_| state.codex.is_some());
        state.providers_checked_at = Some(now());
        state.providers_status()
    }

    /// Where Codex is found and keeps its sign-in; `None` while it isn't enabled.
    pub fn set_codex(&self, setup: Option<CodexSetup>) {
        let mut state = lock(&self.state);
        if setup.is_none() {
            state.codex_account = None;
            if let Some(cancel) = state.codex_sign_in.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.provider_sign_ins.remove(CODEX);
        }
        state.codex = setup;
    }

    /// Starts signing Codex in with ChatGPT and returns the page the user signs in on, for
    /// their browser. The sign-in then waits in the background; how it ends arrives as
    /// `Notification::Providers`. Runs the CLI.
    pub fn sign_in_codex(&self) -> Result<String, ProviderSignInFailure> {
        let setup = {
            let mut state = lock(&self.state);
            let setup = state
                .codex
                .clone()
                .ok_or(ProviderSignInFailure::NotInstalled)?;
            if let Some(cancel) = state.codex_sign_in.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.set_sign_in(CODEX, ProviderSignIn::Starting);
            setup
        };
        let sign_in = match start_codex_sign_in(&setup) {
            Ok(sign_in) => sign_in,
            Err(reason) => {
                lock(&self.state).set_sign_in(
                    CODEX,
                    ProviderSignIn::Failed {
                        reason: reason.clone(),
                    },
                );
                return Err(reason);
            }
        };
        let url = sign_in.url().to_owned();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = lock(&self.state);
            state.codex_sign_in = Some(cancel.clone());
            state.set_sign_in(CODEX, ProviderSignIn::Waiting { url: url.clone() });
        }
        let state = Arc::downgrade(&self.state);
        thread::spawn(move || {
            let result = sign_in.wait(&cancel);
            let account = codex_account(setup.home.as_deref());
            let Some(state) = state.upgrade() else {
                return;
            };
            let mut state = lock(&state);
            let current = state
                .codex_sign_in
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cancel));
            if !current {
                return;
            }
            state.codex_sign_in = None;
            match result {
                Ok(()) => {
                    state.keep_codex_account(account, ProviderAccountStatus::Ready);
                    state.set_sign_in(CODEX, ProviderSignIn::Succeeded);
                }
                Err(reason) => state.set_sign_in(CODEX, ProviderSignIn::Failed { reason }),
            }
        });
        Ok(url)
    }

    /// Stops the Codex sign-in that waits for the user.
    pub fn cancel_codex_sign_in(&self) {
        let mut state = lock(&self.state);
        if let Some(cancel) = state.codex_sign_in.take() {
            cancel.store(true, Ordering::SeqCst);
            state.set_sign_in(CODEX, ProviderSignIn::Idle);
        }
    }

    /// Signs Codex out through Codex itself. Runs the CLI.
    pub fn sign_out_codex(&self) -> Result<(), ProviderSignInFailure> {
        let setup = lock(&self.state)
            .codex
            .clone()
            .ok_or(ProviderSignInFailure::NotInstalled)?;
        codex_sign_out(&setup)?;
        let mut state = lock(&self.state);
        state.keep_codex_account(None, ProviderAccountStatus::SignedOut);
        state.set_sign_in(CODEX, ProviderSignIn::Idle);
        Ok(())
    }

    /// Updates the default provider's CLI in the background; its progress arrives as
    /// `Notification::ProviderVersion`.
    pub fn update_provider(&self) {
        self.update_provider_of(DEFAULT_PROVIDER);
    }

    /// Updates provider `provider`'s CLI in the background; its progress arrives as
    /// `Notification::ProviderVersion`.
    pub fn update_provider_of(&self, provider: &str) {
        let state = Arc::downgrade(&self.state);
        let maintenance = self.maintenance.clone();
        let provider = provider.to_owned();
        thread::spawn(move || {
            if let Some(state) = state.upgrade() {
                update_provider(&state, &maintenance, &provider);
            }
        });
    }

    /// What a prompt can start with in the thread's folder, or in the chats folder for a
    /// chat not created yet. `None` while no provider is enabled or when it can't say. Can
    /// run the provider's CLI.
    pub fn slash_commands(
        &self,
        project_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Option<Vec<SlashCommand>>, CommandError> {
        let cwd = match self.composer_cwd(project_id, thread_id)? {
            Some(cwd) => cwd,
            None => {
                let chats_dir = lock(&self.state).chats_dir.clone();
                std::fs::create_dir_all(&chats_dir).map_err(|error| CommandError::Internal {
                    detail: error.to_string(),
                })?;
                chats_dir.to_string_lossy().into_owned()
            }
        };
        let adapter = {
            let state = lock(&self.state);
            let provider = thread_id
                .and_then(|thread_id| state.model.thread(thread_id))
                .map_or(DEFAULT_PROVIDER, |thread| thread.provider.as_str());
            state.adapter_for(provider)
        };
        let Some(adapter) = adapter else {
            return Ok(None);
        };
        let root = PathBuf::from(&cwd);
        let key = format!("{}\n{cwd}", adapter.provider());
        let commands = self
            .slash_commands
            .get(&key, move || adapter.slash_commands(&root));
        Ok(commands.map(|commands| commands.as_ref().clone()))
    }

    /// The default provider's models. Can run the provider's CLI.
    pub fn models(&self) -> Option<ProviderModels> {
        let adapter = lock(&self.state).adapter.clone()?;
        Some(adapter.models())
    }

    /// Every enabled provider's models, the default provider first. Runs the providers' CLIs
    /// side by side, and keeps what they list for [`Engine::known_provider_models`].
    pub fn provider_models(&self) -> Vec<ProviderModels> {
        let (adapters, cache_dir) = {
            let state = lock(&self.state);
            (state.adapters.clone(), state.provider_cache_dir.clone())
        };
        let mut catalogs: Vec<ProviderModels> = thread::scope(|scope| {
            let reads: Vec<_> = adapters
                .iter()
                .map(|adapter| {
                    scope.spawn(|| ProviderModels {
                        compacts_idle_threads: adapter.compacts_idle_threads(),
                        ..adapter.models()
                    })
                })
                .collect();
            reads
                .into_iter()
                .filter_map(|read| read.join().ok())
                .collect()
        });
        catalogs.sort_by_key(|catalog| catalog.provider != DEFAULT_PROVIDER);
        for catalog in catalogs.iter().filter(|catalog| !catalog.models.is_empty()) {
            if let Err(error) = provider_cache::write(&cache_dir, catalog) {
                eprintln!(
                    "hoshi: failed to keep the models of {}: {error}",
                    catalog.provider
                );
            }
        }
        catalogs
    }

    /// Every enabled provider's models as known without running a CLI, the default provider
    /// first: its offline catalog, then what [`Engine::provider_models`] last kept. Providers
    /// with neither are left out.
    pub fn known_provider_models(&self) -> Vec<ProviderModels> {
        let (adapters, cache_dir) = {
            let state = lock(&self.state);
            (state.adapters.clone(), state.provider_cache_dir.clone())
        };
        let mut catalogs: Vec<ProviderModels> = adapters
            .iter()
            .filter_map(|adapter| {
                let kept = provider_cache::read(&cache_dir, adapter.provider());
                let catalog = provider_cache::hydrate(adapter.offline_models(), kept)?;
                Some(ProviderModels {
                    compacts_idle_threads: adapter.compacts_idle_threads(),
                    ..catalog
                })
            })
            .collect();
        catalogs.sort_by_key(|catalog| catalog.provider != DEFAULT_PROVIDER);
        catalogs
    }

    /// The default provider's limits last read or reported, possibly in an earlier run,
    /// without asking it.
    pub fn known_usage_limits(&self) -> Option<UsageLimits> {
        self.known_usage_limits_of(DEFAULT_PROVIDER)
    }

    /// Provider `provider`'s limits last read or reported, possibly in an earlier run, without
    /// asking it.
    pub fn known_usage_limits_of(&self, provider: &str) -> Option<UsageLimits> {
        lock(&self.state).usage_limits.get(provider).cloned()
    }

    /// The default provider plan's limits; see [`Engine::usage_limits_of`].
    pub fn usage_limits(&self, force: bool) -> Option<UsageLimits> {
        self.usage_limits_of(DEFAULT_PROVIDER, force)
    }

    /// Provider `provider`'s plan limits, asking it unless it answered within the last minute.
    /// `force` always waits for an answer given after the call. `None` when it can't say or
    /// isn't enabled.
    pub fn usage_limits_of(&self, provider: &str, force: bool) -> Option<UsageLimits> {
        check_usage_limits(&self.state, &self.usage_probes, provider, force)
    }

    /// The enabled providers that can read their plan's limits, in the order they were set.
    pub fn usage_limit_providers(&self) -> Vec<String> {
        lock(&self.state)
            .adapters
            .iter()
            .filter(|adapter| adapter.reports_usage_limits())
            .map(|adapter| adapter.provider().to_owned())
            .collect()
    }

    /// Spends the default provider's next banked reset; see [`Engine::spend_banked_reset_of`].
    pub fn spend_banked_reset(&self) -> BankedResetSpend {
        self.spend_banked_reset_of(DEFAULT_PROVIDER)
    }

    /// Spends provider `provider`'s next banked reset, then reads its limits again. Reaches
    /// the network.
    pub fn spend_banked_reset_of(&self, provider: &str) -> BankedResetSpend {
        self.spend_banked_grant(provider, None)
    }

    /// Spends a banked reset of `provider`'s grant `grant_id`, one its limits list, or its
    /// next one when `None`; then reads its limits again. Reaches the network.
    pub fn spend_banked_grant(&self, provider: &str, grant_id: Option<&str>) -> BankedResetSpend {
        let mut pending = self
            .reset_spend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (adapter, grant) = {
            let state = lock(&self.state);
            let grant = state
                .usage_limits
                .get(provider)
                .and_then(|limits| limits.banked_resets.as_ref())
                .filter(|banked| banked.count > 0)
                .and_then(|banked| match grant_id {
                    Some(id) => banked
                        .grants
                        .iter()
                        .any(|grant| grant.id == id && grant.resets > 0)
                        .then(|| id.to_owned()),
                    None => banked.next_grant_id.clone(),
                });
            (state.adapter_for(provider), grant)
        };
        let (Some(adapter), Some(grant)) = (adapter, grant) else {
            return BankedResetSpend::NoneLeft;
        };
        let key = (provider.to_owned(), grant);
        let request_id = pending.remove(&key).unwrap_or_else(delegation::new_id);
        let spend = adapter.spend_banked_reset(&key.1, &request_id);
        if !spend.settled() {
            pending.insert(key, request_id);
            return spend;
        }
        if matches!(spend, BankedResetSpend::Failed { .. }) {
            return spend;
        }
        let refreshed = check_usage_limits(&self.state, &self.usage_probes, provider, true);
        if spend == BankedResetSpend::Reset && refreshed.is_none() {
            return BankedResetSpend::Failed {
                reason: BankedResetFailure::LimitsUnread,
            };
        }
        spend
    }
}
