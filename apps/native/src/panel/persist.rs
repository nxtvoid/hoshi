//! What the panel remembers across restarts, in `panel.json` beside the settings, with the
//! shape of the web's `hoshi-panel` store.

use super::surfaces::{DiffShown, Surface, SurfaceState, ThreadSurfaces};
use super::{DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH};
use hoshi_core::DiffScope;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    /// Threads whose panel is open.
    pub open: HashSet<String>,
    pub maximized: bool,
    pub width: f32,
    pub threads: HashMap<String, ThreadSurfaces>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            open: HashSet::new(),
            maximized: false,
            width: DEFAULT_WIDTH,
            threads: HashMap::new(),
        }
    }
}

impl Saved {
    /// Drops the threads `exists` no longer knows; whether anything went.
    pub fn forget(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = (self.open.len(), self.threads.len());
        self.open.retain(|id| exists(id));
        self.threads.retain(|id, _| exists(id));
        before != (self.open.len(), self.threads.len())
    }
}

pub fn file(home: &Path) -> PathBuf {
    home.join("panel.json")
}

fn surface_value(surface: &Surface) -> Value {
    match &surface.state {
        SurfaceState::Diff {
            shown,
            turn_id,
            focus_path,
        } => {
            let scope = match shown {
                DiffShown::Scope(scope) => serde_json::to_value(scope).unwrap_or_default(),
                DiffShown::Turn => json!("turn"),
            };
            let mut value = json!({ "id": surface.id, "kind": "diff", "scope": scope });
            if let Some(turn_id) = turn_id {
                value["turnId"] = json!(turn_id);
            }
            if let Some(focus_path) = focus_path {
                value["focusPath"] = json!(focus_path);
            }
            value
        }
        SurfaceState::PullRequest => json!({ "id": surface.id, "kind": "pullRequest" }),
        SurfaceState::Terminal => json!({ "id": surface.id, "kind": "terminal" }),
        SurfaceState::Browser => json!({ "id": surface.id, "kind": "browser" }),
        SurfaceState::File { path } => json!({ "id": surface.id, "kind": "file", "path": path }),
    }
}

/// A saved surface, or `None` for one another version wrote that this one doesn't know.
fn parse_surface(value: &Value) -> Option<Surface> {
    let id = value.get("id")?.as_str()?.to_owned();
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let state = match value.get("kind")?.as_str()? {
        "diff" => SurfaceState::Diff {
            shown: match value.get("scope") {
                Some(Value::String(scope)) if scope == "turn" => DiffShown::Turn,
                Some(scope) => DiffShown::Scope(
                    serde_json::from_value(scope.clone()).unwrap_or(DiffScope::Uncommitted),
                ),
                None => DiffShown::Scope(DiffScope::Uncommitted),
            },
            turn_id: text("turnId"),
            focus_path: text("focusPath"),
        },
        "pullRequest" => SurfaceState::PullRequest,
        "terminal" => SurfaceState::Terminal,
        "browser" => SurfaceState::Browser,
        "file" => SurfaceState::File {
            path: text("path").filter(|path| !path.is_empty())?,
        },
        _ => return None,
    };
    Some(Surface { id, state })
}

fn parse_thread(value: &Value) -> ThreadSurfaces {
    let surfaces: Vec<Surface> = value
        .get("surfaces")
        .and_then(Value::as_array)
        .map(|surfaces| surfaces.iter().filter_map(parse_surface).collect())
        .unwrap_or_default();
    let saved = value.get("activeId").and_then(Value::as_str);
    let active_id = surfaces
        .iter()
        .find(|surface| Some(surface.id.as_str()) == saved)
        .or(surfaces.first())
        .map(|surface| surface.id.clone());
    ThreadSurfaces {
        surfaces,
        active_id,
    }
}

pub fn parse(text: &str) -> Saved {
    let value: Value = serde_json::from_str(text).unwrap_or_default();
    let open = value
        .get("openThreads")
        .and_then(Value::as_object)
        .map(|open| {
            open.iter()
                .filter(|(_, open)| open.as_bool() == Some(true))
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default();
    let threads = value
        .get("threads")
        .and_then(Value::as_object)
        .map(|threads| {
            threads
                .iter()
                .map(|(id, panel)| (id.clone(), parse_thread(panel)))
                .collect()
        })
        .unwrap_or_default();
    Saved {
        open,
        maximized: value
            .get("maximized")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        width: value
            .get("width")
            .and_then(Value::as_f64)
            .map_or(DEFAULT_WIDTH, |width| {
                (width as f32).clamp(MIN_WIDTH, MAX_WIDTH).round()
            }),
        threads,
    }
}

pub fn serialize(saved: &Saved) -> String {
    let mut open: Vec<&String> = saved.open.iter().collect();
    open.sort();
    let open: Map<String, Value> = open
        .into_iter()
        .map(|id| (id.clone(), json!(true)))
        .collect();
    let mut ids: Vec<&String> = saved.threads.keys().collect();
    ids.sort();
    let threads: Map<String, Value> = ids
        .into_iter()
        .map(|id| {
            let panel = &saved.threads[id];
            let surfaces: Vec<Value> = panel.surfaces.iter().map(surface_value).collect();
            (
                id.clone(),
                json!({ "surfaces": surfaces, "activeId": panel.active_id }),
            )
        })
        .collect();
    json!({
        "openThreads": open,
        "maximized": saved.maximized,
        "width": saved.width,
        "threads": threads,
    })
    .to_string()
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
    use crate::panel::surfaces::{DiffShown, Surface, SurfaceState, ThreadSurfaces};
    use hoshi_core::DiffScope;

    fn saved() -> Saved {
        let mut saved = Saved {
            maximized: true,
            width: 512.,
            ..Saved::default()
        };
        saved.open.insert("t1".into());
        saved.threads.insert(
            "t1".into(),
            ThreadSurfaces {
                surfaces: vec![
                    Surface {
                        id: "a".into(),
                        state: SurfaceState::Diff {
                            shown: DiffShown::Turn,
                            turn_id: Some("m1".into()),
                            focus_path: Some("src/a.rs".into()),
                        },
                    },
                    Surface {
                        id: "b".into(),
                        state: SurfaceState::PullRequest,
                    },
                    Surface {
                        id: "c".into(),
                        state: SurfaceState::Terminal,
                    },
                ],
                active_id: Some("b".into()),
            },
        );
        saved.threads.insert(
            "t2".into(),
            ThreadSurfaces {
                surfaces: vec![
                    Surface {
                        id: "d".into(),
                        state: SurfaceState::Diff {
                            shown: DiffShown::Scope(DiffScope::Branch),
                            turn_id: None,
                            focus_path: None,
                        },
                    },
                    Surface {
                        id: "e".into(),
                        state: SurfaceState::Browser,
                    },
                    Surface {
                        id: "f".into(),
                        state: SurfaceState::File {
                            path: "/home/dev/shop/sales.csv".into(),
                        },
                    },
                ],
                active_id: Some("e".into()),
            },
        );
        saved
    }

    #[test]
    fn the_panel_survives_a_round_trip_in_the_webs_shape() {
        let text = serialize(&saved());
        assert_eq!(parse(&text), saved());
        assert!(text.contains(r#""openThreads":{"t1":true}"#), "{text}");
        assert!(
            text.contains(r#"{"id":"d","kind":"diff","scope":"branch"}"#),
            "{text}"
        );
        assert!(text.contains(r#""turnId":"m1""#), "{text}");
        assert!(
            text.contains(r#"{"id":"f","kind":"file","path":"/home/dev/shop/sales.csv"}"#),
            "{text}"
        );
    }

    #[test]
    fn unknown_surfaces_drop_out_and_bad_files_read_as_defaults() {
        let text = r#"{"width":9000,"threads":{"t":{"surfaces":[{"id":"x","kind":"notes"},{"id":"y","kind":"browser"}],"activeId":"x"}},"openThreads":{"t":true,"u":false}}"#;
        let saved = parse(text);
        assert_eq!(saved.width, super::MAX_WIDTH);
        assert_eq!(saved.open.len(), 1);
        let panel = &saved.threads["t"];
        assert_eq!(panel.surfaces.len(), 1);
        assert_eq!(panel.active_id.as_deref(), Some("y"));
        assert_eq!(parse("not json"), Saved::default());
    }

    #[test]
    fn forgetting_drops_deleted_threads_only() {
        let mut saved = saved();
        assert!(!saved.forget(|_| true));
        assert!(saved.forget(|id| id == "t2"));
        assert!(saved.open.is_empty());
        assert_eq!(saved.threads.keys().collect::<Vec<_>>(), ["t2"]);
    }
}
