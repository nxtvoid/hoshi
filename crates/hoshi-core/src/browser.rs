use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a thread's browser page painted, a JPEG in base64, while the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserFrame {
    pub thread_id: String,
    pub data: String,
    /// The page's size in CSS pixels, which clicks on the frame map back to.
    pub width: u32,
    pub height: u32,
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
