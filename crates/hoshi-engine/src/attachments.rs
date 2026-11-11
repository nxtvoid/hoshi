//! Images sent with messages, saved one file each under the app's data folder.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hoshi_core::MessageAttachment;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Above what providers accept for one image.
const MAX_BYTES: usize = 10 * 1024 * 1024;

pub(crate) struct Attachments {
    dir: PathBuf,
    counter: AtomicU64,
}

/// The image types every provider reads.
fn extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// The image at `path`, for the transcript to show it; `None` when its file is gone.
pub(crate) fn data_url(path: &Path, mime_type: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime_type};base64,{}",
        STANDARD.encode(bytes)
    ))
}

impl Attachments {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            counter: AtomicU64::new(0),
        }
    }

    pub(crate) fn save_image(
        &self,
        name: &str,
        mime_type: &str,
        base64: &str,
    ) -> Result<MessageAttachment, String> {
        let extension = extension(mime_type).ok_or("unsupported image type")?;
        let bytes = STANDARD.decode(base64).map_err(|error| error.to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(format!("image of {} bytes", bytes.len()));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let id = format!(
            "{nanos:x}{:x}",
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        std::fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        std::fs::write(self.dir.join(format!("{id}.{extension}")), &bytes)
            .map_err(|error| error.to_string())?;
        Ok(MessageAttachment::Image {
            id,
            name: name.to_owned(),
            mime_type: mime_type.to_owned(),
            size: bytes.len() as u64,
        })
    }

    /// `None` for ids that could reach outside the folder.
    pub(crate) fn path(&self, id: &str, mime_type: &str) -> Option<PathBuf> {
        let valid = !id.is_empty() && id.chars().all(|char| char.is_ascii_alphanumeric());
        let extension = extension(mime_type)?;
        valid.then(|| self.dir.join(format!("{id}.{extension}")))
    }

    pub(crate) fn remove(&self, attachment: &MessageAttachment) {
        if let MessageAttachment::Image { id, mime_type, .. } = attachment
            && let Some(path) = self.path(id, mime_type)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachments(name: &str) -> Attachments {
        let dir =
            std::env::temp_dir().join(format!("hoshi-attachments-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Attachments::new(dir)
    }

    #[test]
    fn a_saved_image_reads_back_and_can_be_removed() {
        let attachments = attachments("roundtrip");

        let image = attachments
            .save_image("shot.png", "image/png", "cG5n")
            .unwrap();
        let MessageAttachment::Image { id, size, .. } = &image else {
            panic!("not an image");
        };

        let path = attachments.path(id, "image/png").unwrap();
        assert_eq!(*size, 3);
        assert_eq!(
            data_url(&path, "image/png").as_deref(),
            Some("data:image/png;base64,cG5n")
        );
        attachments.remove(&image);
        assert_eq!(data_url(&path, "image/png"), None);
    }

    #[test]
    fn other_types_and_unsafe_ids_are_refused() {
        let attachments = attachments("refused");

        assert!(
            attachments
                .save_image("a.svg", "image/svg+xml", "cG5n")
                .is_err()
        );
        assert_eq!(attachments.path("../secret", "image/png"), None);
        assert_eq!(attachments.path("abc", "text/plain"), None);
    }
}
