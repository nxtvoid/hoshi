use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a thread's browser page painted, a JPEG, while the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserFrame {
    pub thread_id: String,
    pub jpeg: Vec<u8>,
    /// The page's size in CSS pixels, which clicks on the frame map back to.
    pub width: u32,
    pub height: u32,
}

impl BrowserFrame {
    /// The frame as raw bytes for the UI, numbers little-endian: the thread id's length as
    /// a `u16` and its UTF-8, the width and the height as `u32`, then the JPEG.
    pub fn to_bytes(&self) -> Vec<u8> {
        let id = self.thread_id.as_bytes();
        let mut bytes = Vec::with_capacity(2 + id.len() + 8 + self.jpeg.len());
        bytes.extend_from_slice(&(id.len() as u16).to_le_bytes());
        bytes.extend_from_slice(id);
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.jpeg);
        bytes
    }
}

/// A point of the page, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserPoint {
    pub x: f64,
    pub y: f64,
}

/// Where a thread's browser page is, and what last touched it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserPage {
    pub thread_id: String,
    /// `false` once the page closed.
    pub open: bool,
    pub url: String,
    pub title: String,
    /// Console errors so far.
    pub errors: u32,
    /// The agent acted on the page just now.
    pub by_agent: bool,
    /// Where the agent acted, when it acted on an element.
    pub agent_point: Option<BrowserPoint>,
    /// The agent opened the thread's page just now, for the first time.
    pub opened: bool,
    /// Whether the page takes the panel's size; `None` when this update doesn't say.
    pub fitted: Option<bool>,
}

/// What the user does on the page in the panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum BrowserInput {
    Click {
        x: f64,
        y: f64,
    },
    Scroll {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    },
    /// A key such as `Enter`, `Backspace` or `ArrowDown`.
    Key {
        key: String,
    },
    /// Typed or pasted text.
    Text {
        text: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_its_thread_its_size_and_its_jpeg() {
        let frame = BrowserFrame {
            thread_id: "té".into(),
            jpeg: vec![0xff, 0xd8, 0xff],
            width: 1280,
            height: 800,
        };
        assert_eq!(
            frame.to_bytes(),
            [
                &[3, 0][..],
                "té".as_bytes(),
                &1280u32.to_le_bytes(),
                &800u32.to_le_bytes(),
                &[0xff, 0xd8, 0xff],
            ]
            .concat()
        );
    }
}
