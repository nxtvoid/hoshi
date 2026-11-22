use crate::*;

impl Engine {
    pub(crate) fn browsing(&self) -> &Browsing {
        &self.browsing
    }

    /// The panel shows the thread's browser page, which then streams to it, or stopped.
    pub fn browser_view(&self, thread_id: &str, viewing: bool) -> Result<(), CommandError> {
        self.browsing()
            .view(thread_id, viewing)
            .map_err(|detail| CommandError::Internal { detail })
    }

    pub fn browser_open(&self, thread_id: &str, url: &str) -> Result<(), CommandError> {
        self.browsing()
            .user_open(thread_id, url)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// Back (`-1`), forward (`1`) or reload (`0`).
    pub fn browser_go(&self, thread_id: &str, delta: i32) -> Result<(), CommandError> {
        self.browsing()
            .user_go(thread_id, delta)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    pub fn browser_resize(
        &self,
        thread_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), CommandError> {
        self.browsing()
            .user_resize(thread_id, (width, height))
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The panel is `width` × `height` and the page fills it.
    pub fn browser_fit(
        &self,
        thread_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), CommandError> {
        self.browsing()
            .user_fit(thread_id, (width, height))
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    pub fn browser_input(&self, thread_id: &str, input: &BrowserInput) -> Result<(), CommandError> {
        self.browsing()
            .user_input(thread_id, input)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The screen showing the thread's page has `scale` device pixels per pixel, which the
    /// page then paints at.
    pub fn browser_scale(&self, thread_id: &str, scale: f64) -> Result<(), CommandError> {
        self.browsing()
            .user_scale(thread_id, scale)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// Runs `expression` in the thread's open page, awaiting a promise, and gives its value.
    pub fn browser_evaluate(
        &self,
        thread_id: &str,
        expression: &str,
    ) -> Result<serde_json::Value, CommandError> {
        self.browsing()
            .user_evaluate(thread_id, expression)
            .map_err(|detail| CommandError::BrowserFailed { detail })
    }

    /// The user closed the thread's page from the panel.
    pub fn browser_close(&self, thread_id: &str) {
        self.browsing().user_close(thread_id);
    }

    pub fn close_browser(&self) {
        let browsing = self.browsing.clone();
        let (closed, done) = mpsc::channel();
        thread::spawn(move || {
            browsing.close();
            let _ = closed.send(());
        });
        let _ = done.recv_timeout(BROWSER_CLOSE_WAIT);
    }
}
