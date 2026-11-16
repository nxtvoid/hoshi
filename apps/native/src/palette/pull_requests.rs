use super::frame::{Entry, Frame, Group, Problem, Row, Target};
use super::{Palette, PaletteEvent};
use crate::m;
use crate::sections::{PullRequests, View};
use crate::state::Workspace;
use gpui_kit::{App, AppContext as _, Context, Task};
use hoshi_core::{
    ChecksState, Command, CommandError, PullRequest, PullRequestState, Thread, ThreadWorkspace,
};
use std::time::Duration;

const SEARCH_DELAY: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct PullRequestSearch {
    /// `None` until the first search answers.
    results: Option<Vec<PullRequest>>,
    error: Option<String>,
    task: Option<Task<()>>,
}

/// What holds an open pull request back, as its icon shows.
pub fn problem(pull_request: &PullRequest) -> Option<Problem> {
    if pull_request.state != PullRequestState::Open {
        None
    } else if pull_request.conflicts || pull_request.checks == Some(ChecksState::Failing) {
        Some(Problem::Other)
    } else if pull_request.changes_requested {
        Some(Problem::ChangesRequested)
    } else {
        None
    }
}

/// The project's thread already on the pull request: by its number, or by its branch.
pub fn thread_on<'a>(
    threads: impl IntoIterator<Item = &'a Thread>,
    pull_requests: &PullRequests,
    project_id: &str,
    pull_request: &PullRequest,
) -> Option<&'a Thread> {
    threads.into_iter().find(|thread| {
        thread.project_id.as_deref() == Some(project_id)
            && !thread.archived
            && (pull_requests
                .get(&thread.id)
                .is_some_and(|known| known.number == pull_request.number)
                || thread.branch.as_deref() == Some(pull_request.head_branch.as_str()))
    })
}

fn forge_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ForgeUnauthorized => m::forge_error_unauthorized(),
        CommandError::ForgeRateLimited => m::forge_error_rate_limited(),
        CommandError::ForgeRejected { detail } => detail.clone(),
        CommandError::ForgeUnreachable { .. } => m::forge_error_unreachable(),
        _ => m::forge_error_unavailable(),
    }
}

pub fn start_error_message(error: &CommandError) -> String {
    match error {
        CommandError::WorktreeFailed { detail } => m::pull_request_thread_failed(detail),
        CommandError::NotAGitRepository { .. } => m::error_not_a_git_repository(),
        CommandError::ForgeUnavailable => m::pull_requests_need_forge(),
        _ => forge_error_message(error),
    }
}

fn existing(workspace: &Workspace, project_id: &str, pull_request: &PullRequest) -> Option<String> {
    thread_on(
        workspace.model.threads(),
        &workspace.pull_requests,
        project_id,
        pull_request,
    )
    .map(|thread| thread.id.clone())
}

impl Palette {
    pub(super) fn search_pull_requests(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.selected_project(cx) else {
            return;
        };
        let engine = self.workspace.read(cx).engine.clone();
        let query = self.query.clone();
        self.pulls.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let found = cx
                .background_spawn(async move { engine.search_pull_requests(&project_id, &query) })
                .await;
            this.update(cx, |this, cx| {
                match found {
                    Ok(found) => {
                        this.pulls.results = Some(
                            found
                                .into_iter()
                                .filter(|pull_request| {
                                    matches!(
                                        pull_request.state,
                                        PullRequestState::Open | PullRequestState::Draft
                                    )
                                })
                                .collect(),
                        );
                        this.pulls.error = None;
                    }
                    Err(error) => {
                        this.pulls.results = Some(Vec::new());
                        this.pulls.error = Some(start_error_message(&error));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn pull_requests_frame(&self, cx: &App) -> Frame {
        let workspace = self.workspace.read(cx);
        let project_id = self.selected_project(cx).unwrap_or_default();
        let entries: Vec<Entry> = self
            .pulls
            .results
            .iter()
            .flatten()
            .map(|pull_request| {
                Entry::new(
                    format!("pr:{}", pull_request.number),
                    Target::PullRequest(Box::new(pull_request.clone())),
                    Row::PullRequest {
                        state: pull_request.state,
                        problem: problem(pull_request),
                        number: pull_request.number,
                        title: pull_request.title.clone(),
                        branch: pull_request.head_branch.clone(),
                        existing: existing(workspace, &project_id, pull_request).is_some(),
                    },
                )
            })
            .collect();
        let groups = if entries.is_empty() {
            Vec::new()
        } else {
            vec![Group::new(m::palette_pull_requests(), entries)]
        };
        let empty = match (&self.pulls.results, &self.pulls.error) {
            (None, _) => Some(m::loading_pull_requests()),
            (Some(_), Some(_)) => None,
            (Some(_), None) => Some(m::no_open_pull_requests()),
        };
        let mut frame = Frame::new(groups, empty);
        frame.status = self.pulls.error.clone();
        frame
    }

    /// Opens the pull request's thread, or starts one in a worktree on its branch.
    pub(super) fn start_from_pull_request(
        &mut self,
        key: String,
        pull_request: PullRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.selected_project(cx) else {
            return;
        };
        cx.emit(PaletteEvent::ShowView {
            view: View::Project,
            project_id: Some(project_id.clone()),
        });
        if let Some(thread_id) = existing(self.workspace.read(cx), &project_id, &pull_request) {
            self.workspace
                .update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
            self.close(cx);
            return;
        }
        let workspace = self.workspace.read(cx);
        let choices = workspace.new_thread_choices(Some(&project_id));
        let thread_id = uuid::Uuid::new_v4().to_string();
        let create = Command::ThreadCreate {
            thread_id: thread_id.clone(),
            project_id: Some(project_id),
            title: pull_request.title,
            workspace: ThreadWorkspace::BranchWorktree {
                branch: pull_request.head_branch,
            },
            provider: choices.model.provider.clone(),
            model: choices.model.model.clone(),
            effort: choices.model.effort,
            fast_mode: choices.model.fast_mode,
            ultracode: choices.model.ultracode,
            access_mode: choices.access_mode,
            plan_mode: choices.plan_mode,
        };
        let engine = workspace.engine.clone();
        self.busy = Some(key);
        self.pulls.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let created = cx
                .background_spawn(async move { engine.dispatch(create) })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match created {
                    Ok(()) => {
                        this.workspace
                            .update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
                        this.close(cx);
                    }
                    Err(error) => this.pulls.error = Some(start_error_message(&error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{Problem, problem, start_error_message, thread_on};
    use crate::sections::PullRequests;
    use hoshi_core::{ChecksState, CommandError, PullRequest, PullRequestState, Thread};

    fn pull_request(number: u32, branch: &str) -> PullRequest {
        serde_json::from_value(serde_json::json!({
            "forge": "github",
            "number": number,
            "title": "",
            "url": "",
            "author": null,
            "headBranch": branch,
            "baseBranch": "main",
            "state": "open",
            "checks": null,
            "unresolvedComments": 0,
            "additions": 0,
            "deletions": 0,
            "updatedAt": "2026-01-01T00:00:00Z",
        }))
        .unwrap()
    }

    fn thread(id: &str, branch: Option<&str>) -> Thread {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "projectId": "p",
            "title": id,
            "status": "idle",
            "branch": branch,
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
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn a_pull_request_finds_its_thread_by_number_or_branch() {
        let mut archived = thread("old", Some("fix"));
        archived.archived = true;
        let threads = [archived, thread("a", Some("feat")), thread("b", None)];
        let mut known = PullRequests::new();
        known.insert("b".into(), pull_request(7, "other"));
        let found = |number, branch| {
            thread_on(&threads, &known, "p", &pull_request(number, branch)).map(|t| t.id.clone())
        };
        assert_eq!(found(1, "feat"), Some("a".into()));
        assert_eq!(found(7, "x"), Some("b".into()));
        assert_eq!(found(2, "fix"), None);
        assert_eq!(
            thread_on(&threads, &known, "q", &pull_request(1, "feat")).map(|t| t.id.clone()),
            None
        );
    }

    #[test]
    fn only_an_open_pull_request_has_a_problem() {
        let mut open = pull_request(1, "a");
        assert_eq!(problem(&open), None);
        open.changes_requested = true;
        assert_eq!(problem(&open), Some(Problem::ChangesRequested));
        open.checks = Some(ChecksState::Failing);
        assert_eq!(problem(&open), Some(Problem::Other));
        open.state = PullRequestState::Draft;
        assert_eq!(problem(&open), None);
    }

    #[test]
    fn a_failed_start_says_why() {
        assert!(
            start_error_message(&CommandError::WorktreeFailed {
                detail: "locked".into()
            })
            .contains("locked")
        );
        assert_eq!(
            start_error_message(&CommandError::ForgeRejected {
                detail: "nope".into()
            }),
            "nope"
        );
    }
}
