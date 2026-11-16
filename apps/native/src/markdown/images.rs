//! The images messages were sent with: read once each from where hoshi saved them, off the UI
//! thread, and shown large over the window when opened.

use super::{Links, sites};
use gpui_kit::*;
use hoshi_core::MessageAttachment;
use hoshi_ui::prelude::{DialogLayer, DialogSize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

enum Loaded {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

fn loaded() -> &'static Mutex<HashMap<String, Loaded>> {
    static LOADED: OnceLock<Mutex<HashMap<String, Loaded>>> = OnceLock::new();
    LOADED.get_or_init(Default::default)
}

fn format(mime_type: &str) -> Option<ImageFormat> {
    match mime_type {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::Webp),
        _ => None,
    }
}

/// The image, once read; asks for it the first time.
pub fn image(id: &str, mime_type: &str, cx: &App) -> Option<Arc<Image>> {
    let mut cache = loaded().lock().ok()?;
    match cache.get(id) {
        Some(Loaded::Ready(image)) => return Some(image.clone()),
        Some(Loaded::Loading | Loaded::Failed) => return None,
        None => {}
    }
    let engine = cx.try_global::<Links>()?.engine.clone();
    cache.insert(id.to_owned(), Loaded::Loading);
    drop(cache);
    let attachment = MessageAttachment::Image {
        id: id.to_owned(),
        name: String::new(),
        mime_type: mime_type.to_owned(),
        size: 0,
    };
    let id = id.to_owned();
    let image_format = format(mime_type);
    std::thread::spawn(move || {
        let image = engine
            .attachment_path(&attachment)
            .and_then(|path| std::fs::read(path).ok())
            .zip(image_format)
            .map(|(bytes, format)| Arc::new(Image::from_bytes(format, bytes)));
        if let Ok(mut loaded) = loaded().lock() {
            loaded.insert(
                id,
                match image {
                    Some(image) => Loaded::Ready(image),
                    None => Loaded::Failed,
                },
            );
        }
        sites::redraw();
    });
    None
}

/// An image not sent yet at full size, over the window.
pub fn show_source(source: ImageSource, name: String, window: &mut Window, cx: &mut App) {
    DialogLayer::open(window, cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        dialog
            .size(DialogSize::Custom(viewport.width * 0.9))
            .title(name.clone())
            .child(
                div().flex().justify_center().child(
                    img(source.clone())
                        .max_w(viewport.width * 0.85)
                        .max_h(viewport.height * 0.75)
                        .object_fit(ObjectFit::Contain)
                        .rounded_lg(),
                ),
            )
    });
}

/// The image at full size, over the window.
pub fn show(id: String, mime_type: String, name: String, window: &mut Window, cx: &mut App) {
    DialogLayer::open(window, cx, move |dialog, window, cx| {
        let viewport = window.viewport_size();
        let picture = image(&id, &mime_type, cx).map(|picture| {
            img(ImageSource::Image(picture))
                .max_w(viewport.width * 0.85)
                .max_h(viewport.height * 0.75)
                .object_fit(ObjectFit::Contain)
                .rounded_lg()
        });
        dialog
            .size(DialogSize::Custom(viewport.width * 0.9))
            .title(name.clone())
            .child(div().flex().justify_center().children(picture))
    });
}
