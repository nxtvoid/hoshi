//! Every way to add a project: a folder on disk, a new repository, or a clone.

use super::folder_path::{self as path, Completion};
use super::frame::{CloneSource, Entry, Frame, Group, Mark, Row, Source, Submit, Summary, Target};
use super::{Mode, Palette};
use crate::folder_picker;
use crate::m;
use gpui_kit::{App, AppContext as _, AsyncWindowContext, Context, Task, WeakEntity, Window};
use hoshi_core::{Command, CommandError, FolderListing, ForgeKind, RemoteRepository};
use hoshi_engine::Engine;
use hoshi_ui::prelude::IconName;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub const SEPARATOR: char = std::path::MAIN_SEPARATOR;
const LIST_DELAY: Duration = Duration::from_millis(60);
/// More folders than this wait for the name to narrow them.
const FOLDER_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Sources,
    Folder,
    NewProject,
    Repository(CloneSource),
    Destination {
        source: CloneSource,
        url: String,
        title: String,
        subtitle: Option<String>,
    },
}

struct ForgeSource {
    forge: ForgeKind,
    title: fn() -> String,
    description: fn() -> String,
    placeholder: fn() -> String,
}

const FORGE_SOURCES: [ForgeSource; 2] = [
    ForgeSource {
        forge: ForgeKind::Github,
        title: m::add_project_github,
        description: m::add_project_github_description,
        placeholder: m::add_project_github_placeholder,
    },
    ForgeSource {
        forge: ForgeKind::Bitbucket,
        title: m::add_project_bitbucket,
        description: m::add_project_bitbucket_description,
        placeholder: m::add_project_bitbucket_placeholder,
    },
];

fn forge_source(forge: ForgeKind) -> &'static ForgeSource {
    FORGE_SOURCES
        .iter()
        .find(|source| source.forge == forge)
        .unwrap_or(&FORGE_SOURCES[0])
}

pub struct AddProject {
    /// The steps taken, each with the query it had; the last one's query is the field's.
    stack: Vec<(Step, String)>,
    /// Folder listings, read once per folder while the palette stays open; `None` when unreadable.
    listings: HashMap<String, Option<FolderListing>>,
    reading: HashSet<String>,
    list_task: Option<Task<()>>,
    pub error: Option<String>,
    pub pending: bool,
}

impl Default for AddProject {
    fn default() -> Self {
        Self {
            stack: vec![(Step::Sources, String::new())],
            listings: HashMap::new(),
            reading: HashSet::new(),
            list_task: None,
            error: None,
            pending: false,
        }
    }
}

impl AddProject {
    fn step(&self) -> &Step {
        self.stack.last().map_or(&Step::Sources, |(step, _)| step)
    }

    pub fn on_sources(&self) -> bool {
        *self.step() == Step::Sources
    }

    pub fn browsing(&self) -> bool {
        matches!(self.step(), Step::Folder | Step::Destination { .. })
    }

    pub fn placeholder(&self) -> String {
        match self.step() {
            Step::Sources => m::add_project_search(),
            Step::Folder | Step::Destination { .. } => m::add_project_path_placeholder(),
            Step::NewProject => m::add_project_name_placeholder(),
            Step::Repository(CloneSource::Url) => m::add_project_url_placeholder(),
            Step::Repository(CloneSource::Forge(forge)) => (forge_source(*forge).placeholder)(),
        }
    }

    /// The folder the query looks into, when it is a path the palette can list.
    fn listed_directory(&self, query: &str) -> Option<String> {
        let (directory, _) = path::split_query(query, SEPARATOR);
        (self.browsing() && path::is_absolute(directory, SEPARATOR)).then(|| directory.to_owned())
    }

    /// The folder's listing, and whether it is still being read.
    fn listing(&self, directory: Option<&str>) -> (Option<&FolderListing>, bool) {
        match directory.map(|directory| self.listings.get(directory)) {
            Some(Some(listing)) => (listing.as_ref(), false),
            Some(None) => (None, true),
            None => (None, false),
        }
    }
}

pub fn home() -> Option<String> {
    std::env::home_dir().map(|home| home.to_string_lossy().into_owned())
}

fn file_manager_label() -> String {
    if cfg!(target_os = "macos") {
        m::add_project_open_in_finder()
    } else if cfg!(windows) {
        m::add_project_open_in_explorer()
    } else {
        m::add_project_open_in_file_manager()
    }
}

fn same_name(left: &str, right: &str) -> bool {
    if SEPARATOR == '\\' {
        left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

pub(crate) fn same_path(a: &str, b: &str) -> bool {
    let normalize = |path: &str| path.replace('\\', "/").trim_end_matches('/').to_lowercase();
    normalize(a) == normalize(b)
}

fn folder_name(path: &str) -> String {
    path.split(['\\', '/'])
        .rfind(|part| !part.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn forge_host(forge: ForgeKind) -> &'static str {
    match forge {
        ForgeKind::Github => "github.com",
        ForgeKind::Bitbucket => "bitbucket.org",
    }
}

/// The host and path of a git URL in any form git takes (`https://host/owner/repo.git`,
/// `git@host:owner/repo` or `ssh://git@host:22/owner/repo`), as folder names.
fn url_folders(url: &str) -> Option<Vec<String>> {
    let url = url.trim();
    let url = url.split(['?', '#']).next()?;
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url
            .split_once(':')
            .filter(|(authority, _)| authority.len() > 1 && !authority.contains(['/', '\\']))?,
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let path = path.trim_matches('/');
    let path = match path.len().checked_sub(4) {
        Some(at) if path.is_char_boundary(at) && path[at..].eq_ignore_ascii_case(".git") => {
            &path[..at]
        }
        _ => path,
    };
    let folders: Vec<String> = std::iter::once(host.to_ascii_lowercase())
        .chain(path.split('/').map(str::to_owned))
        .collect();
    (folders.len() > 2
        && folders
            .iter()
            .all(|folder| path::is_valid_folder_name(folder)))
    .then_some(folders)
}

/// A forge repository's host and `owner/name`, as folder names.
fn forge_folders(forge: ForgeKind, full_name: &str) -> Option<Vec<String>> {
    url_folders(&format!("https://{}/{full_name}", forge_host(forge)))
}

/// Where a clone goes in `base`: its host folders when given, else a folder named `name`.
fn clone_destination(base: &str, name: &str, host_folders: Option<Vec<String>>) -> String {
    match host_folders {
        Some(folders) => folders.iter().fold(base.to_owned(), |folder, part| {
            path::join(&folder, part, SEPARATOR)
        }),
        None => path::join(base, name, SEPARATOR),
    }
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

pub fn error_message(error: &CommandError) -> String {
    match error {
        CommandError::FolderUnavailable { path } | CommandError::WorkspaceRootNotFound { path } => {
            m::error_folder_unavailable(path)
        }
        CommandError::FolderNotEmpty { path } => m::error_folder_not_empty(path),
        CommandError::RepositoryNameInvalid { .. } => m::error_repository_name_invalid(),
        CommandError::ForgeUnavailable => m::error_repository_not_found(),
        CommandError::GitFailed { detail } => m::error_git_failed(detail),
        CommandError::ForgeUnauthorized
        | CommandError::ForgeRateLimited
        | CommandError::ForgeRejected { .. }
        | CommandError::ForgeUnreachable { .. } => forge_error_message(error),
        _ => m::error_generic(),
    }
}

/// Adds the folder as a project, or finds the one already on it; returns the project's id.
pub(crate) fn add_at(
    engine: &Engine,
    path: &str,
    projects: &[(String, String)],
) -> Result<String, CommandError> {
    if let Some((id, _)) = projects.iter().find(|(_, root)| same_path(root, path)) {
        return Ok(id.clone());
    }
    let project_id = uuid::Uuid::new_v4().to_string();
    let created = engine.dispatch(Command::ProjectCreate {
        project_id: project_id.clone(),
        title: folder_name(path),
        workspace_root: path.to_owned(),
    });
    match created {
        Ok(()) => Ok(project_id),
        Err(CommandError::DuplicateWorkspaceRoot { project_id }) => Ok(project_id),
        Err(error) => Err(error),
    }
}

impl Palette {
    /// Where new projects and clones go: the folder set in settings, or the home folder.
    fn root_folder(&self, cx: &App) -> Option<String> {
        folder_picker::configured(
            self.workspace
                .read(cx)
                .settings
                .general
                .projects_folder
                .as_deref(),
        )
        .or_else(home)
    }

    fn clones_into_host_folders(&self, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .settings
            .general
            .clone_into_host_folders
    }

    fn known_projects(&self, cx: &App) -> Vec<(String, String)> {
        self.workspace
            .read(cx)
            .model
            .projects()
            .map(|project| (project.id.clone(), project.workspace_root.clone()))
            .collect()
    }

    /// Reads the folder unless it was read; the task ends once it is cached.
    fn fetch_listing(&mut self, directory: String, cx: &mut Context<Self>) -> Task<()> {
        if self.add.listings.contains_key(&directory) || !self.add.reading.insert(directory.clone())
        {
            return Task::ready(());
        }
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let read = directory.clone();
            let listing = cx
                .background_spawn(async move { engine.list_folders(&read).ok() })
                .await;
            this.update(cx, |this, cx| {
                this.add.reading.remove(&directory);
                this.add.listings.insert(directory, listing);
                cx.notify();
            })
            .ok();
        })
    }

    pub(super) fn schedule_listing(&mut self, cx: &mut Context<Self>) {
        let Some(directory) = self.add.listed_directory(&self.query) else {
            return;
        };
        if self.add.listings.contains_key(&directory) {
            return;
        }
        self.add.list_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LIST_DELAY).await;
            let Ok(read) = this.update(cx, |this, cx| this.fetch_listing(directory, cx)) else {
                return;
            };
            read.await;
        }));
    }

    /// Reads `directory` first, so the list changes once, then runs `then`.
    fn after_listing(
        &mut self,
        directory: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let read = match directory {
            Some(directory) => self.fetch_listing(directory, cx),
            None => Task::ready(()),
        };
        cx.spawn_in(window, async move |this, cx: &mut AsyncWindowContext| {
            read.await;
            this.update_in(cx, |this, window, cx| then(this, window, cx))
                .ok();
        })
        .detach();
    }

    fn push_step(
        &mut self,
        step: Step,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, kept)) = self.add.stack.last_mut() {
            kept.clone_from(&self.query);
        }
        self.add.stack.push((step, String::new()));
        self.add.error = None;
        self.set_query(query, window, cx);
    }

    pub(super) fn back_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add.pending {
            return;
        }
        self.add.error = None;
        if self.add.stack.len() > 1 {
            self.add.stack.pop();
            let query = self
                .add
                .stack
                .last()
                .map(|(_, query)| query.clone())
                .unwrap_or_default();
            self.set_query(query, window, cx);
        } else {
            self.set_mode(Mode::Commands, window, cx);
        }
    }

    pub(super) fn browse_to(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        let (directory, _) = path::split_query(&query, SEPARATOR);
        let directory = path::is_absolute(directory, SEPARATOR).then(|| directory.to_owned());
        self.after_listing(directory, window, cx, move |this, window, cx| {
            this.set_query(query, window, cx)
        });
    }

    pub(super) fn pick_source(
        &mut self,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match source {
            Source::New => self.push_step(Step::NewProject, String::new(), window, cx),
            Source::Clone(source) => {
                self.push_step(Step::Repository(source), String::new(), window, cx)
            }
            Source::Folder => {
                let Some(root) = self.root_folder(cx) else {
                    return self.push_step(Step::Folder, String::new(), window, cx);
                };
                let base = path::with_trailing_separator(&root, SEPARATOR);
                self.after_listing(Some(base.clone()), window, cx, move |this, window, cx| {
                    this.push_step(Step::Folder, base, window, cx)
                });
            }
        }
    }

    /// Runs `work` in the background while the palette shows it pending; a failure shows why.
    fn attempt<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Engine) -> Result<T, CommandError> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.add.pending {
            return;
        }
        self.add.pending = true;
        self.add.error = None;
        cx.notify();
        let engine: Arc<Engine> = self.workspace.read(cx).engine.clone();
        cx.spawn_in(
            window,
            async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
                let result = cx.background_spawn(async move { work(&engine) }).await;
                this.update_in(cx, |this, window, cx| {
                    this.add.pending = false;
                    match result {
                        Ok(value) => done(this, value, window, cx),
                        Err(error) => this.add.error = Some(error_message(&error)),
                    }
                    cx.notify();
                })
                .ok();
            },
        )
        .detach();
    }

    fn add_folder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        create: impl FnOnce(&Engine) -> Result<String, CommandError> + Send + 'static,
    ) {
        let projects = self.known_projects(cx);
        self.attempt(
            window,
            cx,
            move |engine| {
                let path = create(engine)?;
                add_at(engine, &path, &projects)
            },
            |this, project_id, _, cx| this.select_project(project_id, cx),
        );
    }

    fn folder_exists(&self) -> bool {
        let directory = self.add.listed_directory(&self.query);
        let (listing, _) = self.add.listing(directory.as_deref());
        let (_, name) = path::split_query(&self.query, SEPARATOR);
        if path::ends_with_separator(&self.query, SEPARATOR) {
            listing.is_some()
        } else {
            listing
                .is_some_and(|listing| listing.folders.iter().any(|folder| same_name(folder, name)))
        }
    }

    pub(super) fn submit_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.add.step().clone() {
            Step::Sources => {}
            Step::Folder => {
                let folder = path::resolve_query(&self.query, SEPARATOR);
                if !path::is_absolute(&folder, SEPARATOR) {
                    self.add.error = Some(m::error_path_not_absolute());
                    return cx.notify();
                }
                let exists = self.folder_exists();
                self.add_folder(window, cx, move |engine| {
                    if exists {
                        Ok(folder)
                    } else {
                        engine.create_folder(&folder, false)
                    }
                });
            }
            Step::NewProject => {
                if !path::is_valid_folder_name(&self.query) {
                    self.add.error = Some(m::error_project_name_invalid());
                    return cx.notify();
                }
                let Some(root) = self.root_folder(cx) else {
                    return;
                };
                let folder = path::join(&root, self.query.trim(), SEPARATOR);
                self.add_folder(window, cx, move |engine| {
                    engine.create_folder(&folder, true)
                });
            }
            Step::Repository(source) => self.submit_repository(source, window, cx),
            Step::Destination { url, .. } => {
                let destination = path::resolve_query(&self.query, SEPARATOR);
                if !path::is_absolute(&destination, SEPARATOR) {
                    self.add.error = Some(m::error_path_not_absolute());
                    return cx.notify();
                }
                self.add_folder(window, cx, move |engine| {
                    engine.clone_repository(&url, &destination)
                });
            }
        }
    }

    fn submit_repository(
        &mut self,
        source: CloneSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.query.trim().to_owned();
        let Some(base) = self.root_folder(cx).filter(|_| !input.is_empty()) else {
            return;
        };
        let listed = path::with_trailing_separator(&base, SEPARATOR);
        let host_folders = self.clones_into_host_folders(cx);
        match source {
            CloneSource::Url => {
                let folder = path::repository_folder_name(&input);
                if folder.is_empty() || !input.contains(['/', ':']) {
                    self.add.error = Some(m::error_git_url_invalid());
                    return cx.notify();
                }
                let hosted = host_folders.then(|| url_folders(&input)).flatten();
                let query = clone_destination(&base, &folder, hosted);
                let step = Step::Destination {
                    source,
                    url: input.clone(),
                    title: input,
                    subtitle: None,
                };
                let parent = path::split_query(&query, SEPARATOR).0.to_owned();
                self.after_listing(Some(parent), window, cx, move |this, window, cx| {
                    this.push_step(step, query, window, cx)
                });
            }
            CloneSource::Forge(forge) => {
                let read = listed.clone();
                self.attempt(
                    window,
                    cx,
                    move |engine| {
                        let repository = engine.find_repository(forge, &input)?;
                        Ok((repository, engine.list_folders(&read).ok()))
                    },
                    move |this, (repository, listing): (RemoteRepository, _), window, cx| {
                        this.add.listings.insert(listed, listing);
                        let hosted = host_folders
                            .then(|| forge_folders(forge, &repository.full_name))
                            .flatten();
                        let query = clone_destination(&base, &repository.name, hosted);
                        let parent = path::split_query(&query, SEPARATOR).0.to_owned();
                        let step = Step::Destination {
                            source,
                            url: repository.clone_url,
                            title: repository.full_name,
                            subtitle: Some(repository.description.unwrap_or(repository.web_url)),
                        };
                        this.after_listing(Some(parent), window, cx, move |this, window, cx| {
                            this.push_step(step, query, window, cx)
                        });
                    },
                );
            }
        }
    }

    pub(super) fn pick_with_system_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add.pending {
            return;
        }
        let start = folder_picker::start_folder([
            self.add.listed_directory(&self.query),
            self.root_folder(cx),
        ]);
        let picked = folder_picker::pick_folder(start, Some(m::add_project().into()), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(picked) = picked.await else {
                return;
            };
            let picked = picked.to_string_lossy().into_owned();
            this.update_in(cx, |this, window, cx| {
                this.add_folder(window, cx, move |_| Ok(picked))
            })
            .ok();
        })
        .detach();
    }

    /// Tab with nothing highlighted completes the folder name as far as the folders agree.
    pub(super) fn complete_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if *self.add.step() != Step::Folder {
            return;
        }
        let directory = self.add.listed_directory(&self.query);
        let (Some(listing), _) = self.add.listing(directory.as_deref()) else {
            return;
        };
        let (directory, name) = path::split_query(&self.query, SEPARATOR);
        let shown = path::filter_folders(&listing.folders, name);
        let Some(Completion { name, complete }) = path::complete_name(&shown, name) else {
            return;
        };
        let completed = path::join(directory, &name, SEPARATOR);
        if complete {
            self.browse_to(
                path::with_trailing_separator(&completed, SEPARATOR),
                window,
                cx,
            );
        } else {
            self.set_query(completed, window, cx);
        }
    }

    fn sources_groups(&self, cx: &App) -> Vec<Group> {
        let logins = &self.workspace.read(cx).logins;
        let two_line = |mark: Mark, title: String, description: String| Row::TwoLine {
            mark,
            title,
            description,
            disabled: false,
            setup: false,
        };
        let mut entries = vec![
            (
                format!(
                    "{} {} git init",
                    m::add_project_new(),
                    m::add_project_new_description()
                ),
                Entry::new(
                    "new",
                    Target::Source(Source::New),
                    two_line(
                        Mark::Icon(IconName::FolderGit),
                        m::add_project_new(),
                        m::add_project_new_description(),
                    ),
                ),
            ),
            (
                format!(
                    "{} {}",
                    m::add_project_local(),
                    m::add_project_local_description()
                ),
                Entry::new(
                    "folder",
                    Target::Source(Source::Folder),
                    two_line(
                        Mark::Icon(IconName::FolderPlus),
                        m::add_project_local(),
                        m::add_project_local_description(),
                    ),
                ),
            ),
            (
                format!(
                    "{} {} clone",
                    m::add_project_git_url(),
                    m::add_project_git_url_description()
                ),
                Entry::new(
                    "url",
                    Target::Source(Source::Clone(CloneSource::Url)),
                    two_line(
                        Mark::Icon(IconName::Link),
                        m::add_project_git_url(),
                        m::add_project_git_url_description(),
                    ),
                ),
            ),
        ];
        for source in &FORGE_SOURCES {
            let signed_out = !logins.contains_key(&source.forge);
            let mut entry = Entry::new(
                match source.forge {
                    ForgeKind::Github => "github",
                    ForgeKind::Bitbucket => "bitbucket",
                },
                Target::Source(Source::Clone(CloneSource::Forge(source.forge))),
                Row::TwoLine {
                    mark: Mark::Forge(source.forge),
                    title: (source.title)(),
                    description: (source.description)(),
                    disabled: signed_out,
                    setup: signed_out,
                },
            );
            entry.disabled = signed_out;
            entries.push((
                format!("{} {} clone", (source.title)(), (source.description)()),
                entry,
            ));
        }
        let needle = self.query.trim().to_lowercase();
        let found: Vec<Entry> = entries
            .into_iter()
            .filter(|(search, _)| search.to_lowercase().contains(&needle))
            .map(|(_, entry)| entry)
            .collect();
        if found.is_empty() {
            Vec::new()
        } else {
            vec![Group::new(m::add_project_sources(), found)]
        }
    }

    fn folder_groups(&self, listing: Option<&FolderListing>) -> Vec<Group> {
        let Some(listing) = listing else {
            return Vec::new();
        };
        let destination = matches!(self.add.step(), Step::Destination { .. });
        let (directory, name) = path::split_query(&self.query, SEPARATOR);
        let clone_name = if destination { name } else { "" };
        let into = |folder: &str| {
            let folder = path::join(directory, folder, SEPARATOR);
            if destination {
                path::join(&folder, clone_name, SEPARATOR)
            } else {
                path::with_trailing_separator(&folder, SEPARATOR)
            }
        };
        let mut entries = Vec::new();
        if let Some(parent) = &listing.parent
            && (destination || name.is_empty())
        {
            let up = if destination {
                path::join(parent, clone_name, SEPARATOR)
            } else {
                path::with_trailing_separator(parent, SEPARATOR)
            };
            entries.push(Entry::new("up", Target::Browse(up), Row::Folder(None)));
        }
        let filter = if destination { "" } else { name };
        for folder in path::filter_folders(&listing.folders, filter)
            .into_iter()
            .take(FOLDER_LIMIT)
        {
            entries.push(Entry::new(
                format!("folder:{folder}"),
                Target::Browse(into(folder)),
                Row::Folder(Some(folder.to_owned())),
            ));
        }
        if entries.is_empty() {
            return Vec::new();
        }
        let label = if destination {
            m::add_project_clone_into()
        } else {
            m::add_project_folders()
        };
        vec![Group::new(label, entries)]
    }

    pub(super) fn add_project_frame(&self, cx: &App) -> Frame {
        let directory = self.add.listed_directory(&self.query);
        let (listing, loading) = self.add.listing(directory.as_deref());
        let pending = self.add.pending;
        let blank = self.query.trim().is_empty();
        let submit = |label: String| Submit {
            label,
            pending,
            disabled: pending || blank,
        };
        let step = self.add.step().clone();
        let groups = match step {
            Step::Sources => self.sources_groups(cx),
            Step::Folder | Step::Destination { .. } => self.folder_groups(listing),
            _ => Vec::new(),
        };
        let mut frame = Frame::new(groups, Some(m::palette_no_results()));
        let highlighted = self.current(frame.len()).is_some();
        frame.back = true;
        frame.enter_label = m::palette_select();
        frame.status = self.add.error.clone();
        match step {
            Step::Sources => {}
            Step::Folder => {
                let exists = self.folder_exists();
                let label = if pending {
                    m::add_project_adding()
                } else if exists || loading {
                    m::add_project_add()
                } else {
                    m::add_project_create_and_add()
                };
                if !highlighted {
                    frame.enter_label = label.clone();
                }
                frame.submit = Some(submit(label));
                frame.submits = true;
                frame.tab = true;
                frame.footer = Some(file_manager_label());
                frame.empty = if blank {
                    None
                } else if directory.is_none() {
                    Some(m::error_path_not_absolute())
                } else if loading {
                    Some(m::add_project_loading_folders())
                } else if exists {
                    Some(m::add_project_no_folders())
                } else {
                    Some(m::add_project_will_create())
                };
            }
            Step::NewProject => {
                let root = self.root_folder(cx);
                frame.submit = Some(submit(if pending {
                    m::add_project_creating()
                } else {
                    m::add_project_create()
                }));
                frame.submits = true;
                frame.enter_label = m::add_project_create();
                frame.empty = None;
                frame.summary = Some(Summary {
                    mark: Mark::Icon(IconName::FolderGit),
                    title: match &root {
                        Some(root) if !blank => path::join(root, self.query.trim(), SEPARATOR),
                        _ => m::add_project_new_location(root.as_deref().unwrap_or("…")),
                    },
                    subtitle: Some(m::add_project_new_git()),
                });
            }
            Step::Repository(source) => {
                let forge = match source {
                    CloneSource::Forge(forge) => Some(forge),
                    CloneSource::Url => None,
                };
                frame.submit = Some(submit(match (pending, forge) {
                    (true, _) => m::add_project_looking_up(),
                    (false, Some(_)) => m::add_project_look_up(),
                    (false, None) => m::add_project_continue(),
                }));
                frame.submits = true;
                frame.enter_label = match forge {
                    Some(_) => m::add_project_look_up(),
                    None => m::add_project_continue(),
                };
                frame.empty = Some(match forge {
                    Some(_) => m::add_project_repository_hint(),
                    None => m::add_project_url_hint(),
                });
            }
            Step::Destination {
                source,
                title,
                subtitle,
                ..
            } => {
                frame.submit = Some(submit(if pending {
                    m::add_project_cloning()
                } else {
                    m::add_project_clone()
                }));
                frame.submits = true;
                frame.tab = true;
                if !highlighted {
                    frame.enter_label = m::add_project_clone();
                }
                frame.empty = Some(if loading {
                    m::add_project_loading_folders()
                } else {
                    m::add_project_clone_hint()
                });
                frame.summary = Some(Summary {
                    mark: match source {
                        CloneSource::Url => Mark::Icon(IconName::Link),
                        CloneSource::Forge(forge) => Mark::Forge(forge),
                    },
                    title,
                    subtitle,
                });
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SEPARATOR, clone_destination, error_message, folder_name, forge_folders, same_path,
        url_folders,
    };
    use hoshi_core::{CommandError, ForgeKind};

    fn folders(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|part| (*part).to_owned()).collect())
    }

    #[test]
    fn git_urls_name_their_host_and_owner_folders() {
        let expected = folders(&["github.com", "nxtvoid", "hoshi"]);
        for url in [
            "https://github.com/nxtvoid/hoshi.git",
            "https://GitHub.com/nxtvoid/hoshi/",
            "git@github.com:nxtvoid/hoshi.git",
            "ssh://git@github.com:22/nxtvoid/hoshi",
        ] {
            assert_eq!(url_folders(url), expected, "{url}");
        }
        assert_eq!(
            url_folders("https://gitlab.example.com/group/sub/app.git"),
            folders(&["gitlab.example.com", "group", "sub", "app"])
        );
        assert_eq!(url_folders("C:/code/app.git"), None);
        assert_eq!(url_folders("/srv/git/app.git"), None);
        assert_eq!(url_folders("https://github.com/app.git"), None);
        assert_eq!(url_folders("https://github.com/owner/../app"), None);
        assert_eq!(
            forge_folders(ForgeKind::Bitbucket, "team/app"),
            folders(&["bitbucket.org", "team", "app"])
        );
    }

    #[test]
    fn a_clone_goes_into_host_folders_only_when_given() {
        let sep = SEPARATOR;
        assert_eq!(
            clone_destination("/code", "hoshi", None),
            format!("/code{sep}hoshi")
        );
        assert_eq!(
            clone_destination(
                &format!("/code{sep}"),
                "hoshi",
                folders(&["github.com", "nxtvoid", "hoshi"])
            ),
            format!("/code{sep}github.com{sep}nxtvoid{sep}hoshi")
        );
    }

    #[test]
    fn paths_compare_loosely_and_name_their_folder() {
        assert!(same_path("C:\\code\\app\\", "c:/code/app"));
        assert!(!same_path("/code/app", "/code/apps"));
        assert_eq!(folder_name("C:\\code\\app\\"), "app");
        assert_eq!(folder_name("/home/nxtvoid/hoshi"), "hoshi");
    }

    #[test]
    fn failures_name_the_folder_or_the_forge() {
        assert!(
            error_message(&CommandError::FolderNotEmpty {
                path: "/code/app".into()
            })
            .contains("/code/app")
        );
        assert_eq!(
            error_message(&CommandError::ForgeRejected {
                detail: "private".into()
            }),
            "private"
        );
    }
}
