use super::*;

/// The transcript of the thread on screen.
pub struct ThreadState {
    pub thread_id: String,
    pub items: Vec<ThreadItem>,
    pub has_older: bool,
    pub loading: bool,
    pub loading_older: bool,
    /// Text still being written, in the order it started.
    pub streaming: Vec<Streaming>,
    /// Where the thread's agent runs, which paths in its calls are shown relative to.
    pub cwd: Option<String>,
    /// Items whole, read when a row that left something out opens.
    pub whole: HashMap<String, ItemContent>,
    pub(super) reading: HashSet<String>,
    /// `None` in tests that build a transcript by hand.
    pub engine: Option<Arc<Engine>>,
    /// How many of the first items the thread inherited from the threads it was forked from.
    pub inherited: usize,
    /// Each fork up the chain with the thread it came from, this thread first: their
    /// inherited history shows above its own items.
    pub lineage: Vec<(String, String)>,
    /// How many items each fork of `lineage` inherited, as far as read.
    pub(super) segments: Vec<usize>,
    /// The fork of `lineage` whose inherited history reads next, and the item it reads before.
    pub(super) next_history: usize,
    pub(super) history_before: Option<String>,
}

pub struct Streaming {
    pub item_id: String,
    pub kind: StreamKind,
    pub text: String,
}

impl ThreadState {
    pub fn new(thread_id: &str, items: Vec<ThreadItem>) -> Self {
        Self {
            thread_id: thread_id.to_owned(),
            items,
            has_older: false,
            loading: false,
            loading_older: false,
            streaming: Vec::new(),
            cwd: None,
            whole: HashMap::new(),
            reading: HashSet::new(),
            engine: None,
            inherited: 0,
            lineage: Vec::new(),
            segments: Vec::new(),
            next_history: 0,
            history_before: None,
        }
    }

    pub(super) fn load(
        engine: Arc<Engine>,
        thread_id: &str,
        cwd: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = thread_id.to_owned();
        let reader = engine.clone();
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { reader.transcript(&id, None) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading = false;
                match page {
                    Ok(page) => {
                        thread.items = page.items;
                        thread.has_older = page.has_older;
                    }
                    Err(error) => eprintln!("hoshi: transcript failed: {error}"),
                }
                thread.load_history(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self {
            loading: true,
            cwd,
            engine: Some(engine),
            ..Self::new(thread_id, Vec::new())
        }
    }

    /// The item as shown: whole once read, else as its page left it.
    pub fn content<'a>(&'a self, item: &'a ThreadItem) -> &'a ItemContent {
        self.whole.get(&item.id).unwrap_or(&item.content)
    }

    /// Reads the item whole in the background when its page left part of it out.
    pub fn read_whole(&mut self, item_id: &str, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let Some(at) = self.items.iter().position(|item| item.id == item_id) else {
            return;
        };
        let incomplete = left_out(&self.items[at].content);
        if !incomplete || self.whole.contains_key(item_id) || !self.reading.insert(item_id.into()) {
            return;
        }
        let thread_id = if at < self.inherited {
            self.items[at].thread_id.clone()
        } else {
            self.thread_id.clone()
        };
        let id = item_id.to_owned();
        cx.spawn(async move |this, cx| {
            let read = id.clone();
            let item = cx
                .background_spawn(async move { engine.thread_item(&thread_id, &read) })
                .await;
            this.update(cx, |thread, cx| {
                thread.reading.remove(&id);
                match item {
                    Ok(Some(item)) => {
                        thread.whole.insert(id, item.content);
                        cx.notify();
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("hoshi: could not read a tool call: {error}"),
                }
            })
            .ok();
        })
        .detach();
    }

    /// What a running call printed so far.
    pub fn live_output(&self, item_id: &str) -> Option<&str> {
        self.streaming
            .iter()
            .find(|streaming| {
                streaming.kind == StreamKind::ToolOutput && streaming.item_id == item_id
            })
            .map(|streaming| streaming.text.as_str())
    }

    /// Puts the page before the first item shown above it.
    pub fn load_older(&mut self, cx: &mut Context<Self>) {
        if !self.has_older {
            self.load_history(cx);
            return;
        }
        let (Some(engine), Some(first)) = (self.engine.clone(), self.items.first()) else {
            return;
        };
        if self.loading || self.loading_older {
            return;
        }
        self.loading_older = true;
        let (id, before) = (self.thread_id.clone(), first.id.clone());
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { engine.transcript(&id, Some(&before)) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading_older = false;
                match page {
                    Ok(page) => {
                        thread.has_older = page.has_older;
                        let mut items = page.items;
                        items.append(&mut thread.items);
                        thread.items = items;
                    }
                    Err(error) => eprintln!("hoshi: older transcript failed: {error}"),
                }
                thread.load_history(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Sets the forks the thread's inherited history comes from, once it is known to be one.
    pub fn set_lineage(&mut self, lineage: Vec<(String, String)>, cx: &mut Context<Self>) {
        if self.lineage == lineage || self.inherited > 0 {
            return;
        }
        self.lineage = lineage;
        self.load_history(cx);
        cx.notify();
    }

    /// Puts the next page of inherited history above, once the thread's own items are all read.
    pub(super) fn load_history(&mut self, cx: &mut Context<Self>) {
        let Some((fork, _)) = self.lineage.get(self.next_history) else {
            return;
        };
        let Some(engine) = self.engine.clone() else {
            return;
        };
        if self.has_older || self.loading || self.loading_older {
            return;
        }
        self.loading_older = true;
        let (fork, before) = (fork.clone(), self.history_before.clone());
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { engine.fork_history(&fork, before.as_deref()) })
                .await;
            this.update(cx, |thread, cx| {
                thread.loading_older = false;
                match page {
                    Ok(page) => thread.add_history(page, cx),
                    Err(error) => {
                        eprintln!("hoshi: inherited history failed: {error}");
                        thread.next_history = thread.lineage.len();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A page of the history the fork `lineage[next_history]` inherited, put above. The next
    /// fork up the chain starts right away once this one's is whole.
    pub(crate) fn add_history(&mut self, page: TranscriptPage, cx: &mut Context<Self>) {
        let at = self.next_history;
        if self.segments.len() <= at {
            self.segments.resize(at + 1, 0);
        }
        self.segments[at] += page.items.len();
        self.inherited += page.items.len();
        let more = page.has_older && !page.items.is_empty();
        self.history_before = page
            .items
            .first()
            .filter(|_| more)
            .map(|item| item.id.clone());
        if !more {
            self.next_history += 1;
        }
        let mut items = page.items;
        items.append(&mut self.items);
        self.items = items;
        if !more {
            self.load_history(cx);
        }
    }

    /// Where a "Forked from" divider goes, as the index of the item it goes before, and the
    /// thread it names: above the thread's own items, and above each fork's inherited history
    /// once read whole. None until the thread's own items are all read.
    pub fn fork_boundaries(&self) -> Vec<(usize, &str)> {
        if self.has_older || self.loading {
            return Vec::new();
        }
        let mut at = self.inherited;
        let mut boundaries = Vec::new();
        for (index, (_, source)) in self.lineage.iter().enumerate() {
            if index > 0 {
                if self.next_history < index {
                    break;
                }
                at -= self.segments.get(index - 1).copied().unwrap_or(0);
            }
            boundaries.push((at, source.as_str()));
        }
        boundaries
    }

    pub(super) fn apply(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::ItemAdded { item } if item.thread_id == self.thread_id => {
                if item.content == ItemContent::RunEnded {
                    self.drop_unfinished_replies();
                }
                self.streaming
                    .retain(|streaming| streaming.item_id != item.id);
                match self.items.iter_mut().find(|known| known.id == item.id) {
                    Some(known) => *known = item.clone(),
                    None => self.items.push(item.clone()),
                }
            }
            Event::ToolCallCompleted {
                thread_id,
                item_id,
                output: done,
                is_error: failed,
                detail: done_detail,
            } if *thread_id == self.thread_id => {
                self.streaming
                    .retain(|streaming| &streaming.item_id != item_id);
                if let Some(ItemContent::ToolCall {
                    output,
                    is_error,
                    detail,
                    ..
                }) = self.call_mut(item_id)
                {
                    *output = Some(done.clone());
                    *is_error = *failed;
                    if done_detail.is_some() {
                        detail.clone_from(done_detail);
                    }
                }
            }
            Event::ToolCallDetailSet {
                thread_id,
                item_id,
                detail: set,
            } if *thread_id == self.thread_id => {
                if let Some(ItemContent::ToolCall { detail, .. }) = self.call_mut(item_id) {
                    *detail = Some(set.clone());
                }
            }
            Event::ToolCallInterrupted { thread_id, item_id } if *thread_id == self.thread_id => {
                self.streaming
                    .retain(|streaming| &streaming.item_id != item_id);
                if let Some(ItemContent::ToolCall { interrupted, .. }) = self.call_mut(item_id) {
                    *interrupted = true;
                }
            }
            Event::MessagesDropped {
                thread_id,
                message_ids,
            } if *thread_id == self.thread_id => {
                for item in &mut self.items {
                    if let ItemContent::UserMessage { dropped, .. } = &mut item.content
                        && message_ids.contains(&item.id)
                    {
                        *dropped = true;
                    }
                }
            }
            Event::TurnCompleted { thread_id, .. } if *thread_id == self.thread_id => {
                self.drop_unfinished_replies();
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn drop_unfinished_replies(&mut self) {
        self.streaming
            .retain(|streaming| streaming.kind == StreamKind::ToolOutput);
    }

    pub(super) fn call_mut(&mut self, item_id: &str) -> Option<&mut ItemContent> {
        self.whole.remove(item_id);
        self.items
            .iter_mut()
            .find(|item| item.id == item_id)
            .map(|item| &mut item.content)
    }

    pub(super) fn stream(&mut self, deltas: Vec<StreamDelta>, cx: &mut Context<Self>) {
        let mut changed = false;
        for delta in deltas {
            if delta.thread_id != self.thread_id {
                continue;
            }
            changed = true;
            let index = match self
                .streaming
                .iter()
                .position(|streaming| streaming.item_id == delta.item_id)
            {
                Some(index) => index,
                None => {
                    self.streaming.push(Streaming {
                        item_id: delta.item_id.clone(),
                        kind: delta.kind,
                        text: String::new(),
                    });
                    self.streaming.len() - 1
                }
            };
            let streaming = &mut self.streaming[index];
            if delta.kind == StreamKind::ToolOutput {
                streaming.text = delta.delta;
            } else {
                streaming.text.push_str(&delta.delta);
            }
        }
        if changed {
            cx.notify();
        }
    }
}

/// Whether a page of the transcript left part of the item out: long output or a diff.
pub fn left_out(content: &ItemContent) -> bool {
    match content {
        ItemContent::ToolCall {
            output_omitted,
            detail,
            ..
        } => *output_omitted || matches!(detail, Some(ToolDetail::FileChange { diff: None, .. })),
        _ => false,
    }
}
