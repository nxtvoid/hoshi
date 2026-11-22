use crate::*;

impl State {
    /// `events` with what each turn runs on as the provider's catalog names it where it stood
    /// for its defaults, so a change of default later doesn't rewrite what a turn ran on; a
    /// change that turns out to be none goes. A rewind and a fork get what the turn before
    /// their point ran on. `None` when nothing needed it.
    pub(crate) fn with_models_named(&self, events: &[Event]) -> Option<Vec<Event>> {
        let concerned = events.iter().any(|event| match event {
            Event::TurnStarted { model, .. } => model.is_some(),
            Event::ThreadRewound { .. } => true,
            Event::ThreadCreated { thread } => thread.forked_from.is_some(),
            Event::ItemAdded { item } => matches!(
                item.content,
                ItemContent::UserMessage {
                    model_switch: Some(_),
                    ..
                } | ItemContent::ModelSwitched { .. }
            ),
            _ => false,
        });
        if !concerned {
            return None;
        }
        let mut catalogs: HashMap<String, Option<ProviderModels>> = HashMap::new();
        // What a turn ran on is kept as recorded; only what it left to the defaults is named.
        let mut name = |choice: &mut ModelChoice, recorded: bool| {
            let catalog = catalogs
                .entry(choice.provider.clone())
                .or_insert_with(|| self.known_models(&choice.provider));
            if let Some(catalog) = catalog {
                name_choice(choice, catalog, recorded);
            }
        };
        let same = |from: &ModelChoice, to: &ModelChoice| {
            from.model == to.model && from.effort == to.effort
        };
        // A fork's history before its first own turn is its source's, up to the fork's turn.
        let before = |thread_id: &str, item_id: &str| {
            let (mut thread_id, mut item_id) = (thread_id.to_owned(), item_id.to_owned());
            for _ in 0..MAX_FORK_DEPTH {
                match self.store.turn_model_before(&thread_id, &item_id) {
                    Ok(Some(model)) => return Some(model),
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("hoshi: could not read what a turn ran on: {error}");
                        return None;
                    }
                }
                let origin = self.model.thread(&thread_id)?.forked_from.as_deref()?;
                thread_id.clone_from(&origin.thread_id);
                item_id.clone_from(&origin.turn.through_item_id);
            }
            None
        };
        let mut events = events.to_vec();
        for event in &mut events {
            match event {
                Event::TurnStarted {
                    model: Some(model), ..
                } => name(model, false),
                Event::ThreadRewound {
                    thread_id,
                    message_id,
                    last_turn_model,
                    ..
                } => *last_turn_model = before(thread_id, message_id),
                Event::ThreadCreated { thread } => {
                    if let Some(origin) = &thread.forked_from {
                        thread.last_turn_model =
                            before(&origin.thread_id, &origin.turn.through_item_id).map(Box::new);
                    }
                }
                Event::ItemAdded { item } => match &mut item.content {
                    ItemContent::UserMessage { model_switch, .. } => {
                        if let Some(switch) = model_switch {
                            name(&mut switch.from, true);
                            name(&mut switch.to, false);
                            if same(&switch.from, &switch.to) {
                                *model_switch = None;
                            }
                        }
                    }
                    ItemContent::ModelSwitched { from, to } => {
                        name(from, true);
                        name(to, false);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        events.retain(|event| {
            !matches!(
                event,
                Event::ItemAdded {
                    item: ThreadItem {
                        content: ItemContent::ModelSwitched { from, to },
                        ..
                    },
                } if same(from, to)
            )
        });
        Some(events)
    }

    /// The models `provider` offers as last listed, without asking its CLI.
    pub(crate) fn known_models(&self, provider: &str) -> Option<ProviderModels> {
        let adapter = self.adapter_for(provider)?;
        let kept = provider_cache::read(&self.provider_cache_dir, provider);
        provider_cache::hydrate(adapter.offline_models(), kept)
    }

    pub(crate) fn keep_provider_version(
        &mut self,
        provider: &str,
        installation: &Installation,
        update: ProviderUpdate,
    ) {
        let version = ProviderVersion {
            installed: installation.installed.clone(),
            latest: installation.latest.clone(),
            update_available: installation.update_available(),
            update_command: installation.updater.as_ref().map(Updater::display),
            update,
        };
        self.provider_versions.insert(provider.to_owned(), version);
        self.notify_provider_version(provider);
    }

    pub(crate) fn set_provider_update(&mut self, provider: &str, update: ProviderUpdate) {
        let version = self
            .provider_versions
            .entry(provider.to_owned())
            .or_default();
        version.update = update;
        self.notify_provider_version(provider);
    }

    pub(crate) fn forget_provider(&mut self, provider: String) {
        self.provider_versions.remove(&provider);
        self.provider_accounts.remove(&provider);
        (self.notify)(Notification::ProviderVersion(ProviderVersionUpdate {
            provider,
            version: None,
        }));
        self.notify_providers();
    }

    /// Also sends the whole status, so a status taken before this change can't undo it.
    pub(crate) fn notify_provider_version(&self, provider: &str) {
        (self.notify)(Notification::ProviderVersion(ProviderVersionUpdate {
            provider: provider.to_owned(),
            version: self.provider_versions.get(provider).cloned(),
        }));
        self.notify_providers();
    }

    pub(crate) fn providers_status(&self) -> ProvidersStatus {
        ProvidersStatus {
            versions: self.provider_versions.clone(),
            accounts: self.provider_accounts.clone(),
            checked_at: self.providers_checked_at.clone(),
            sign_ins: self.provider_sign_ins.clone(),
            logins: self.provider_logins.clone(),
            codex_account: self.codex_account.clone(),
            usage_limits: self.usage_limits.clone(),
            revision: self.providers_revision.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub(crate) fn notify_providers(&self) {
        (self.notify)(Notification::Providers(self.providers_status()));
    }

    pub(crate) fn set_sign_in(&mut self, provider: &str, sign_in: ProviderSignIn) {
        self.provider_sign_ins.insert(provider.to_owned(), sign_in);
        self.notify_providers();
    }

    /// Codex signed in or out: its account as its files now say.
    pub(crate) fn keep_codex_account(
        &mut self,
        account: Option<CodexAccount>,
        status: ProviderAccountStatus,
    ) {
        self.codex_account = account;
        if self.adapter_for(CODEX).is_some() {
            self.provider_accounts.insert(CODEX.to_owned(), status);
        }
    }

    pub(crate) fn keep_usage_limits(&mut self, provider: &str, limits: UsageLimits) {
        if let Err(error) = self.store.save_provider_usage_limits(provider, &limits) {
            eprintln!("hoshi: failed to cache the usage limits: {error}");
        }
        if provider == DEFAULT_PROVIDER {
            (self.notify)(Notification::UsageLimits(limits.clone()));
        }
        self.usage_limits.insert(provider.to_owned(), limits);
        self.notify_providers();
    }
}
