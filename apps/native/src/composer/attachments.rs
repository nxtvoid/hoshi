//! Images and files added to the draft: picked with the clip, dropped on the composer or
//! pasted. The engine saves each one as soon as it is added; the message carries them when it
//! is sent.

use super::Composer;
use super::chips::{ChipInfo, attachment_chip_id, attachment_chip_source, attachment_key};
use crate::m;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use gpui_kit::component::Icon;
use gpui_kit::component::input::InlineToken;
use gpui_kit::*;
use hoshi_core::MessageAttachment;
use hoshi_engine::MAX_FILE_BYTES;
use hoshi_ui::prelude::*;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The most images and files one message carries.
pub const MAX_ATTACHMENTS: usize = 100;
/// Providers refuse larger images; a larger one is scaled down first.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Larger images are scaled down before anything else.
const MAX_EDGE: u32 = 4096;
/// A pasted text this long goes as a file, so the prompt stays readable.
pub const PASTED_TEXT_ATTACHMENT_BYTES: usize = 32 * 1024;

/// `pasted-text.txt`, or the first `pasted-text-<n>.txt` none of `taken` has.
pub fn pasted_text_file_name<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let names: Vec<String> = taken.into_iter().map(str::to_lowercase).collect();
    if !names.iter().any(|name| name == "pasted-text.txt") {
        return "pasted-text.txt".into();
    }
    (2..)
        .map(|sequence| format!("pasted-text-{sequence}.txt"))
        .find(|name| !names.contains(name))
        .unwrap_or_default()
}

/// Where an attachment's bytes come from.
#[derive(Clone, Debug)]
pub enum Source {
    Path(PathBuf),
    /// An image pasted from the clipboard.
    Pasted(Arc<Image>),
    /// A text too long to paste inline, as a file.
    Text {
        name: String,
        text: Arc<str>,
    },
}

impl Source {
    fn name(&self) -> String {
        match self {
            Self::Path(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into()),
            Self::Pasted(image) => format!("image.{}", image.format.extension()),
            Self::Text { name, .. } => name.clone(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Path(path) => std::fs::metadata(path).map_or(0, |meta| meta.len()),
            Self::Pasted(image) => image.bytes.len() as u64,
            Self::Text { text, .. } => text.len() as u64,
        }
    }

    fn read(&self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Path(path) => std::fs::read(path),
            Self::Pasted(image) => Ok(image.bytes.clone()),
            Self::Text { text, .. } => Ok(text.as_bytes().to_vec()),
        }
    }

    pub fn preview(&self) -> ImageSource {
        match self {
            Self::Path(path) => path.clone().into(),
            Self::Pasted(image) => image.clone().into(),
            Self::Text { .. } => Arc::new(Image::from_bytes(ImageFormat::Png, Vec::new())).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Clone, Debug)]
pub struct DraftAttachment {
    pub key: u64,
    pub kind: AttachmentKind,
    pub name: String,
    pub size: u64,
    pub source: Source,
    /// Set once the engine saved it.
    pub saved: Option<MessageAttachment>,
    pub failed: bool,
    /// Shown as a chip in the text; without its chip, the message leaves it out.
    pub chipped: bool,
}

/// A draft's attachment as it comes back from a kept draft, read from where the engine saved
/// it.
pub fn restored(kept: &crate::drafts::Attachment, path: PathBuf) -> Option<DraftAttachment> {
    let (kind, name, size) = match &kept.attachment {
        MessageAttachment::Image { name, size, .. } => (AttachmentKind::Image, name, *size),
        MessageAttachment::File { name, size, .. } => (AttachmentKind::File, name, *size),
        _ => return None,
    };
    Some(DraftAttachment {
        key: kept.key,
        kind,
        name: name.clone(),
        size,
        source: Source::Path(path),
        saved: Some(kept.attachment.clone()),
        failed: false,
        chipped: kept.chipped,
    })
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Whether `name` is an image the agent sees, redrawn as PNG or JPEG when it has to be;
/// anything else goes as a file it reads from disk.
pub fn is_image(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "pnm"
    )
}

fn sendable_type(name: &str) -> Option<&'static str> {
    match extension(name).as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// An image as every provider takes it: its name, type and bytes. One of another type, or too
/// large, is redrawn as PNG, or as JPEG when even that is too large, and scaled down until it
/// fits. `None` when it can't be read as an image.
pub fn prepare_image(name: &str, bytes: Vec<u8>) -> Option<(String, &'static str, Vec<u8>)> {
    if let Some(mime_type) = sendable_type(name)
        && bytes.len() <= MAX_IMAGE_BYTES
    {
        return Some((name.to_owned(), mime_type, bytes));
    }
    let image = image::load_from_memory(&bytes).ok()?;
    let longest = image.width().max(image.height()).max(1);
    let mut scale = (MAX_EDGE as f32 / longest as f32).min(1.);
    let png = extension(name) == "png" || bytes.len() <= MAX_IMAGE_BYTES;
    let (format, mime_type, suffix) = if png {
        (image::ImageFormat::Png, "image/png", "png")
    } else {
        (image::ImageFormat::Jpeg, "image/jpeg", "jpg")
    };
    let stem = Path::new(name).file_stem().map_or_else(
        || "image".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    for _ in 0..6 {
        let width = ((image.width() as f32 * scale).round() as u32).max(1);
        let height = ((image.height() as f32 * scale).round() as u32).max(1);
        let drawn = if scale < 1. {
            image.resize(width, height, image::imageops::FilterType::Triangle)
        } else {
            image.clone()
        };
        let drawn = if png {
            drawn
        } else {
            image::DynamicImage::ImageRgb8(drawn.to_rgb8())
        };
        let mut encoded = Vec::new();
        drawn
            .write_to(&mut Cursor::new(&mut encoded), format)
            .ok()?;
        if encoded.len() <= MAX_IMAGE_BYTES {
            return Some((format!("{stem}.{suffix}"), mime_type, encoded));
        }
        scale *= 0.7;
    }
    None
}

/// The type a file goes with, from its name.
pub fn file_type(name: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// The largest attachment, as the error names it.
fn max_file_label() -> String {
    format!("{} MB", MAX_FILE_BYTES / (1024 * 1024))
}

/// The sources small enough to attach, and the first one too large to.
pub fn within_limit(sources: Vec<Source>) -> (Vec<Source>, Option<Source>) {
    let (fit, too_large): (Vec<_>, Vec<_>) = sources
        .into_iter()
        .partition(|source| source.size() <= MAX_FILE_BYTES as u64);
    (fit, too_large.into_iter().next())
}

/// A size such as "8 kB" or "1.4 MB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("GB", 1_000_000_000), ("MB", 1_000_000), ("kB", 1_000)];
    match UNITS.iter().find(|(_, size)| bytes >= *size) {
        Some((unit, size)) if bytes >= 1_000_000 => {
            let value = bytes as f64 / *size as f64;
            let shown = format!("{value:.1}");
            format!("{} {unit}", shown.trim_end_matches(".0"))
        }
        Some((unit, size)) => format!("{} {unit}", (bytes as f64 / *size as f64).round()),
        None => format!("{bytes} B"),
    }
}

impl Composer {
    pub(super) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                this.update_in(cx, |this, window, cx| this.attach_paths(paths, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn attach_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attach(paths.into_iter().map(Source::Path).collect(), window, cx);
    }

    /// Takes the images and files on the clipboard; `false` leaves the paste to the editor.
    pub(super) fn paste_attachments(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let sources: Vec<Source> = item
            .entries()
            .iter()
            .flat_map(|entry| match entry {
                ClipboardEntry::Image(image) => vec![Source::Pasted(Arc::new(image.clone()))],
                ClipboardEntry::ExternalPaths(paths) => paths
                    .paths()
                    .iter()
                    .filter(|path| path.is_file())
                    .cloned()
                    .map(Source::Path)
                    .collect(),
                ClipboardEntry::String(_) => Vec::new(),
            })
            .collect();
        if sources.is_empty() {
            let large = item.text().filter(|text| {
                text.len() >= PASTED_TEXT_ATTACHMENT_BYTES && self.pending_request(cx).is_none()
            });
            let Some(text) = large else {
                return false;
            };
            let name = pasted_text_file_name(
                self.attachments
                    .iter()
                    .map(|attachment| attachment.name.as_str()),
            );
            let size = format_size(text.len() as u64);
            self.attach(
                vec![Source::Text {
                    name: name.clone(),
                    text: text.into(),
                }],
                window,
                cx,
            );
            let shortcut = if cfg!(target_os = "macos") {
                "⌘⇧V"
            } else {
                "Ctrl+Shift+V"
            };
            self.show_notice(
                format!(
                    "{} · {}",
                    m::composer_pasted_text_attached(&name),
                    m::composer_pasted_text_keep_inline(shortcut, &size)
                ),
                cx,
            );
            return true;
        }
        self.attach(sources, window, cx);
        true
    }

    /// An image opens large over the window, a file where the system or the editor opens it.
    pub(super) fn open_attachment(&self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(attachment) = self
            .attachments
            .iter()
            .find(|attachment| attachment.key == key)
        else {
            return;
        };
        if attachment.kind == AttachmentKind::Image {
            crate::markdown::show_image_source(
                attachment.source.preview(),
                attachment.name.clone(),
                window,
                cx,
            );
            return;
        }
        let path = match (&attachment.saved, &attachment.source) {
            (Some(saved), _) => self.workspace.read(cx).engine.attachment_path(saved),
            (None, Source::Path(path)) => Some(path.to_string_lossy().into_owned()),
            _ => None,
        };
        if let Some(path) = path {
            crate::markdown::open_local(path, cx);
        }
    }

    /// Ctrl+Shift+V: pastes the clipboard's text inline however long it is.
    pub(super) fn paste_as_text(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        self.editor
            .update(cx, |editor, cx| editor.insert_text(&text, window, cx));
        true
    }

    pub(super) fn attach(
        &mut self,
        sources: Vec<Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_text = !self.editor.read(cx).is_empty(cx);
        let mut chips = Vec::new();
        let (sources, too_large) = within_limit(sources);
        if let Some(source) = too_large {
            self.error =
                Some(m::composer_attachment_too_large(max_file_label(), source.name()).into());
        }
        let room = MAX_ATTACHMENTS.saturating_sub(self.attachments.len());
        if sources.len() > room {
            self.error = Some(m::composer_attachments_too_many(MAX_ATTACHMENTS).into());
        }
        for source in sources.into_iter().take(room) {
            let name = source.name();
            let image = is_image(&name);
            self.next_attachment += 1;
            let key = self.next_attachment;
            self.attachments.push(DraftAttachment {
                key,
                kind: if image {
                    AttachmentKind::Image
                } else {
                    AttachmentKind::File
                },
                name: name.clone(),
                size: source.size(),
                source: source.clone(),
                saved: None,
                failed: false,
                chipped: false,
            });
            if !image || has_text {
                let kind = if image {
                    AttachmentKind::Image
                } else {
                    AttachmentKind::File
                };
                chips.push(
                    InlineToken::new(attachment_chip_id(key), attachment_chip_source(kind, key))
                        .with_label(name.clone()),
                );
            }
            let engine = self.workspace.read(cx).engine.clone();
            cx.spawn(async move |this, cx| {
                let saved = cx
                    .background_spawn(async move {
                        let bytes = source
                            .read()
                            .map_err(|_| m::composer_attachment_empty(&name))?;
                        if bytes.is_empty() {
                            return Err(m::composer_attachment_empty(&name));
                        }
                        let saved = if image {
                            let (name, mime_type, bytes) = prepare_image(&name, bytes)
                                .ok_or_else(|| m::composer_attachment_empty(&name))?;
                            engine.save_image(&name, mime_type, &STANDARD.encode(&bytes))
                        } else {
                            engine.save_file(&name, file_type(&name), &bytes)
                        };
                        saved.map_err(|error| {
                            eprintln!("hoshi: could not save an attachment: {error}");
                            m::composer_attachment_failed(&name)
                        })
                    })
                    .await;
                this.update(cx, |this, cx| {
                    let Some(attachment) = this
                        .attachments
                        .iter_mut()
                        .find(|attachment| attachment.key == key)
                    else {
                        this.saved_while_away(key, saved, cx);
                        return;
                    };
                    match saved {
                        Ok(saved) => {
                            attachment.saved = Some(saved);
                            this.keep_draft(cx);
                        }
                        Err(error) => {
                            attachment.failed = true;
                            this.error = Some(error.into());
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        self.insert_chips(chips, window, cx);
        cx.notify();
    }

    /// Places attachment chips at the caret, each followed by a space.
    fn insert_chips(
        &mut self,
        chips: Vec<InlineToken>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut placed = Vec::new();
        let Some(input) = self.input(cx) else {
            return;
        };
        input.update(cx, |editor, cx| {
            for chip in chips {
                let cursor = editor.cursor();
                let id = chip.id().to_string();
                if editor
                    .replace_range_with_token(cursor..cursor, chip, window, cx)
                    .is_ok()
                {
                    editor.insert(" ", window, cx);
                    placed.extend(attachment_key(&id));
                }
            }
        });
        for attachment in &mut self.attachments {
            if placed.contains(&attachment.key) {
                attachment.chipped = true;
            }
        }
    }

    /// The keys of the attachment chips in the text, in order, each once.
    pub(super) fn chip_keys(&self, cx: &App) -> Vec<u64> {
        let mut keys = Vec::new();
        for span in self.editor.read(cx).tokens(cx) {
            if let Some(key) = attachment_key(span.token().id())
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        keys
    }

    /// What the message goes with: an attachment whose chip was taken out of the text is
    /// left out.
    pub(super) fn attached(&self, cx: &App) -> Vec<&DraftAttachment> {
        let keys = self.chip_keys(cx);
        self.attachments
            .iter()
            .filter(|attachment| !attachment.chipped || keys.contains(&attachment.key))
            .collect()
    }

    /// The attachments in the order they are sent: those with a chip as the text shows them,
    /// then the rest, so `[Image #n]` names the image sent nth.
    pub(super) fn ordered_attachments(&self, cx: &App) -> Vec<DraftAttachment> {
        let attached = self.attached(cx);
        let keys = self.chip_keys(cx);
        let in_text = keys
            .iter()
            .filter_map(|key| attached.iter().find(|attachment| attachment.key == *key));
        let rest = attached
            .iter()
            .filter(|attachment| !keys.contains(&attachment.key));
        in_text
            .chain(rest)
            .map(|attachment| (*attachment).clone())
            .collect()
    }

    pub(super) fn chip_infos(&self) -> HashMap<u64, ChipInfo> {
        self.attachments
            .iter()
            .map(|attachment| {
                (
                    attachment.key,
                    ChipInfo {
                        kind: attachment.kind,
                        size: attachment.size,
                        source: attachment.source.clone(),
                    },
                )
            })
            .collect()
    }

    /// An image or file finished saving after the composer left its draft: it joins the kept
    /// draft, or goes when nothing holds it any more.
    fn saved_while_away(
        &mut self,
        key: u64,
        saved: Result<MessageAttachment, String>,
        cx: &mut Context<Self>,
    ) {
        let parked = self.parked.iter_mut().find_map(|(draft, parked)| {
            let index = parked.iter().position(|attachment| attachment.key == key)?;
            Some((draft.clone(), parked.remove(index)))
        });
        self.parked.retain(|_, parked| !parked.is_empty());
        let (draft_key, attachment, saved) = match (parked, saved) {
            (Some((draft_key, attachment)), Ok(saved)) => (draft_key, attachment, saved),
            (None, Ok(saved)) => return self.discard(saved, cx),
            (_, Err(_)) => return,
        };
        let mut draft = self
            .workspace
            .read(cx)
            .drafts
            .get(&draft_key)
            .cloned()
            .unwrap_or_default();
        draft.attachments.push(crate::drafts::Attachment {
            key,
            chipped: attachment.chipped,
            attachment: saved,
        });
        self.workspace.update(cx, |workspace, cx| {
            workspace.set_draft(&draft_key, draft, cx)
        });
    }

    pub(super) fn discard_all(&self, attachments: &[DraftAttachment], cx: &mut Context<Self>) {
        for attachment in attachments {
            if let Some(saved) = attachment.saved.clone() {
                self.discard(saved, cx);
            }
        }
    }

    pub(super) fn discard(&self, saved: MessageAttachment, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move { engine.discard_attachment(&saved) })
            .detach();
    }

    pub(super) fn remove_attachment(
        &mut self,
        key: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chip = attachment_chip_id(key);
        self.editor
            .update(cx, |editor, cx| editor.remove_tokens(&chip, window, cx));
        let Some(index) = self
            .attachments
            .iter()
            .position(|attachment| attachment.key == key)
        else {
            return;
        };
        let removed = self.attachments.remove(index);
        if let Some(saved) = removed.saved {
            self.discard(saved, cx);
        }
        if !self.attachments.iter().any(|attachment| attachment.failed) {
            self.error = None;
        }
        cx.notify();
    }

    /// Whether an attachment is still being saved: the message waits for it.
    pub(super) fn attachments_saving(&self, cx: &App) -> bool {
        self.attached(cx)
            .into_iter()
            .any(|attachment| attachment.saved.is_none() && !attachment.failed)
    }

    pub(super) fn attach_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("attach")
            .icon(IconName::Paperclip)
            .size(ButtonSize::IconSm)
            .variant(ButtonVariant::Ghost)
            .rounded_full()
            .tooltip(Tooltip::new(m::composer_attach_files()))
            .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx)))
            .into_any_element()
    }

    pub(super) fn render_attachments(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let colors = cx.hoshi().clone();
        let shown: Vec<&DraftAttachment> = self
            .attached(cx)
            .into_iter()
            .filter(|attachment| {
                attachment.kind == AttachmentKind::Image || !attachment.chipped || attachment.failed
            })
            .collect();
        if shown.is_empty() {
            return None;
        }
        let tiles = shown.into_iter().map(|attachment| {
            let key = attachment.key;
            let group = SharedString::from(format!("attachment-{key}"));
            let pending = attachment.saved.is_none() && !attachment.failed;
            let tile = div()
                .h(px(56.))
                .flex_none()
                .overflow_hidden()
                .rounded_lg()
                .border_1()
                .border_color(if attachment.failed {
                    colors.destructive
                } else {
                    colors.border
                })
                .bg(colors.muted)
                .map(|tile| match attachment.kind {
                    AttachmentKind::Image => tile.w(px(56.)).child(
                        img(attachment.source.preview())
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .when(pending, |image| image.opacity(0.5)),
                    ),
                    AttachmentKind::File => tile.max_w(px(224.)).child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .when(pending, |row| row.opacity(0.5))
                            .child(Icon::new(IconName::FileText).size_5().flex_none())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(attachment.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(rems(0.7))
                                            .text_color(colors.muted_foreground)
                                            .child(format_size(attachment.size)),
                                    ),
                            ),
                    ),
                });
            let remove = div()
                .id(("remove-attachment", key))
                .absolute()
                .top(px(-6.))
                .right(px(-6.))
                .size_5()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .bg(colors.foreground)
                .text_color(colors.background)
                .opacity(0.)
                .group_hover(group.clone(), |style| style.opacity(1.))
                .child(Icon::new(IconName::X).size_3())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.remove_attachment(key, window, cx)),
                );
            div()
                .id(("attachment", key))
                .group(group)
                .relative()
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open_attachment(key, window, cx)),
                )
                .child(tile)
                .child(remove)
                .with_tooltip(if attachment.failed {
                    m::composer_attachment_failed(&attachment.name)
                } else {
                    attachment.name.clone()
                })
        });
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .px_2()
                .pt_1()
                .pb_1p5()
                .children(tiles)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod pasted_text_tests {
    use super::pasted_text_file_name;

    #[test]
    fn pasted_texts_get_names_of_their_own() {
        assert_eq!(pasted_text_file_name([]), "pasted-text.txt");
        assert_eq!(
            pasted_text_file_name(["Pasted-Text.txt", "pasted-text-2.txt"]),
            "pasted-text-3.txt"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Source, file_type, format_size, is_image, prepare_image, within_limit};
    use hoshi_engine::MAX_FILE_BYTES;

    fn text(name: &str, bytes: usize) -> Source {
        Source::Text {
            name: name.into(),
            text: "x".repeat(bytes).into(),
        }
    }

    #[test]
    fn a_file_over_the_limit_stays_out_before_it_is_read() {
        let (fit, too_large) = within_limit(vec![
            text("small.txt", 10),
            text("huge.txt", MAX_FILE_BYTES + 1),
            text("edge.txt", MAX_FILE_BYTES),
            text("other.txt", MAX_FILE_BYTES + 2),
        ]);
        let names: Vec<String> = fit.iter().map(Source::name).collect();
        assert_eq!(names, ["small.txt", "edge.txt"]);
        assert_eq!(
            too_large.map(|source| source.name()).as_deref(),
            Some("huge.txt")
        );
    }

    #[test]
    fn drawable_images_go_as_images_and_the_rest_as_files() {
        assert!(is_image("shot.PNG"));
        assert!(is_image("screen.bmp"));
        assert!(!is_image("logo.svg"));
        assert!(!is_image("scan.tiff"));
        assert_eq!(file_type("logo.svg"), "image/svg+xml");
        assert_eq!(file_type("notes"), "application/octet-stream");
    }

    fn encoded(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image =
            image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
                image::Rgb([x as u8, y as u8, (x ^ y) as u8])
            }));
        let mut bytes = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn a_small_png_goes_as_it_is_and_a_bmp_is_redrawn_as_png() {
        let png = encoded(image::ImageFormat::Png, 8, 8);
        let (name, mime_type, bytes) = prepare_image("shot.png", png.clone()).unwrap();
        assert_eq!((name.as_str(), mime_type), ("shot.png", "image/png"));
        assert_eq!(bytes, png);

        let bmp = encoded(image::ImageFormat::Bmp, 8, 8);
        let (name, mime_type, bytes) = prepare_image("image.bmp", bmp).unwrap();
        assert_eq!((name.as_str(), mime_type), ("image.png", "image/png"));
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Png
        );

        assert_eq!(prepare_image("broken.bmp", vec![1, 2, 3]), None);
    }

    #[test]
    fn sizes_read_like_the_web() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(8_200), "8 kB");
        assert_eq!(format_size(1_400_000), "1.4 MB");
        assert_eq!(format_size(2_000_000), "2 MB");
    }
}
