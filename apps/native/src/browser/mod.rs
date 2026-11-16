pub mod layout;
mod mini;
mod page;
mod page_view;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use gpui_kit::*;
use hoshi_core::{BrowserFrame, BrowserInput, BrowserPage};
use hoshi_engine::Engine;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub use mini::{MiniBrowser, Obstacle, obstacle};
pub use page::{PAGE_SIZES, key_input, page_bounds, page_point, page_scroll};
pub use page_view::{PageFit, PageView, ViewportChanged};

use layout::{Interaction, Placement};

/// How long the agent's last action stays marked on the page.
pub const AGENT_SHOWN: Duration = Duration::from_secs(4);

/// A frame on screen, decoded once, and the page's size in CSS pixels.
pub struct ShownFrame {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

pub struct ShownPage {
    pub page: BrowserPage,
    /// When the agent last acted on it.
    pub agent_at: Option<Instant>,
    /// Whether the page takes the panel's size; `None` until the engine says.
    pub fitted: Option<bool>,
}

pub enum BrowsersEvent {
    /// The agent opened the thread's page for the first time.
    AgentOpened(String),
    /// The page left the panel to float over the chat.
    Floated(String),
    /// The floating page goes to the panel.
    Docked(String),
}

#[derive(Debug)]
enum Request {
    View(bool),
    Open(String),
    Go(i32),
    Fit(u32, u32),
    Resize(u32, u32),
    Scale(f32),
    Input(BrowserInput),
    Evaluate(String, oneshot::Sender<serde_json::Value>),
    Close,
}

/// Each thread's headless browser page as the engine last told, and what goes back to it.
/// Each page's requests run in order on a thread of their own, as pages are slow.
pub struct Browsers {
    pages: HashMap<String, ShownPage>,
    frames: HashMap<String, ShownFrame>,
    /// The frame each one replaced, kept until the next so a frame still drawing it can finish.
    replaced: HashMap<String, Arc<RenderImage>>,
    decoding: HashSet<String>,
    /// The newest frame that came while the one before was decoding.
    waiting: HashMap<String, BrowserFrame>,
    /// How many views show each thread's page; it streams while any does.
    viewers: HashMap<String, usize>,
    failed: HashSet<String>,
    /// Each thread's page floating over the chat, where the user left it.
    minis: HashMap<String, Placement>,
    /// The scale of the screen each page shows on, as last sent.
    scales: HashMap<String, f32>,
    requests: Sender<(String, Request)>,
}

impl EventEmitter<BrowsersEvent> for Browsers {}

impl Browsers {
    pub fn new(engine: Arc<Engine>, cx: &mut Context<Self>) -> Self {
        let (failures, mut failed) = unbounded();
        let requests = run_requests(engine, failures);
        cx.spawn(async move |this, cx| {
            while let Some(thread_id) = failed.next().await {
                let gone = this
                    .update(cx, |this, cx| {
                        this.failed.insert(thread_id);
                        cx.notify();
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        })
        .detach();
        Self {
            pages: HashMap::new(),
            frames: HashMap::new(),
            replaced: HashMap::new(),
            decoding: HashSet::new(),
            waiting: HashMap::new(),
            viewers: HashMap::new(),
            failed: HashSet::new(),
            minis: HashMap::new(),
            scales: HashMap::new(),
            requests,
        }
    }

    /// Browsers whose requests go nowhere but to the returned list.
    #[cfg(test)]
    pub fn detached() -> (Self, SentRequests) {
        let (requests, sent) = std::sync::mpsc::channel();
        let browsers = Self {
            pages: HashMap::new(),
            frames: HashMap::new(),
            replaced: HashMap::new(),
            decoding: HashSet::new(),
            waiting: HashMap::new(),
            viewers: HashMap::new(),
            failed: HashSet::new(),
            minis: HashMap::new(),
            scales: HashMap::new(),
            requests,
        };
        (browsers, SentRequests(sent))
    }

    #[cfg(test)]
    pub fn fail(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.failed.insert(thread_id.to_owned());
        cx.notify();
    }

    /// Drops what it keeps of a deleted thread's page, its floating player's place included.
    pub fn forget(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.pages.remove(thread_id);
        self.frames.remove(thread_id);
        self.replaced.remove(thread_id);
        self.decoding.remove(thread_id);
        self.waiting.remove(thread_id);
        self.viewers.remove(thread_id);
        self.failed.remove(thread_id);
        self.minis.remove(thread_id);
        self.scales.remove(thread_id);
        cx.notify();
    }

    pub fn page(&self, thread_id: &str) -> Option<&ShownPage> {
        self.pages.get(thread_id)
    }

    pub fn frame(&self, thread_id: &str) -> Option<&ShownFrame> {
        self.frames.get(thread_id)
    }

    /// The last thing asked of the thread's page failed.
    pub fn failed(&self, thread_id: &str) -> bool {
        self.failed.contains(thread_id)
    }

    #[cfg(test)]
    pub fn viewers(&self, thread_id: &str) -> usize {
        self.viewers.get(thread_id).copied().unwrap_or(0)
    }

    /// Whether the agent acted on the thread's page in the last few seconds.
    pub fn agent_active(&self, thread_id: &str) -> bool {
        self.pages
            .get(thread_id)
            .and_then(|page| page.agent_at)
            .is_some_and(|at| at.elapsed() < AGENT_SHOWN)
    }

    pub fn receive_frame(&mut self, frame: BrowserFrame, cx: &mut Context<Self>) {
        if self.decoding.contains(&frame.thread_id) {
            self.waiting.insert(frame.thread_id.clone(), frame);
        } else {
            self.decode(frame, cx);
        }
    }

    fn decode(&mut self, frame: BrowserFrame, cx: &mut Context<Self>) {
        let BrowserFrame {
            thread_id,
            jpeg,
            width,
            height,
        } = frame;
        self.decoding.insert(thread_id.clone());
        cx.spawn(async move |this, cx| {
            let image = cx.background_spawn(async move { decode_jpeg(&jpeg) }).await;
            this.update(cx, |this, cx| {
                this.decoding.remove(&thread_id);
                if let Some(image) = image {
                    let shown = ShownFrame {
                        image,
                        width,
                        height,
                    };
                    if let Some(before) = this.frames.insert(thread_id.clone(), shown)
                        && let Some(old) = this.replaced.insert(thread_id.clone(), before.image)
                    {
                        cx.drop_image(old, None);
                    }
                    cx.notify();
                }
                if let Some(next) = this.waiting.remove(&thread_id) {
                    this.decode(next, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn receive_page(&mut self, page: BrowserPage, cx: &mut Context<Self>) {
        let thread_id = page.thread_id.clone();
        if !page.open {
            self.minis.remove(&thread_id);
            self.pages.remove(&thread_id);
            self.waiting.remove(&thread_id);
            for image in [
                self.frames.remove(&thread_id).map(|frame| frame.image),
                self.replaced.remove(&thread_id),
            ]
            .into_iter()
            .flatten()
            {
                cx.drop_image(image, None);
            }
            cx.notify();
            return;
        }
        let before = self.pages.get(&thread_id);
        let agent_at = if page.by_agent {
            Some(Instant::now())
        } else {
            before.and_then(|before| before.agent_at)
        };
        let fitted = page.fitted.or(before.and_then(|before| before.fitted));
        if page.by_agent {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AGENT_SHOWN).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
        let opened = page.opened;
        self.pages.insert(
            thread_id.clone(),
            ShownPage {
                page,
                agent_at,
                fitted,
            },
        );
        cx.notify();
        if opened {
            cx.emit(BrowsersEvent::AgentOpened(thread_id));
        }
    }

    /// Where the thread's floating page is, while it floats.
    pub fn mini(&self, thread_id: &str) -> Option<Placement> {
        self.minis.get(thread_id).copied()
    }

    pub fn open_mini(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if !self.minis.contains_key(thread_id) {
            self.minis
                .insert(thread_id.to_owned(), Placement::default());
            cx.notify();
        }
    }

    pub fn close_mini(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if self.minis.remove(thread_id).is_some() {
            cx.notify();
        }
    }

    pub fn move_mini(&mut self, thread_id: &str, position: layout::Point, cx: &mut Context<Self>) {
        if let Some(placement) = self.minis.get_mut(thread_id) {
            placement.position = Some(position);
            placement.interaction = Interaction::Drag;
            cx.notify();
        }
    }

    pub fn resize_mini(
        &mut self,
        thread_id: &str,
        width: f64,
        position: layout::Point,
        cx: &mut Context<Self>,
    ) {
        if let Some(placement) = self.minis.get_mut(thread_id) {
            *placement = Placement {
                position: Some(position),
                width: Some(width),
                interaction: Interaction::Resize,
            };
            cx.notify();
        }
    }

    /// Takes the page out of the panel to float over the chat.
    pub fn float(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.open_mini(thread_id, cx);
        cx.emit(BrowsersEvent::Floated(thread_id.to_owned()));
    }

    /// Sends the floating page to the panel.
    pub fn dock(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.close_mini(thread_id, cx);
        cx.emit(BrowsersEvent::Docked(thread_id.to_owned()));
    }

    /// A view starts or stops showing the thread's page.
    pub fn view(&mut self, thread_id: &str, viewing: bool) {
        let count = self.viewers.entry(thread_id.to_owned()).or_default();
        if viewing {
            *count += 1;
            if *count == 1 {
                self.request(thread_id, Request::View(true));
            }
        } else {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.viewers.remove(thread_id);
                self.request(thread_id, Request::View(false));
            }
        }
    }

    pub fn open(&mut self, thread_id: &str, address: &str, cx: &mut Context<Self>) {
        let address = address.trim();
        if address.is_empty() {
            return;
        }
        if self.failed.remove(thread_id) {
            cx.notify();
        }
        self.request(thread_id, Request::Open(address.to_owned()));
    }

    /// Back (`-1`), forward (`1`) or reload (`0`).
    pub fn go(&self, thread_id: &str, delta: i32) {
        self.request(thread_id, Request::Go(delta));
    }

    /// The panel is `width` × `height` and the page fills it.
    pub fn fit(&self, thread_id: &str, width: u32, height: u32) {
        self.request(thread_id, Request::Fit(width, height));
    }

    pub fn resize(&self, thread_id: &str, width: u32, height: u32) {
        self.request(thread_id, Request::Resize(width, height));
    }

    /// The page shows on a screen of `scale` device pixels per pixel, which it paints at.
    pub fn scale(&mut self, thread_id: &str, scale: f32) {
        if self.scales.get(thread_id) != Some(&scale) {
            self.scales.insert(thread_id.to_owned(), scale);
            self.request(thread_id, Request::Scale(scale));
        }
    }

    pub fn input(&self, thread_id: &str, input: BrowserInput) {
        self.request(thread_id, Request::Input(input));
    }

    /// Runs `expression` in the open page after what was asked before it; its value, or
    /// nothing when the page isn't open or the script failed.
    pub fn evaluate(
        &self,
        thread_id: &str,
        expression: &str,
    ) -> oneshot::Receiver<serde_json::Value> {
        let (sender, receiver) = oneshot::channel();
        self.request(thread_id, Request::Evaluate(expression.to_owned(), sender));
        receiver
    }

    /// Closes the page, as when the user closes it.
    pub fn close(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        if self.failed.remove(thread_id) {
            cx.notify();
        }
        self.request(thread_id, Request::Close);
    }

    fn request(&self, thread_id: &str, request: Request) {
        let _ = self.requests.send((thread_id.to_owned(), request));
    }
}

#[cfg(test)]
pub struct SentRequests(std::sync::mpsc::Receiver<(String, Request)>);

#[cfg(test)]
impl SentRequests {
    /// What was asked since last read, as `key: request`.
    pub fn take(&self) -> Vec<String> {
        self.0
            .try_iter()
            .map(|(key, request)| format!("{key}: {request:?}"))
            .collect()
    }
}

/// How long a page's request thread waits for more before it ends.
const WORKER_IDLE: Duration = Duration::from_secs(30);

type Workers = Arc<std::sync::Mutex<HashMap<String, Sender<Request>>>>;

fn run_requests(
    engine: Arc<Engine>,
    failures: UnboundedSender<String>,
) -> Sender<(String, Request)> {
    let (sender, receiver) = std::sync::mpsc::channel::<(String, Request)>();
    let spawned = std::thread::Builder::new()
        .name("hoshi-browser".into())
        .spawn(move || {
            let workers = Workers::default();
            for (thread_id, request) in receiver {
                let mut running = lock(&workers);
                let request = match running.get(&thread_id) {
                    Some(worker) => match worker.send(request) {
                        Ok(()) => continue,
                        Err(std::sync::mpsc::SendError(request)) => request,
                    },
                    None => request,
                };
                let worker = start_worker(
                    thread_id.clone(),
                    engine.clone(),
                    failures.clone(),
                    workers.clone(),
                );
                if let Some(worker) = worker {
                    let _ = worker.send(request);
                    running.insert(thread_id, worker);
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("hoshi-native: could not start the browser thread: {error}");
    }
    sender
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs one page's requests in order, apart from the other pages, so a slow page keeps no
/// other waiting. It ends once idle, leaving the list under its lock so no request is lost.
fn start_worker(
    thread_id: String,
    engine: Arc<Engine>,
    failures: UnboundedSender<String>,
    workers: Workers,
) -> Option<Sender<Request>> {
    let (sender, receiver) = std::sync::mpsc::channel::<Request>();
    let spawned = std::thread::Builder::new()
        .name("hoshi-browser-page".into())
        .spawn(move || {
            loop {
                let request = match receiver.recv_timeout(WORKER_IDLE) {
                    Ok(request) => request,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        let mut running = lock(&workers);
                        match receiver.try_recv() {
                            Ok(request) => {
                                drop(running);
                                request
                            }
                            Err(_) => {
                                running.remove(&thread_id);
                                return;
                            }
                        }
                    }
                };
                run_request(&engine, &thread_id, request, &failures);
            }
        });
    match spawned {
        Ok(_) => Some(sender),
        Err(error) => {
            eprintln!("hoshi-native: could not start a browser page thread: {error}");
            None
        }
    }
}

fn run_request(
    engine: &Engine,
    thread_id: &str,
    request: Request,
    failures: &UnboundedSender<String>,
) {
    let (result, reported) = match request {
        Request::View(viewing) => (engine.browser_view(thread_id, viewing), false),
        Request::Open(address) => (engine.browser_open(thread_id, &address), true),
        Request::Go(delta) => (engine.browser_go(thread_id, delta), true),
        Request::Fit(width, height) => (engine.browser_fit(thread_id, width, height), false),
        Request::Resize(width, height) => (engine.browser_resize(thread_id, width, height), true),
        Request::Scale(scale) => (engine.browser_scale(thread_id, f64::from(scale)), false),
        Request::Input(input) => (engine.browser_input(thread_id, &input), true),
        Request::Evaluate(expression, reply) => {
            if let Ok(value) = engine.browser_evaluate(thread_id, &expression) {
                let _ = reply.send(value);
            }
            (Ok(()), false)
        }
        Request::Close => {
            engine.browser_close(thread_id);
            (Ok(()), false)
        }
    };
    if let Err(error) = result {
        eprintln!("hoshi-native: browser request failed: {error:?}");
        if reported {
            let _ = failures.unbounded_send(thread_id.to_owned());
        }
    }
}

/// A JPEG as GPUI draws it, BGRA.
fn decode_jpeg(jpeg: &[u8]) -> Option<Arc<RenderImage>> {
    let mut pixels = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .ok()?
        .into_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

#[cfg(test)]
mod tests {
    use super::{Browsers, decode_jpeg};

    #[test]
    fn a_page_hears_of_its_screen_s_scale_once_per_change() {
        let (mut browsers, sent) = Browsers::detached();
        browsers.scale("t", 1.5);
        browsers.scale("t", 1.5);
        browsers.scale("other", 1.5);
        browsers.scale("t", 2.);
        assert_eq!(
            sent.take(),
            ["t: Scale(1.5)", "other: Scale(1.5)", "t: Scale(2.0)"]
        );
    }

    #[test]
    fn a_frame_decodes_to_bgra_at_its_size() {
        let mut jpeg = Vec::new();
        let red = image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]));
        image::DynamicImage::ImageRgb8(red)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let image = decode_jpeg(&jpeg).unwrap();
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (4, 2));
        let bytes = image.as_bytes(0).unwrap();
        assert!(bytes[0] < 30 && bytes[2] > 220, "{:?}", &bytes[..4]);
        assert!(decode_jpeg(b"not a jpeg").is_none());
    }
}
