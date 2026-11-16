//! What the sidebar remembers across restarts, in `sidebar.json` beside the settings, with the
//! shape of the web's `hoshi-sidebar` storage.

use crate::sections::{Filter, Section, SectionId, Sort, View};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const MIN_WIDTH: f32 = 220.;
pub const MAX_WIDTH: f32 = 420.;
pub const DEFAULT_WIDTH: f32 = 272.;

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub sort: Sort,
    pub view: View,
    pub filter: Filter,
    pub width: f32,
    /// Sections the user folded; inactive and done start folded.
    pub collapsed: HashSet<SectionId>,
    /// Project ids in the order the user dragged them into on the rail.
    pub project_order: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            sort: Sort::Activity,
            view: View::Project,
            filter: Filter::All,
            width: DEFAULT_WIDTH,
            collapsed: HashSet::from([SectionId::Inactive, SectionId::Done]),
            project_order: Vec::new(),
        }
    }
}

const SECTIONS: [(SectionId, &str); 9] = [
    (SectionId::Pinned, "pinned"),
    (SectionId::Attention, "attention"),
    (SectionId::Active, "active"),
    (SectionId::Review, "review"),
    (SectionId::Reviewing, "reviewing"),
    (SectionId::Inactive, "inactive"),
    (SectionId::Snoozed, "snoozed"),
    (SectionId::Done, "done"),
    (SectionId::Completed, "completed"),
];

pub fn clamp_width(width: f32) -> f32 {
    width.clamp(MIN_WIDTH, MAX_WIDTH).round()
}

pub fn file(home: &Path) -> PathBuf {
    home.join("sidebar.json")
}

pub fn parse(text: &str) -> Prefs {
    let value: Value = serde_json::from_str(text).unwrap_or_default();
    let defaults = Prefs::default();
    let text_of = |key: &str| value.get(key).and_then(Value::as_str);
    let collapsed = match value.get("collapsed").and_then(Value::as_object) {
        Some(folded) => SECTIONS
            .iter()
            .filter(|(_, name)| folded.get(*name).and_then(Value::as_bool) == Some(true))
            .map(|(id, _)| *id)
            .collect(),
        None => defaults.collapsed.clone(),
    };
    Prefs {
        sort: match text_of("threadSort") {
            Some("created") => Sort::Created,
            _ => Sort::Activity,
        },
        view: match text_of("view") {
            Some("chats") => View::Chats,
            Some("inbox") => View::Inbox,
            _ => View::Project,
        },
        filter: match text_of("filter") {
            Some("worktrees") => Filter::Worktrees,
            Some("pullRequests") => Filter::PullRequests,
            _ => Filter::All,
        },
        width: value
            .get("width")
            .and_then(Value::as_f64)
            .map(|width| clamp_width(width as f32))
            .unwrap_or(defaults.width),
        collapsed,
        project_order: value
            .get("projectOrder")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub fn serialize(prefs: &Prefs) -> String {
    let collapsed: Map<String, Value> = SECTIONS
        .iter()
        .map(|(id, name)| ((*name).to_owned(), json!(prefs.collapsed.contains(id))))
        .collect();
    json!({
        "threadSort": match prefs.sort {
            Sort::Activity => "activity",
            Sort::Created => "created",
        },
        "view": match prefs.view {
            View::Project => "project",
            View::Chats => "chats",
            View::Inbox => "inbox",
        },
        "filter": match prefs.filter {
            Filter::All => "all",
            Filter::Worktrees => "worktrees",
            Filter::PullRequests => "pullRequests",
        },
        "width": prefs.width,
        "collapsed": collapsed,
        "projectOrder": prefs.project_order,
    })
    .to_string()
}

pub fn load(home: &Path) -> Prefs {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(home: &Path, prefs: &Prefs) -> std::io::Result<()> {
    std::fs::write(file(home), serialize(prefs))
}

/// `items` in the order the user dragged them into; ones added since go last, as they came.
pub fn order_by<T>(items: Vec<T>, order: &[String], id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut ranked: Vec<(usize, T)> = items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let rank = order
                .iter()
                .position(|kept| kept == id(&item))
                .unwrap_or(order.len() + index);
            (rank, item)
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// `ids` with `moved` placed where `over` was.
pub fn move_id(ids: &[String], moved: &str, over: &str) -> Vec<String> {
    let mut next = ids.to_vec();
    let (Some(from), Some(to)) = (
        ids.iter().position(|id| id == moved),
        ids.iter().position(|id| id == over),
    ) else {
        return next;
    };
    if from != to {
        let id = next.remove(from);
        next.insert(to, id);
    }
    next
}

/// The threads the list shows, top to bottom: folded sections keep only the open thread.
pub fn visible_order(
    sections: &[Section],
    is_open: impl Fn(SectionId) -> bool,
    selected: Option<&str>,
) -> Vec<String> {
    sections
        .iter()
        .flat_map(|section| {
            let open = is_open(section.id);
            section
                .threads
                .iter()
                .filter(move |thread| open || selected == Some(thread.id.as_str()))
                .map(|thread| thread.id.clone())
        })
        .collect()
}

/// The thread `step` places away from the current one, wrapping around; the first or last when
/// none is current.
pub fn step_thread(order: &[String], current: Option<&str>, step: isize) -> Option<String> {
    if order.is_empty() {
        return None;
    }
    let len = order.len() as isize;
    let index = match current.and_then(|current| order.iter().position(|id| id == current)) {
        Some(index) => (index as isize + step).rem_euclid(len),
        None if step > 0 => 0,
        None => len - 1,
    };
    order.get(index as usize).cloned()
}

#[cfg(test)]
pub(super) fn sample_thread(id: &str) -> hoshi_core::Thread {
    hoshi_core::Thread {
        id: id.into(),
        project_id: Some("project".into()),
        title: id.into(),
        status: hoshi_core::ThreadStatus::Idle,
        branch: None,
        worktree_path: None,
        session_id: None,
        conversation_cursor: None,
        resume_at: None,
        watching_pull_request: false,
        provider: hoshi_core::default_provider(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: hoshi_core::AccessMode::default(),
        plan_mode: false,
        active_turn_id: None,
        archived: false,
        pinned: false,
        worktree_released: false,
        worktree_adopted: false,
        parent_thread_id: None,
        task_cancelled: false,
        task_answered: false,
        snoozed_until: None,
        snoozed_at: None,
        last_snooze_wake_at: None,
        last_visited_at: None,
        last_turn_ended_at: None,
        forked_from: None,
        pending_model: None,
        provider_sessions: Vec::new(),
        pending_handoffs: Vec::new(),
        created_at: "2026-11-13T10:00:00Z".into(),
        updated_at: "2026-11-13T10:00:00Z".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_in_the_web_shape() {
        let prefs = Prefs {
            sort: Sort::Created,
            view: View::Inbox,
            filter: Filter::PullRequests,
            width: 300.,
            collapsed: HashSet::from([SectionId::Done]),
            project_order: vec!["b".into(), "a".into()],
        };
        let text = serialize(&prefs);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["threadSort"], "created");
        assert_eq!(value["filter"], "pullRequests");
        assert_eq!(value["collapsed"]["done"], true);
        assert_eq!(value["collapsed"]["inactive"], false);
        assert_eq!(parse(&text), prefs);
    }

    #[test]
    fn missing_or_broken_prefs_fall_back_to_the_defaults() {
        assert_eq!(parse(""), Prefs::default());
        let narrow = parse(r#"{"width": 90, "view": "nowhere"}"#);
        assert_eq!(narrow.width, MIN_WIDTH);
        assert_eq!(narrow.view, View::Project);
    }

    #[test]
    fn projects_follow_the_dragged_order_and_new_ones_go_last() {
        let order = vec!["c".to_owned(), "a".to_owned()];
        let items = vec!["a", "b", "c", "d"];
        assert_eq!(order_by(items, &order, |id| id), vec!["c", "a", "b", "d"]);
    }

    #[test]
    fn a_dragged_project_takes_the_place_of_the_one_it_lands_on() {
        let ids: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(move_id(&ids, "a", "c"), ["b", "c", "a"]);
        assert_eq!(move_id(&ids, "c", "a"), ["c", "a", "b"]);
        assert_eq!(move_id(&ids, "x", "a"), ids);
    }

    #[test]
    fn stepping_wraps_and_starts_at_an_end() {
        let order: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(step_thread(&order, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(step_thread(&order, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(step_thread(&order, None, 1).as_deref(), Some("a"));
        assert_eq!(step_thread(&order, None, -1).as_deref(), Some("c"));
        assert_eq!(step_thread(&[], None, 1), None);
    }

    #[test]
    fn a_folded_section_keeps_only_the_open_thread_in_the_order() {
        let thread = sample_thread;
        let sections = vec![
            Section {
                id: SectionId::Active,
                threads: vec![thread("a"), thread("b")],
            },
            Section {
                id: SectionId::Done,
                threads: vec![thread("c"), thread("d")],
            },
        ];
        let order = visible_order(&sections, |id| id != SectionId::Done, Some("d"));
        assert_eq!(order, ["a", "b", "d"]);
    }
}
