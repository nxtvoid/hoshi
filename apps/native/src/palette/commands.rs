use super::frame::{Entry, Frame, Group, Mark, Row, Target};
use super::pull_requests::problem;
use super::search::{self, Action};
use super::{Mode, Palette, PaletteEvent};
use crate::m;
use crate::sections::View;
use crate::settings::Section;
use gpui_kit::{App, Context, Window};
use hoshi_core::{Project, Thread};
use hoshi_ui::prelude::IconName;

pub fn action_label(action: Action) -> String {
    match action {
        Action::NewThread => m::new_thread(),
        Action::NewThreadFromPullRequest => m::new_thread_from_pull_request(),
        Action::AddProject => m::add_project(),
        Action::Inbox => m::inbox(),
        Action::Settings => m::settings(),
        Action::Usage => m::usage(),
    }
}

fn action_icon(action: Action) -> IconName {
    match action {
        Action::NewThread => IconName::SquarePen,
        Action::NewThreadFromPullRequest => IconName::GitPullRequest,
        Action::AddProject => IconName::FolderPlus,
        Action::Inbox => IconName::Inbox,
        Action::Settings => IconName::Settings,
        Action::Usage => IconName::ChartColumn,
    }
}

fn view_of(thread: &Thread) -> View {
    match thread.project_id {
        Some(_) => View::Project,
        None => View::Chats,
    }
}

impl Palette {
    /// What the main view is on: the open thread's project, or the draft's; `Some(None)` for a
    /// chat.
    pub(super) fn scope(&self, cx: &App) -> Option<Option<String>> {
        if let Some(sidebar) = &self.sidebar {
            return match sidebar.view {
                View::Project => sidebar.project_id.clone().map(Some),
                View::Chats => Some(None),
                View::Inbox => None,
            };
        }
        let workspace = self.workspace.read(cx);
        if let Some(id) = workspace.open_thread_id(cx) {
            return workspace
                .model
                .thread(&id)
                .map(|thread| thread.project_id.clone());
        }
        workspace
            .draft
            .as_ref()
            .map(|draft| draft.project_id.clone())
    }

    pub(super) fn selected_project(&self, cx: &App) -> Option<String> {
        self.scope(cx).flatten()
    }

    pub(super) fn commands_frame(&self, cx: &App) -> Frame {
        let workspace = self.workspace.read(cx);
        let threads: Vec<&Thread> = workspace.model.threads().collect();
        let projects: Vec<&Project> = workspace.model.projects().collect();
        let results = search::search(
            &self.query,
            &threads,
            &projects,
            &workspace.pull_requests,
            action_label,
        );
        let has_project = self.selected_project(cx).is_some();
        let title_of = |id: &str| {
            projects
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.title.clone())
        };
        let thread_entries = results
            .threads
            .iter()
            .map(|thread| {
                let project = match &thread.project_id {
                    None => Some(m::chat()),
                    Some(id) if projects.len() > 1 => title_of(id),
                    Some(_) => None,
                };
                let pull_request = workspace.pull_requests.get(&thread.id).map(|pull_request| {
                    (
                        pull_request.state,
                        problem(pull_request),
                        pull_request.number,
                    )
                });
                Entry::new(
                    format!("thread:{}", thread.id),
                    Target::Thread(thread.id.clone()),
                    Row::Thread {
                        title: thread.title.clone(),
                        provider: thread.provider.clone(),
                        archived: thread.archived,
                        project,
                        branch: thread.branch.clone(),
                        worktree: thread.worktree_path.is_some(),
                        pull_request,
                    },
                )
            })
            .collect();
        let project_entries = results
            .projects
            .iter()
            .map(|project| {
                Entry::new(
                    format!("project:{}", project.id),
                    Target::Project(project.id.clone()),
                    Row::Project {
                        id: project.id.clone(),
                        title: project.title.clone(),
                        root: project.workspace_root.clone(),
                    },
                )
            })
            .collect();
        let action_entries = results
            .actions
            .into_iter()
            .filter(|action| has_project || !action.needs_project())
            .map(|action| {
                Entry::new(
                    format!("action:{action:?}"),
                    Target::Action(action),
                    Row::Plain {
                        mark: Mark::Icon(action_icon(action)),
                        label: action_label(action),
                    },
                )
            })
            .collect();
        let threads_label = if self.query.trim().is_empty() {
            m::palette_recent()
        } else {
            m::palette_threads()
        };
        let groups = [
            Group::new(threads_label, thread_entries),
            Group::new(m::palette_projects(), project_entries),
            Group::new(m::palette_actions(), action_entries),
        ]
        .into_iter()
        .filter(|group| !group.entries.is_empty())
        .collect();
        Frame::new(groups, Some(m::palette_no_results()))
    }

    /// No project at all for a chat, set apart at the top, or a project, the selected one first.
    pub(super) fn new_thread_frame(&self, cx: &App) -> Frame {
        let workspace = self.workspace.read(cx);
        let needle = self.query.trim().to_lowercase();
        let matches = |fields: &[&str]| {
            fields
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        };
        let selected = self.selected_project(cx);
        let mut projects: Vec<&Project> = workspace.model.projects().collect();
        projects.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        if let Some(sidebar) = &self.sidebar {
            let order = &sidebar.project_order;
            projects.sort_by_key(|project| {
                order
                    .iter()
                    .position(|id| *id == project.id)
                    .unwrap_or(order.len())
            });
        }
        projects.sort_by_key(|project| Some(&project.id) != selected.as_ref());
        let project_entries: Vec<Entry> = projects
            .into_iter()
            .filter(|project| matches(&[&project.title, &project.workspace_root]))
            .map(|project| {
                Entry::new(
                    project.id.clone(),
                    Target::StartIn(Some(project.id.clone())),
                    Row::TwoLine {
                        mark: Mark::Project(project.id.clone()),
                        title: project.title.clone(),
                        description: project.workspace_root.clone(),
                        disabled: false,
                        setup: false,
                    },
                )
            })
            .collect();
        let mut groups = Vec::new();
        if matches(&[&m::no_project(), &m::chat(), &m::no_project_description()]) {
            let mut chat = Group::new(
                "",
                vec![Entry::new(
                    "chat",
                    Target::StartIn(None),
                    Row::TwoLine {
                        mark: Mark::Icon(IconName::MessageCircle),
                        title: m::no_project(),
                        description: m::no_project_description(),
                        disabled: false,
                        setup: false,
                    },
                )],
            );
            chat.divider = !project_entries.is_empty();
            groups.push(chat);
        }
        if !project_entries.is_empty() {
            groups.push(Group::new(m::palette_projects(), project_entries));
        }
        let mut frame = Frame::new(groups, Some(m::palette_no_results()));
        frame.back = true;
        frame.enter_label = m::palette_select();
        frame
    }

    fn show_view(&self, view: View, project_id: Option<String>, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::ShowView { view, project_id });
    }

    pub(super) fn open_thread(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let thread = self.workspace.read(cx).model.thread(thread_id).cloned();
        if let Some(thread) = thread {
            self.show_view(view_of(&thread), thread.project_id.clone(), cx);
        }
        self.workspace
            .update(cx, |workspace, cx| workspace.open_thread(thread_id, cx));
        self.close(cx);
    }

    pub(super) fn select_project(&mut self, project_id: String, cx: &mut Context<Self>) {
        self.start_in(Some(project_id), cx);
    }

    /// Opens the project, or the chats for `None`, on an empty composer. The thread only exists
    /// once its first message is sent.
    pub(super) fn start_in(&mut self, project_id: Option<String>, cx: &mut Context<Self>) {
        let view = match project_id {
            Some(_) => View::Project,
            None => View::Chats,
        };
        self.show_view(view, project_id.clone(), cx);
        self.workspace
            .update(cx, |workspace, cx| workspace.start_draft(project_id, cx));
        self.close(cx);
    }

    /// A new thread where the main view is, or asks where when it is on nothing.
    pub(super) fn new_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let no_projects = self.workspace.read(cx).model.projects().next().is_none();
        match self.scope(cx) {
            Some(scope) => self.start_in(scope, cx),
            None if no_projects => self.start_in(None, cx),
            None => self.set_mode(Mode::NewThread, window, cx),
        }
    }

    pub(super) fn run_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::NewThread => self.new_thread(window, cx),
            Action::NewThreadFromPullRequest => self.set_mode(Mode::PullRequests, window, cx),
            Action::AddProject => self.set_mode(Mode::AddProject, window, cx),
            Action::Inbox => {
                self.show_view(View::Inbox, None, cx);
                self.close(cx);
            }
            Action::Settings => cx.emit(PaletteEvent::OpenSettings(Section::General)),
            Action::Usage => cx.emit(PaletteEvent::OpenUsage),
        }
    }
}
