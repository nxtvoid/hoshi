//! What the webview can't read from the clipboard itself: text for the edit menu's Paste,
//! an image on the system clipboard, as an email client copies it, and image files a file
//! manager copied, which reach a Linux webview as paths only.

use std::path::Path;
use tauri::ipc::Response;

/// Larger files are no image anyone means to send.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// The bytes of the image file at `path`, as the paste of a copied file gives it.
#[tauri::command(async)]
pub fn read_image_file(path: String) -> Result<Response, String> {
    image_file(Path::new(&path)).map(Response::new)
}

/// The image on the system clipboard as PNG, or the first image file copied there.
#[tauri::command(async)]
pub fn read_clipboard_image() -> Result<Response, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    if let Ok(image) = clipboard.get_image() {
        let (width, height) = (image.width as u32, image.height as u32);
        return encode_png(width, height, &image.bytes).map(Response::new);
    }
    let files = clipboard
        .get()
        .file_list()
        .map_err(|error| error.to_string())?;
    files
        .iter()
        .find_map(|path| image_file(path).ok())
        .map(Response::new)
        .ok_or_else(|| "no image on the clipboard".into())
}

/// The text on the system clipboard, for the edit menu's Paste.
#[tauri::command(async)]
pub fn read_clipboard_text() -> Result<String, String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .map_err(|error| error.to_string())
}

fn image_file(path: &Path) -> Result<Vec<u8>, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err("not an image file".into());
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("not an image file".into());
    }
    std::fs::read(path).map_err(|error| error.to_string())
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(rgba)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_image_files_are_read() {
        let dir = std::env::temp_dir().join(format!("hoshi-clipboard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("shot.PNG");
        let text = dir.join("notes.txt");
        std::fs::write(&image, b"png").unwrap();
        std::fs::write(&text, b"text").unwrap();

        assert_eq!(image_file(&image).unwrap(), b"png");
        assert!(image_file(&text).is_err());
        assert!(image_file(&dir.join("gone.png")).is_err());
        assert!(image_file(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clipboard_pixels_become_a_png() {
        let png = encode_png(1, 1, &[255, 0, 0, 255]).unwrap();

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
