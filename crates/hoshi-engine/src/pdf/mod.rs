//! PDFs drawn page by page for the side panel, and searched. One thread holds the PDF library
//! and every open document, as the library takes one caller at a time.

mod install;

use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

/// Matches kept from one search, so a word on every line of a long book stays cheap.
pub const MAX_MATCHES: usize = 1000;
/// The longest side a page is drawn at, in pixels; the GPU takes no larger image.
const MAX_SIDE: f32 = 8192.;
/// The most pixels a page is drawn with; a larger drawing is scaled up on screen.
const MAX_PIXELS: f32 = 16_000_000.;
/// The width a page is measured at to place what a search found on it.
const AREA_SCALE: f32 = 10_000.;

/// Why a PDF can't be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfFailure {
    /// No build of the PDF library for this system.
    Unsupported,
    /// The PDF library couldn't be fetched, or wasn't the build expected.
    Download,
    /// The PDF library is there but didn't load.
    Library,
    Password,
    Read,
}

/// A page's size in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// A page drawn: BGRA pixels, row after row.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// A part of a page, as fractions of its width and height from its top left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// Where a search matched: its page, and the areas its text covers there.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub page: usize,
    pub areas: Vec<Area>,
}

enum Request {
    Open {
        path: PathBuf,
        reply: Sender<Result<(u64, Vec<PageSize>), PdfFailure>>,
    },
    Render {
        id: u64,
        page: usize,
        width: u32,
        reply: Sender<Result<Bitmap, PdfFailure>>,
    },
    Search {
        id: u64,
        page: usize,
        query: String,
        reply: Sender<Vec<Match>>,
    },
    Close {
        id: u64,
    },
}

/// The PDF thread: which one it was started as, and where its requests go.
#[derive(Clone)]
struct Service {
    started: u64,
    requests: Sender<Request>,
}

impl Service {
    fn send(&self, request: Request) -> Result<(), PdfFailure> {
        self.requests.send(request).map_err(|_| self.lost())
    }

    /// Forgets this thread once it can't be reached, so the next PDF opened starts another.
    fn lost(&self) -> PdfFailure {
        let mut service = SERVICE.lock().unwrap_or_else(PoisonError::into_inner);
        if service
            .as_ref()
            .is_some_and(|current| current.started == self.started)
        {
            *service = None;
        }
        PdfFailure::Library
    }
}

struct Handle {
    id: u64,
    service: Service,
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.service.requests.send(Request::Close { id: self.id });
    }
}

/// An open PDF; it closes when its last clone goes.
#[derive(Clone)]
pub struct Document {
    handle: Arc<Handle>,
    pages: Arc<Vec<PageSize>>,
}

pub fn is_pdf(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.ends_with(['/', '\\']) && extension.eq_ignore_ascii_case("pdf")
    })
}

/// Whether opening a PDF first fetches the PDF library.
pub fn needs_download(home: &Path) -> bool {
    install::needs_download(&home.join("tools").join("pdfium"))
}

static SERVICE: Mutex<Option<Service>> = Mutex::new(None);
static STARTED: AtomicU64 = AtomicU64::new(0);

/// The PDF thread, started with the library under `home`, fetched first when it isn't there,
/// and again if it stopped.
fn service(home: &Path) -> Result<Service, PdfFailure> {
    let mut service = SERVICE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(started) = service.as_ref() {
        return Ok(started.clone());
    }
    let library = install::library(&home.join("tools").join("pdfium"))?;
    let (requests, received) = mpsc::channel();
    let (started, start) = mpsc::channel();
    thread::Builder::new()
        .name("pdf".into())
        .spawn(move || match Pdfium::bind_to_library(&library) {
            Ok(bindings) => {
                let _ = started.send(Ok(()));
                serve(&Pdfium::new(bindings), received);
            }
            Err(error) => {
                eprintln!("hoshi: could not load {}: {error}", library.display());
                let _ = started.send(Err(PdfFailure::Library));
            }
        })
        .map_err(|_| PdfFailure::Library)?;
    start.recv().map_err(|_| PdfFailure::Library)??;
    let started = Service {
        started: STARTED.fetch_add(1, Ordering::Relaxed),
        requests,
    };
    *service = Some(started.clone());
    Ok(started)
}

fn serve(pdfium: &Pdfium, requests: Receiver<Request>) {
    let mut documents: HashMap<u64, PdfDocument<'_>> = HashMap::new();
    let mut next = 0u64;
    for request in requests {
        let handled = panic::catch_unwind(AssertUnwindSafe(|| {
            handle(pdfium, request, &mut documents, &mut next)
        }));
        if handled.is_err() {
            eprintln!("hoshi: the PDF library failed on a request");
        }
    }
}

fn handle<'a>(
    pdfium: &'a Pdfium,
    request: Request,
    documents: &mut HashMap<u64, PdfDocument<'a>>,
    next: &mut u64,
) {
    match request {
        Request::Open { path, reply } => {
            let opened = pdfium
                .load_pdf_from_file(&path, None)
                .map_err(|error| match error {
                    PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                        PdfFailure::Password
                    }
                    _ => PdfFailure::Read,
                })
                .and_then(|document| {
                    let sizes = document
                        .pages()
                        .page_sizes()
                        .map_err(|_| PdfFailure::Read)?
                        .into_iter()
                        .map(|rect| PageSize {
                            width: rect.width().value,
                            height: rect.height().value,
                        })
                        .collect();
                    Ok((document, sizes))
                });
            let _ = reply.send(opened.map(|(document, sizes)| {
                *next += 1;
                documents.insert(*next, document);
                (*next, sizes)
            }));
        }
        Request::Render {
            id,
            page,
            width,
            reply,
        } => {
            let drawn = documents
                .get(&id)
                .ok_or(PdfFailure::Read)
                .and_then(|document| render(document, page, width));
            let _ = reply.send(drawn);
        }
        Request::Search {
            id,
            page,
            query,
            reply,
        } => {
            let found = documents
                .get(&id)
                .map(|document| search(document, page, &query))
                .unwrap_or_default();
            let _ = reply.send(found);
        }
        Request::Close { id } => {
            documents.remove(&id);
        }
    }
}

fn render(document: &PdfDocument<'_>, page: usize, width: u32) -> Result<Bitmap, PdfFailure> {
    let page = document
        .pages()
        .get(page as PdfPageIndex)
        .map_err(|_| PdfFailure::Read)?;
    let width = drawn_width(width, page.width().value / page.height().value.max(1.))
        .ok_or(PdfFailure::Read)?;
    let config = PdfRenderConfig::new()
        .set_target_width(width as Pixels)
        .render_form_data(true);
    let bitmap = page
        .render_with_config(&config)
        .map_err(|_| PdfFailure::Read)?;
    Ok(Bitmap {
        width: bitmap.width() as u32,
        height: bitmap.height() as u32,
        bgra: bitmap.as_raw_bytes(),
    })
}

/// The width a page `wanted` pixels wide is drawn at, with `aspect` its width over its height:
/// narrower when its longest side or its pixels would pass the most drawn. `None` for a page so
/// thin that even a pixel wide it would be taller than that.
fn drawn_width(wanted: u32, aspect: f32) -> Option<u32> {
    if !aspect.is_finite() || aspect <= 0. {
        return None;
    }
    let width = (wanted.max(1) as f32)
        .min(MAX_SIDE)
        .min(MAX_SIDE * aspect)
        .min((MAX_PIXELS * aspect).sqrt())
        .floor();
    (width >= 1.).then_some(width as u32)
}

fn search(document: &PdfDocument<'_>, index: usize, query: &str) -> Vec<Match> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(page) = document.pages().get(index as PdfPageIndex) else {
        return Vec::new();
    };
    let aspect = page.width().value / page.height().value.max(1.);
    let shown = PdfRenderConfig::new().set_target_width(AREA_SCALE as Pixels);
    let size = (AREA_SCALE, (AREA_SCALE / aspect.max(0.001)).round());
    let Ok(text) = page.text() else {
        return Vec::new();
    };
    let Ok(search) = text.search(query, &PdfSearchOptions::new()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    while let Some(segments) = search.find_next() {
        let areas = segments
            .iter()
            .filter_map(|segment| area(&page, segment.bounds(), &shown, size))
            .collect();
        found.push(Match { page: index, areas });
        if found.len() >= MAX_MATCHES {
            break;
        }
    }
    found
}

/// Where `bounds`, in the page's own space, fall on the page as drawn with `shown`, `size`
/// pixels: past its box's origin, and turned with the page.
fn area(
    page: &PdfPage<'_>,
    bounds: PdfRect,
    shown: &PdfRenderConfig,
    size: (f32, f32),
) -> Option<Area> {
    let (x1, y1) = page
        .points_to_pixels(bounds.left(), bounds.top(), shown)
        .ok()?;
    let (x2, y2) = page
        .points_to_pixels(bounds.right(), bounds.bottom(), shown)
        .ok()?;
    let (width, height) = (size.0.max(1.), size.1.max(1.));
    Some(Area {
        left: x1.min(x2) as f32 / width,
        top: y1.min(y2) as f32 / height,
        width: x1.abs_diff(x2) as f32 / width,
        height: y1.abs_diff(y2) as f32 / height,
    })
}

impl Document {
    /// Opens the PDF at `path`, with the PDF library hoshi keeps under `home`.
    /// A PDF thread found stopped is started again for it.
    pub fn open(home: &Path, path: &Path) -> Result<Self, PdfFailure> {
        let mut retried = false;
        loop {
            let service = service(home)?;
            let (reply, answer) = mpsc::channel();
            let sent = service.send(Request::Open {
                path: path.to_owned(),
                reply,
            });
            if sent.is_err() && !retried {
                retried = true;
                continue;
            }
            sent?;
            let (id, pages) = answer.recv().map_err(|_| PdfFailure::Library)??;
            return Ok(Self {
                handle: Arc::new(Handle { id, service }),
                pages: Arc::new(pages),
            });
        }
    }

    pub fn pages(&self) -> &[PageSize] {
        &self.pages
    }

    /// Page `page` drawn `width` pixels wide, or narrower when that would be too large.
    pub fn render(&self, page: usize, width: u32) -> Result<Bitmap, PdfFailure> {
        let (reply, answer) = mpsc::channel();
        self.handle.service.send(Request::Render {
            id: self.handle.id,
            page,
            width,
            reply,
        })?;
        answer.recv().map_err(|_| PdfFailure::Library)?
    }

    /// Where `query` is on page `page`, in reading order; at most [`MAX_MATCHES`], and none
    /// for blank text.
    pub fn search_page(&self, page: usize, query: &str) -> Vec<Match> {
        let (reply, answer) = mpsc::channel();
        let sent = self.handle.service.send(Request::Search {
            id: self.handle.id,
            page,
            query: query.to_owned(),
            reply,
        });
        if sent.is_err() {
            return Vec::new();
        }
        answer.recv().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
