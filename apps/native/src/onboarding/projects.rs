//! The projects band: the folders the agents worked in, picked to become projects, and the
//! ways to add any other.

use super::page::{AfterExit, Onboarding, OnboardingEvent, band, row_icon};
use crate::folder_picker;
use crate::m;
use crate::palette::add_project::{add_at, error_message, same_path};
use crate::sections::relative_time;
use chrono::{DateTime, Utc};
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_engine::RecentFolder;
use hoshi_ui::prelude::*;
use std::collections::HashSet;

/// Folders worked in this recently are picked from the start.
const RECENT_DAYS: i64 = 30;
const ROW_HEIGHT: f32 = 30.;
/// Longer lists scroll.
const SHOWN_ROWS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Off,
    On,
    Partial,
}

fn recent(folder: &RecentFolder, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&folder.active_at)
        .is_ok_and(|at| now.signed_duration_since(at).num_days() < RECENT_DAYS)
}

/// The folders picked before the user picks: the new ones worked in lately.
pub(super) fn first_pick(folders: &[RecentFolder], now: DateTime<Utc>) -> HashSet<String> {
    folders
        .iter()
        .filter(|folder| !folder.added && recent(folder, now))
        .map(|folder| folder.path.clone())
        .collect()
}

/// What checking the band's box picks: the recent new folders, else every new one.
pub(super) fn default_pick(folders: &[RecentFolder], now: DateTime<Utc>) -> HashSet<String> {
    let recent = first_pick(folders, now);
    if !recent.is_empty() {
        return recent;
    }
    folders
        .iter()
        .filter(|folder| !folder.added)
        .map(|folder| folder.path.clone())
        .collect()
}

impl Onboarding {
    /// Reads the folders the agents worked in, which takes a moment.
    pub(super) fn scan(&mut self, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        self.scan = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { engine.recent_folders() })
                .await;
            this.update(cx, |this, cx| this.receive_folders(found, cx))
                .ok();
        }));
    }

    pub(super) fn receive_folders(&mut self, found: Vec<RecentFolder>, cx: &mut Context<Self>) {
        let mut folders: Vec<RecentFolder> = self
            .folders
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter(|folder| {
                folder.providers.is_empty()
                    && !found
                        .iter()
                        .any(|other| same_path(&other.path, &folder.path))
            })
            .collect();
        folders.extend(found);
        if self.picked {
            self.selected.retain(|path| {
                folders
                    .iter()
                    .any(|folder| folder.path == *path && !folder.added)
            });
        } else {
            self.selected = first_pick(&folders, Utc::now());
        }
        self.folders = Some(folders);
        cx.notify();
    }

    /// The picked folders that aren't projects yet, as listed.
    pub(super) fn to_add(&self) -> Vec<&RecentFolder> {
        self.folders
            .iter()
            .flatten()
            .filter(|folder| !folder.added && self.selected.contains(&folder.path))
            .collect()
    }

    fn check_state(&self) -> Check {
        let folders = self.folders.as_deref().unwrap_or_default();
        let picked = self.to_add().len();
        let new = folders.iter().filter(|folder| !folder.added).count();
        if picked == 0 {
            Check::Off
        } else if picked == new || self.selected == default_pick(folders, Utc::now()) {
            Check::On
        } else {
            Check::Partial
        }
    }

    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        self.picked = true;
        self.selected = if self.to_add().is_empty() {
            default_pick(self.folders.as_deref().unwrap_or_default(), Utc::now())
        } else {
            HashSet::new()
        };
        cx.notify();
    }

    fn pick_all(&mut self, all: bool, cx: &mut Context<Self>) {
        self.picked = true;
        self.selected = if all {
            self.folders
                .iter()
                .flatten()
                .filter(|folder| !folder.added)
                .map(|folder| folder.path.clone())
                .collect()
        } else {
            HashSet::new()
        };
        cx.notify();
    }

    fn toggle(&mut self, path: String, cx: &mut Context<Self>) {
        self.picked = true;
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
        cx.notify();
    }

    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = folder_picker::start_folder([folder_picker::configured(
            self.workspace
                .read(cx)
                .settings
                .general
                .projects_folder
                .as_deref(),
        )]);
        let picked = folder_picker::pick_folder(start, None, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Some(path) = picked.await {
                this.update(cx, |this, cx| {
                    this.add_opened(path.to_string_lossy().into_owned(), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Lists a folder the user opened, picked, at the top.
    pub(super) fn add_opened(&mut self, path: String, cx: &mut Context<Self>) {
        self.picked = true;
        self.error = None;
        let folders = self.folders.get_or_insert_with(Vec::new);
        if let Some(folder) = folders.iter().find(|folder| same_path(&folder.path, &path)) {
            if !folder.added {
                self.selected.insert(folder.path.clone());
            }
        } else {
            let added = self
                .workspace
                .read(cx)
                .model
                .projects()
                .any(|project| same_path(&project.workspace_root, &path));
            let name = std::path::Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
            folders.insert(
                0,
                RecentFolder {
                    path: path.clone(),
                    name,
                    providers: Vec::new(),
                    sessions: 0,
                    active_at: Utc::now().to_rfc3339(),
                    added,
                },
            );
            if !added {
                self.selected.insert(path);
            }
        }
        cx.notify();
    }

    /// Adds the picked folders as projects, then closes the setup. A folder that can't be
    /// added keeps it open with the reason.
    pub(super) fn start(&mut self, then: Option<AfterExit>, cx: &mut Context<Self>) {
        if self.adding {
            return;
        }
        let paths: Vec<String> = self
            .to_add()
            .into_iter()
            .map(|folder| folder.path.clone())
            .collect();
        if paths.is_empty() {
            self.exit(then, cx);
            return;
        }
        self.adding = true;
        self.error = None;
        cx.notify();
        let workspace = self.workspace.read(cx);
        let engine = workspace.engine.clone();
        let projects: Vec<_> = workspace
            .model
            .projects()
            .map(|project| (project.id.clone(), project.workspace_root.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let added = cx
                .background_spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            let added = add_at(&engine, &path, &projects);
                            (path, added)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                this.adding = false;
                let mut first = None;
                let mut failure = None;
                for (path, added) in added {
                    match added {
                        Ok(project_id) => {
                            first.get_or_insert(project_id);
                            this.selected.remove(&path);
                            if let Some(folder) = this
                                .folders
                                .iter_mut()
                                .flatten()
                                .find(|folder| folder.path == path)
                            {
                                folder.added = true;
                            }
                        }
                        Err(error) => {
                            failure.get_or_insert(error);
                        }
                    }
                }
                if let Some(project_id) = first {
                    cx.emit(OnboardingEvent::ProjectAdded(project_id));
                }
                match failure {
                    Some(error) => this.error = Some(error_message(&error)),
                    None => this.exit(then, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn projects_band(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let muted = colors.muted_foreground;
        let summary = match &self.folders {
            None => div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(40.))
                .child(row_icon(spinner(px(14.), muted)))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(m::onboarding_folders_looking()),
                )
                .into_any_element(),
            Some(folders) if folders.is_empty() => div()
                .debug_selector(|| "onboarding-folders-empty".into())
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(40.))
                .child(row_icon(
                    Icon::new(IconName::Folder).size_4().text_color(muted),
                ))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(m::onboarding_folders_empty()),
                )
                .into_any_element(),
            Some(folders) => self.folders_summary(folders, &colors, cx),
        };
        let link = |id: &'static str, icon: IconName, label: String| {
            let hover = colors.foreground;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .items_center()
                .gap_1p5()
                .text_xs()
                .text_color(muted)
                .when(!self.adding, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.text_color(hover))
                })
                .child(Icon::new(icon).size_3p5())
                .child(label)
        };
        let links = div()
            .flex()
            .items_center()
            .gap_4()
            .pl(px(32.))
            .min_h(px(30.))
            .child(
                link(
                    "onboarding-open-folder",
                    IconName::FolderOpen,
                    m::onboarding_open_folder(),
                )
                .when(!self.adding, |this| {
                    this.on_click(cx.listener(|this, _, window, cx| this.open_folder(window, cx)))
                }),
            )
            .child(
                link(
                    "onboarding-add-or-clone",
                    IconName::FolderPlus,
                    m::onboarding_add_or_clone(),
                )
                .when(!self.adding, |this| {
                    this.on_click(
                        cx.listener(|this, _, _, cx| this.start(Some(AfterExit::AddProject), cx)),
                    )
                }),
            );
        let content = div()
            .child(summary)
            .when(self.reviewing, |this| {
                this.children(
                    self.folders
                        .as_deref()
                        .map(|folders| self.review(folders, &colors, cx)),
                )
            })
            .child(links)
            .children(self.error.clone().map(|error| {
                div()
                    .pl(px(32.))
                    .pb_1()
                    .text_xs()
                    .text_color(colors.destructive)
                    .child(error)
            }));
        band(m::onboarding_band_projects(), content, &colors)
    }

    fn folders_summary(
        &self,
        folders: &[RecentFolder],
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = self.to_add().len();
        let reviewing = self.reviewing;
        div()
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(40.))
            .py_1()
            .child(
                div()
                    .id("onboarding-folders-check")
                    .debug_selector(|| "onboarding-folders-check".into())
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .when(!self.adding, |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_all(cx)))
                    })
                    .child(row_icon(checkbox(self.check_state(), colors)))
                    .child(
                        div()
                            .min_w_0()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                                if count > 0 {
                                    m::onboarding_folders_add(count)
                                } else {
                                    m::onboarding_folders_add_none()
                                },
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(m::onboarding_folders_source()),
                            ),
                    ),
            )
            .child(
                Button::new("onboarding-folders-review")
                    .label(if reviewing {
                        m::onboarding_folders_hide()
                    } else {
                        m::onboarding_folders_review(folders.len())
                    })
                    .icon(if reviewing {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Xs)
                    .text_color(colors.muted_foreground)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reviewing = !this.reviewing;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn review(&self, folders: &[RecentFolder], colors: &Colors, cx: &mut Context<Self>) -> Div {
        let picked = self.to_add().len();
        let new = folders.iter().filter(|folder| !folder.added).count();
        let now = Utc::now();
        let rows: Vec<_> = folders
            .iter()
            .map(|folder| {
                (
                    folder.clone(),
                    self.selected.contains(&folder.path),
                    relative_time(&folder.active_at, now),
                )
            })
            .collect();
        let count = rows.len();
        let this = cx.weak_entity();
        let adding = self.adding;
        let row_colors = colors.clone();
        let list = uniform_list("onboarding-folders", count, move |range, _, _| {
            range
                .map(|index| {
                    let (folder, selected, age) = rows[index].clone();
                    folder_row(
                        index,
                        folder,
                        selected,
                        age,
                        adding,
                        &row_colors,
                        this.clone(),
                    )
                })
                .collect()
        })
        .w_full()
        .h(px(ROW_HEIGHT * count.min(SHOWN_ROWS) as f32));
        let action = |id: &'static str, label: String, enabled: bool, all: bool| {
            Button::new(id)
                .label(label)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .disabled(!enabled || adding)
                .on_click(cx.listener(move |this, _, _, cx| this.pick_all(all, cx)))
        };
        div()
            .debug_selector(|| "onboarding-folders-review".into())
            .pl(px(26.))
            .pb_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .pl_1p5()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(m::onboarding_folders_selected(new, picked))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(action(
                                "onboarding-folders-all",
                                m::onboarding_folders_select_all(),
                                picked < new,
                                true,
                            ))
                            .child(action(
                                "onboarding-folders-none",
                                m::onboarding_folders_select_none(),
                                picked > 0,
                                false,
                            )),
                    ),
            )
            .child(list)
    }
}

fn folder_row(
    index: usize,
    folder: RecentFolder,
    selected: bool,
    age: String,
    adding: bool,
    colors: &Colors,
    view: WeakEntity<Onboarding>,
) -> Stateful<Div> {
    let hover = colors.muted.opacity(0.5);
    let logos = folder.providers.iter().filter_map(|id| {
        let provider = crate::providers::known(id)?;
        Some(
            svg()
                .path(provider.mark)
                .size(px(12.))
                .text_color((provider.tint)(colors)),
        )
    });
    let path = folder.path.clone();
    div()
        .id(("onboarding-folder", index))
        .debug_selector(move || format!("onboarding-folder-{index}"))
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .gap_2p5()
        .px_1p5()
        .rounded_md()
        .when(folder.added, |this| this.opacity(0.55))
        .when(!folder.added && !adding, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| view.toggle(path.clone(), cx))
                        .ok();
                })
        })
        .child(checkbox(
            if selected && !folder.added {
                Check::On
            } else {
                Check::Off
            },
            colors,
        ))
        .child(
            div()
                .flex_none()
                .max_w(px(180.))
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(folder.name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(folder.path.clone()),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .text_xs()
                .text_color(colors.muted_foreground)
                .map(|this| {
                    if folder.added {
                        this.child(m::onboarding_folder_added())
                    } else {
                        this.children(logos).child(age)
                    }
                }),
        )
}

fn checkbox(check: Check, colors: &Colors) -> Div {
    let checkbox = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.))
        .rounded(px(4.))
        .border_1();
    match check {
        Check::Off => checkbox.border_color(colors.border),
        Check::On | Check::Partial => checkbox
            .border_color(colors.primary)
            .bg(colors.primary)
            .text_color(colors.primary_foreground)
            .child(
                Icon::new(if check == Check::On {
                    IconName::Check
                } else {
                    IconName::Minus
                })
                .size(px(10.)),
            ),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_pick, first_pick};
    use chrono::{Duration, Utc};
    use hoshi_engine::RecentFolder;
    use std::collections::HashSet;

    fn folder(path: &str, days_ago: i64, added: bool) -> RecentFolder {
        RecentFolder {
            path: path.to_owned(),
            name: path.to_owned(),
            providers: vec!["claudeCode".to_owned()],
            sessions: 1,
            active_at: (Utc::now() - Duration::days(days_ago)).to_rfc3339(),
            added,
        }
    }

    fn paths(set: HashSet<String>) -> Vec<String> {
        let mut paths: Vec<_> = set.into_iter().collect();
        paths.sort();
        paths
    }

    #[test]
    fn the_new_folders_worked_in_lately_start_picked() {
        let folders = [
            folder("/a", 1, false),
            folder("/b", 2, true),
            folder("/c", 90, false),
        ];
        assert_eq!(paths(first_pick(&folders, Utc::now())), ["/a"]);
        assert_eq!(paths(default_pick(&folders, Utc::now())), ["/a"]);
    }

    #[test]
    fn with_nothing_recent_the_box_picks_every_new_folder() {
        let folders = [folder("/a", 60, false), folder("/b", 90, true)];
        assert!(first_pick(&folders, Utc::now()).is_empty());
        assert_eq!(paths(default_pick(&folders, Utc::now())), ["/a"]);
    }
}
