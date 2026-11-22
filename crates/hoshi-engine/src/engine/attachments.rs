use crate::*;

impl Engine {
    /// Saves an image the user is about to send. `base64` holds its bytes.
    pub fn save_image(
        &self,
        name: &str,
        mime_type: &str,
        base64: &str,
    ) -> Result<MessageAttachment, CommandError> {
        lock(&self.state)
            .attachments
            .save_image(name, mime_type, base64)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// Saves a file the user is about to send, and the text of a document beside it.
    pub fn save_file(
        &self,
        name: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<MessageAttachment, CommandError> {
        let attachments = lock(&self.state).attachments.clone();
        attachments
            .save_file(name, mime_type, bytes)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// An image or file the user removed before sending it.
    pub fn discard_attachment(&self, attachment: &MessageAttachment) {
        lock(&self.state).attachments.remove(attachment);
    }

    /// Where a saved image or file is, to open it; `None` when it is gone.
    pub fn attachment_path(&self, attachment: &MessageAttachment) -> Option<String> {
        let path = lock(&self.state).attachments.path_of(attachment)?;
        path.is_file().then(|| path.to_string_lossy().into_owned())
    }

    /// An image on this machine an agent's answer shows by its absolute path, as a data URL;
    /// `None` when it isn't there or isn't an image.
    pub fn local_image(&self, path: &str) -> Option<String> {
        html_pages::image_data_url(Path::new(path))
    }

    /// Opens a file an agent's answer links to by its absolute path: in `editor`, or else
    /// in the app the system has for it.
    pub fn open_local_file(&self, path: &str, editor: Option<Editor>) -> Result<(), CommandError> {
        let file = Path::new(path);
        if !file.is_absolute() || !file.exists() {
            return Err(CommandError::Internal {
                detail: format!("{path} isn't there"),
            });
        }
        editors::open(editor.unwrap_or(Editor::FileManager), path)
            .map_err(|detail| CommandError::Internal { detail })
    }

    /// A page an agent showed, by the id `html_render` gave it; `None` when it is gone.
    pub fn html_page(&self, id: &str) -> Option<Vec<u8>> {
        let pages = lock(&self.state).pages.clone();
        pages.read(id)
    }

    /// The file of a page an agent showed; `None` when it is gone.
    pub fn html_page_path(&self, id: &str) -> Option<PathBuf> {
        let pages = lock(&self.state).pages.clone();
        pages.path(id).filter(|path| path.is_file())
    }

    /// A sent image as a data URL; `None` when its file is gone.
    pub fn image_data_url(&self, id: &str, mime_type: &str) -> Option<String> {
        let path = lock(&self.state).attachments.path(id, mime_type)?;
        attachments::data_url(&path, mime_type)
    }
}
