//! Images and files sent with messages, saved under the app's data folder: an image as one
//! file, any other file in a folder of its own under its own name.

use crate::document_text;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hoshi_core::MessageAttachment;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Above what providers accept for one image.
const MAX_BYTES: usize = 10 * 1024 * 1024;
/// The largest file a message carries.
pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
/// Longest name a saved file keeps, in characters.
const MAX_NAME_CHARS: usize = 120;

#[derive(Clone)]
pub(crate) struct Attachments {
    dir: PathBuf,
    counter: Arc<AtomicU64>,
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

/// `name` as a file name every system takes: its last component, without the characters
/// Windows refuses, and never one of its reserved device names.
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|char| {
            if char.is_control() || "<>:\"|?*".contains(char) {
                '_'
            } else {
                char
            }
        })
        .collect();
    let cleaned = shortened(cleaned.trim().trim_end_matches(['.', ' ']));
    if cleaned.is_empty() || cleaned.chars().all(|char| char == '.') {
        return "file".into();
    }
    let stem = cleaned.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

/// `name` within `MAX_NAME_CHARS`, keeping its extension.
fn shortened(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_owned();
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| extension.chars().count() <= 16)
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let keep = MAX_NAME_CHARS - extension.chars().count();
    let stem: String = name.chars().take(keep).collect();
    format!("{}{extension}", stem.trim_end_matches(['.', ' ']))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|char| char.is_ascii_alphanumeric())
}

impl Attachments {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    fn next_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        format!(
            "{nanos:x}{:x}",
            self.counter.fetch_add(1, Ordering::Relaxed)
        )
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
        let id = self.next_id();
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

    /// Saves a file the user is about to send, as it is and under its own name, with the
    /// text of a document beside it for an agent that can't read the document itself.
    pub(crate) fn save_file(
        &self,
        name: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<MessageAttachment, String> {
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(format!("file of {} bytes", bytes.len()));
        }
        let id = self.next_id();
        let folder = self.files_dir().join(&id);
        std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let path = folder.join(safe_name(name));
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        document_text::extract(&path);
        Ok(MessageAttachment::File {
            id,
            name: name.to_owned(),
            mime_type: mime_type.to_owned(),
            size: bytes.len() as u64,
        })
    }

    fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    /// `None` for ids that could reach outside the folder.
    pub(crate) fn path(&self, id: &str, mime_type: &str) -> Option<PathBuf> {
        let extension = extension(mime_type)?;
        valid_id(id).then(|| self.dir.join(format!("{id}.{extension}")))
    }

    /// Where an image or file is saved; `None` for other attachments and unsafe ids.
    pub(crate) fn path_of(&self, attachment: &MessageAttachment) -> Option<PathBuf> {
        match attachment {
            MessageAttachment::Image { id, mime_type, .. } => self.path(id, mime_type),
            MessageAttachment::File { id, name, .. } => {
                valid_id(id).then(|| self.files_dir().join(id).join(safe_name(name)))
            }
            _ => None,
        }
    }

    pub(crate) fn remove(&self, attachment: &MessageAttachment) {
        match attachment {
            MessageAttachment::Image { .. } => {
                if let Some(path) = self.path_of(attachment) {
                    let _ = std::fs::remove_file(path);
                }
            }
            MessageAttachment::File { id, .. } if valid_id(id) => {
                let _ = std::fs::remove_dir_all(self.files_dir().join(id));
            }
            _ => {}
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

    #[test]
    fn a_saved_file_keeps_its_name_and_goes_with_its_folder() {
        let attachments = attachments("file");

        let file = attachments
            .save_file("notes/../Q3 report.pdf", "application/pdf", b"%PDF-")
            .unwrap();
        let path = attachments.path_of(&file).unwrap();

        assert_eq!(path.file_name().unwrap(), "Q3 report.pdf");
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-");
        attachments.remove(&file);
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn empty_and_unsafe_files_are_refused() {
        let attachments = attachments("file-refused");

        assert!(attachments.save_file("a.txt", "text/plain", b"").is_err());
        let traversal = MessageAttachment::File {
            id: "..".into(),
            name: "a.txt".into(),
            mime_type: "text/plain".into(),
            size: 1,
        };
        assert_eq!(attachments.path_of(&traversal), None);
    }

    #[test]
    fn file_names_are_made_safe_for_every_system() {
        assert_eq!(safe_name(r"C:\Users\me\a<b>.txt"), "a_b_.txt");
        assert_eq!(safe_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_name("com1"), "_com1");
        assert_eq!(safe_name("..."), "file");
        assert_eq!(safe_name("trailing. "), "trailing");
        let short = safe_name(&format!("{}.docx", "a".repeat(300)));
        assert_eq!(short.chars().count(), MAX_NAME_CHARS);
        assert!(short.ends_with(".docx"));
    }
}
