//! What the palette finds for a query among threads, projects and its own actions.

use crate::sections::PullRequests;
use hoshi_core::{Project, Thread};
use std::cmp::Ordering;
use std::collections::HashMap;

const THREAD_LIMIT: usize = 8;
const RECENT_LIMIT: usize = 6;
const PROJECT_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewThread,
    NewThreadFromPullRequest,
    AddProject,
    Inbox,
    Settings,
    Usage,
}

pub const ACTIONS: [Action; 6] = [
    Action::NewThread,
    Action::NewThreadFromPullRequest,
    Action::AddProject,
    Action::Inbox,
    Action::Settings,
    Action::Usage,
];

impl Action {
    /// Works on the selected project, so it needs one.
    pub fn needs_project(self) -> bool {
        matches!(self, Self::NewThread | Self::NewThreadFromPullRequest)
    }
}

pub struct Results<'a> {
    pub threads: Vec<&'a Thread>,
    pub projects: Vec<&'a Project>,
    pub actions: Vec<Action>,
}

/// 3 when the text starts with the word, 2 when one of its words does, 1 when it only contains
/// it.
pub fn score_field(text: &str, word: &str) -> u32 {
    let lower = text.to_lowercase();
    let Some(at) = lower.find(word) else {
        return 0;
    };
    if at == 0 {
        return 3;
    }
    let before = lower[..at].chars().last();
    if before.is_some_and(|char| char.is_whitespace() || "/_.#-".contains(char)) {
        2
    } else {
        1
    }
}

/// Every word has to match one of the fields; the best field of each word adds up.
fn score(fields: &[&str], words: &[String]) -> u32 {
    let mut total = 0;
    for word in words {
        let best = fields
            .iter()
            .map(|field| score_field(field, word))
            .max()
            .unwrap_or(0);
        if best == 0 {
            return 0;
        }
        total += best;
    }
    total
}

fn ranked<'a, 'f, T>(
    items: impl Iterator<Item = &'a T>,
    fields: impl Fn(&'a T) -> Vec<&'f str>,
    words: &[String],
    tie_break: impl Fn(&T, &T) -> Ordering,
) -> Vec<&'a T> {
    let mut scored: Vec<(&T, u32)> = items
        .map(|item| (item, score(&fields(item), words)))
        .filter(|(_, score)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| tie_break(a.0, b.0)));
    scored.into_iter().map(|(item, _)| item).collect()
}

fn newest(a: &Thread, b: &Thread) -> Ordering {
    a.archived
        .cmp(&b.archived)
        .then_with(|| b.updated_at.cmp(&a.updated_at))
}

/// Threads match their title, branch, pull request (`#12` or its title) and project; with no
/// query the palette shows the latest threads and every action.
pub fn search<'a>(
    query: &str,
    threads: &[&'a Thread],
    projects: &[&'a Project],
    pull_requests: &PullRequests,
    action_label: impl Fn(Action) -> String,
) -> Results<'a> {
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        let mut recent: Vec<&Thread> = threads
            .iter()
            .copied()
            .filter(|thread| !thread.archived)
            .collect();
        recent.sort_by(|a, b| newest(a, b));
        recent.truncate(RECENT_LIMIT);
        return Results {
            threads: recent,
            projects: Vec::new(),
            actions: ACTIONS.to_vec(),
        };
    }
    let titles: HashMap<&str, &str> = projects
        .iter()
        .map(|project| (project.id.as_str(), project.title.as_str()))
        .collect();
    let numbers: HashMap<&str, String> = pull_requests
        .iter()
        .map(|(id, pull_request)| (id.as_str(), format!("#{}", pull_request.number)))
        .collect();
    let thread_fields = |thread: &'a Thread| {
        let mut fields = vec![
            thread.title.as_str(),
            thread.branch.as_deref().unwrap_or(""),
            thread
                .project_id
                .as_deref()
                .and_then(|id| titles.get(id).copied())
                .unwrap_or(""),
        ];
        if let (Some(number), Some(pull_request)) = (
            numbers.get(thread.id.as_str()),
            pull_requests.get(&thread.id),
        ) {
            fields.push(number.as_str());
            fields.push(pull_request.title.as_str());
        }
        fields
    };
    let mut found_threads = ranked(threads.iter().copied(), thread_fields, &words, newest);
    found_threads.truncate(THREAD_LIMIT);
    let mut found_projects = ranked(
        projects.iter().copied(),
        |project: &'a Project| vec![project.title.as_str(), project.workspace_root.as_str()],
        &words,
        |a, b| a.title.cmp(&b.title),
    );
    found_projects.truncate(PROJECT_LIMIT);
    let labels: Vec<(Action, String)> = ACTIONS
        .iter()
        .map(|action| (*action, action_label(*action)))
        .collect();
    let actions = ranked(
        labels.iter(),
        |(_, label)| vec![label.as_str()],
        &words,
        |_, _| Ordering::Equal,
    )
    .into_iter()
    .map(|(action, _)| *action)
    .collect();
    Results {
        threads: found_threads,
        projects: found_projects,
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIONS, Action, score_field, search};
    use crate::sections::PullRequests;
    use hoshi_core::{Project, PullRequest, Thread};

    fn thread(id: &str, title: &str, updated_at: &str) -> Thread {
        let json = serde_json::json!({
            "id": id,
            "projectId": "p",
            "title": title,
            "status": "idle",
            "branch": null,
            "worktreePath": null,
            "sessionId": null,
            "conversationCursor": null,
            "resumeAt": null,
            "watchingPullRequest": false,
            "model": null,
            "effort": null,
            "fastMode": false,
            "ultracode": false,
            "accessMode": "acceptEdits",
            "planMode": false,
            "activeTurnId": null,
            "archived": false,
            "pinned": false,
            "createdAt": updated_at,
            "updatedAt": updated_at,
        });
        serde_json::from_value(json).unwrap()
    }

    fn project(id: &str, title: &str, root: &str) -> Project {
        let json = serde_json::json!({
            "id": id,
            "title": title,
            "workspaceRoot": root,
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z",
            "worktreeSetup": null,
        });
        serde_json::from_value(json).unwrap()
    }

    fn label(action: Action) -> String {
        match action {
            Action::NewThread => "New thread",
            Action::NewThreadFromPullRequest => "New thread from pull request",
            Action::AddProject => "Add project",
            Action::Inbox => "Inbox",
            Action::Settings => "Settings",
            Action::Usage => "Usage",
        }
        .into()
    }

    #[test]
    fn a_word_scores_by_where_it_matches() {
        assert_eq!(score_field("Fix login", "fix"), 3);
        assert_eq!(score_field("feat/login-page", "login"), 2);
        assert_eq!(score_field("Fix #12", "12"), 2);
        assert_eq!(score_field("relogin", "login"), 1);
        assert_eq!(score_field("Fix", "x y"), 0);
    }

    #[test]
    fn no_query_lists_recent_threads_and_every_action() {
        let mut archived = thread("old", "Archived", "2026-03-01T00:00:00Z");
        archived.archived = true;
        let threads = [
            thread("a", "First", "2026-01-01T00:00:00Z"),
            thread("b", "Second", "2026-02-01T00:00:00Z"),
            archived,
        ];
        let refs: Vec<&Thread> = threads.iter().collect();
        let results = search(" ", &refs, &[], &PullRequests::new(), label);
        let ids: Vec<&str> = results.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert!(results.projects.is_empty());
        assert_eq!(results.actions, ACTIONS);
    }

    #[test]
    fn every_word_must_match_and_better_matches_rank_first() {
        let mut branched = thread("b", "Polish", "2026-02-01T00:00:00Z");
        branched.branch = Some("feat/login".into());
        let threads = [
            thread("a", "relogin flow", "2026-03-01T00:00:00Z"),
            branched,
            thread("c", "Login page", "2026-01-01T00:00:00Z"),
        ];
        let refs: Vec<&Thread> = threads.iter().collect();
        let projects = [project("p", "hoshi", "/code/hoshi")];
        let project_refs: Vec<&Project> = projects.iter().collect();
        let results = search("login", &refs, &project_refs, &PullRequests::new(), label);
        let ids: Vec<&str> = results.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
        let results = search(
            "login hoshi",
            &refs,
            &project_refs,
            &PullRequests::new(),
            label,
        );
        assert_eq!(results.threads.len(), 3);
        assert_eq!(results.projects.len(), 0);
        let results = search("code", &refs, &project_refs, &PullRequests::new(), label);
        assert_eq!(results.projects[0].id, "p");
        let results = search("thread pull", &refs, &[], &PullRequests::new(), label);
        assert_eq!(results.actions, [Action::NewThreadFromPullRequest]);
    }

    #[test]
    fn a_thread_matches_its_pull_request() {
        let threads = [thread("a", "Something", "2026-01-01T00:00:00Z")];
        let refs: Vec<&Thread> = threads.iter().collect();
        let mut pull_requests = PullRequests::new();
        pull_requests.insert(
            "a".into(),
            PullRequest {
                number: 42,
                title: "Speed up search".into(),
                head_branch: "speed".into(),
                ..pull_request_defaults()
            },
        );
        assert_eq!(
            search("#42", &refs, &[], &pull_requests, label)
                .threads
                .len(),
            1
        );
        assert_eq!(
            search("speed", &refs, &[], &pull_requests, label)
                .threads
                .len(),
            1
        );
        assert_eq!(
            search("#43", &refs, &[], &pull_requests, label)
                .threads
                .len(),
            0
        );
    }

    fn pull_request_defaults() -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "forge": "github",
            "number": 0,
            "title": "",
            "url": "",
            "author": null,
            "headBranch": "",
            "headSha": null,
            "baseBranch": "",
            "state": "open",
            "checks": null,
            "unresolvedComments": 0,
            "openTasks": 0,
            "changesRequested": false,
            "additions": 0,
            "deletions": 0,
            "updatedAt": "2026-01-01T00:00:00Z",
        }))
        .unwrap()
    }
}
