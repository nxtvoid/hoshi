//! The drawers kept across restarts in `terminals.json` beside the settings, shaped like the
//! web's `hoshi-terminals`: the drawer height and each thread's tabs.

use super::drawer::{self, Drawer};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub height: f32,
    pub drawers: HashMap<String, Drawer>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            height: drawer::DEFAULT_HEIGHT,
            drawers: HashMap::new(),
        }
    }
}

pub fn file(home: &Path) -> PathBuf {
    home.join("terminals.json")
}

fn parse_drawer(value: &Value) -> Option<Drawer> {
    let terminal_ids: Vec<String> = value
        .get("terminalIds")?
        .as_array()?
        .iter()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect();
    if terminal_ids.is_empty() {
        return None;
    }
    let active_id = value
        .get("activeId")
        .and_then(Value::as_str)
        .filter(|id| terminal_ids.iter().any(|other| other == id))
        .map(str::to_owned)
        .or_else(|| terminal_ids.first().cloned());
    Some(Drawer {
        open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
        terminal_ids,
        active_id,
    })
}

pub fn parse(text: &str) -> Saved {
    let value: Value = serde_json::from_str(text).unwrap_or_default();
    let height = value
        .get("height")
        .and_then(Value::as_f64)
        .map_or(drawer::DEFAULT_HEIGHT, |height| {
            drawer::clamp_height(height as f32)
        });
    let drawers = value
        .get("drawers")
        .and_then(Value::as_object)
        .map(|drawers| {
            drawers
                .iter()
                .filter_map(|(thread_id, drawer)| Some((thread_id.clone(), parse_drawer(drawer)?)))
                .collect()
        })
        .unwrap_or_default();
    Saved { height, drawers }
}

pub fn serialize(saved: &Saved) -> String {
    let drawers: Map<String, Value> = saved
        .drawers
        .iter()
        .filter(|(_, drawer)| !drawer.terminal_ids.is_empty())
        .map(|(thread_id, drawer)| {
            (
                thread_id.clone(),
                json!({
                    "open": drawer.open,
                    "terminalIds": drawer.terminal_ids,
                    "activeId": drawer.active_id,
                }),
            )
        })
        .collect();
    json!({ "height": saved.height, "drawers": drawers }).to_string()
}

pub fn load(home: &Path) -> Saved {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(home: &Path, saved: &Saved) -> std::io::Result<()> {
    std::fs::write(file(home), serialize(saved))
}

#[cfg(test)]
mod tests {
    use super::{Saved, parse, serialize};
    use crate::terminal::drawer::{DEFAULT_HEIGHT, Drawer};
    use std::collections::HashMap;

    #[test]
    fn drawers_round_trip() {
        let saved = Saved {
            height: 340.,
            drawers: HashMap::from([
                (
                    "t1".to_owned(),
                    Drawer {
                        open: true,
                        terminal_ids: vec!["a".into(), "b".into()],
                        active_id: Some("b".into()),
                    },
                ),
                (
                    "t2".to_owned(),
                    Drawer {
                        open: false,
                        terminal_ids: vec!["c".into()],
                        active_id: Some("c".into()),
                    },
                ),
            ]),
        };
        assert_eq!(parse(&serialize(&saved)), saved);
    }

    #[test]
    fn broken_or_odd_files_fall_back_to_what_makes_sense() {
        assert_eq!(parse("not json"), Saved::default());
        let saved = parse(
            r#"{"height": 5000, "drawers": {
                "empty": {"open": true, "terminalIds": [], "activeId": null},
                "lost": {"open": true, "terminalIds": ["a"], "activeId": "gone"},
                "bad": 3
            }}"#,
        );
        assert_eq!(saved.height, 800.);
        assert_eq!(saved.drawers.len(), 1);
        assert_eq!(saved.drawers["lost"].active_id.as_deref(), Some("a"));
        assert_eq!(parse("{}").height, DEFAULT_HEIGHT);
    }

    #[test]
    fn empty_drawers_are_not_written() {
        let saved = Saved {
            height: 280.,
            drawers: HashMap::from([("t".to_owned(), Drawer::default())]),
        };
        let written: serde_json::Value = serde_json::from_str(&serialize(&saved)).unwrap();
        assert_eq!(
            written,
            serde_json::json!({ "height": 280.0, "drawers": {} })
        );
    }
}
