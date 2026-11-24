//! The palette's shape, shared by every mode: the search field, grouped results driven from it,
//! and the key hints. Modes describe it as data; [`Palette`](super::Palette) draws it.

use super::search::Action;
use hoshi_core::{ForgeKind, PullRequest, PullRequestState};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;

use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img, px,
    rems, svg,
};
use std::path::PathBuf;

/// What picking an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Thread(String),
    Project(String),
    Action(Action),
    /// A new thread in the project, or a chat with `None`.
    StartIn(Option<String>),
    Source(Source),
    /// Browses to this query.
    Browse(String),
    PullRequest(Box<PullRequest>),
}

/// A way to add a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    New,
    Folder,
    Clone(CloneSource),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneSource {
    Url,
    Forge(ForgeKind),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Icon(IconName),
    Forge(ForgeKind),
    /// The project's own icon, a folder until it is found.
    Project(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Thread {
        title: String,
        provider: String,
        archived: bool,
        project: Option<String>,
        branch: Option<String>,
        worktree: bool,
        pull_request: Option<(PullRequestState, Option<Problem>, u32)>,
    },
    Project {
        id: String,
        title: String,
        root: String,
    },
    Plain {
        mark: Mark,
        label: String,
    },
    TwoLine {
        mark: Mark,
        title: String,
        description: String,
        disabled: bool,
        /// Shows a "Setup required" button that opens the forge settings.
        setup: bool,
    },
    /// `None` is the parent folder.
    Folder(Option<String>),
    PullRequest {
        state: PullRequestState,
        problem: Option<Problem>,
        number: u32,
        title: String,
        branch: String,
        existing: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    ChangesRequested,
    Other,
}

pub struct Entry {
    pub key: String,
    pub target: Target,
    pub disabled: bool,
    pub row: Row,
}

impl Entry {
    pub fn new(key: impl Into<String>, target: Target, row: Row) -> Self {
        Self {
            key: key.into(),
            target,
            disabled: false,
            row,
        }
    }
}

/// An empty label shows no heading; `divider` draws a line under the group.
pub struct Group {
    pub label: String,
    pub entries: Vec<Entry>,
    pub divider: bool,
}

impl Group {
    pub fn new(label: impl Into<String>, entries: Vec<Entry>) -> Self {
        Self {
            label: label.into(),
            entries,
            divider: false,
        }
    }
}

/// What a step is about to act on, above the results.
pub struct Summary {
    pub mark: Mark,
    pub title: String,
    pub subtitle: Option<String>,
}

/// The button at the end of the field.
pub struct Submit {
    pub label: String,
    pub pending: bool,
    pub disabled: bool,
}

/// One screen of the palette.
pub struct Frame {
    pub groups: Vec<Group>,
    /// Shown when no group has entries.
    pub empty: Option<String>,
    pub status: Option<String>,
    pub summary: Option<Summary>,
    pub back: bool,
    pub enter_label: String,
    pub submit: Option<Submit>,
    /// Enter with no entry highlighted, or with the platform's modifier held, submits.
    pub submits: bool,
    /// Tab completes or picks.
    pub tab: bool,
    /// The link at the end of the key hints: the system's folder picker.
    pub footer: Option<String>,
}

impl Frame {
    pub fn new(groups: Vec<Group>, empty: Option<String>) -> Self {
        Self {
            groups,
            empty,
            status: None,
            summary: None,
            back: false,
            enter_label: crate::m::palette_open(),
            submit: None,
            submits: false,
            tab: false,
            footer: None,
        }
    }

    pub fn len(&self) -> usize {
        self.groups.iter().map(|group| group.entries.len()).sum()
    }

    /// The list's child that draws entry `index`: headings and dividers are children too.
    pub fn child_index(&self, index: usize) -> usize {
        let mut child = 0;
        let mut seen = 0;
        for group in &self.groups {
            if !group.label.is_empty() {
                child += 1;
            }
            if index < seen + group.entries.len() {
                return child + index - seen;
            }
            seen += group.entries.len();
            child += group.entries.len() + usize::from(group.divider);
        }
        child
    }
}

pub fn modifier_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

pub fn mark(mark: &Mark, icon: Option<PathBuf>, colors: &Colors) -> AnyElement {
    let muted = colors.muted_foreground;
    match mark {
        Mark::Icon(name) => Icon::new(*name)
            .size_4()
            .flex_none()
            .text_color(muted)
            .into_any_element(),
        Mark::Forge(ForgeKind::Github) => Icon::new(IconName::Github)
            .size_4()
            .flex_none()
            .text_color(colors.foreground)
            .into_any_element(),
        Mark::Forge(ForgeKind::Bitbucket) => svg()
            .path(brand::BITBUCKET)
            .size_4()
            .flex_none()
            .text_color(colors.link)
            .into_any_element(),
        Mark::Project(_) => {
            let folder = Icon::new(IconName::Folder).size_4().text_color(muted);
            match icon {
                Some(path) => img(path)
                    .size_4()
                    .flex_none()
                    .rounded(px(4.))
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        Icon::new(IconName::Folder)
                            .size_4()
                            .text_color(muted)
                            .into_any_element()
                    })
                    .into_any_element(),
                None => div().flex_none().child(folder).into_any_element(),
            }
        }
    }
}

fn pull_request_icon(state: PullRequestState, problem: Option<Problem>, colors: &Colors) -> Icon {
    let (name, color) = match state {
        PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open),
        PullRequestState::Draft => (IconName::GitPullRequestDraft, colors.muted_foreground),
        PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged),
        PullRequestState::Closed => (IconName::GitPullRequestClosed, colors.status_error),
    };
    let color = match problem {
        Some(Problem::ChangesRequested) => colors.warning,
        Some(Problem::Other) => colors.status_error,
        None => color,
    };
    Icon::new(name).text_color(color)
}

fn branch(name: &str, worktree: bool, mono: &SharedString) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            Icon::new(if worktree {
                IconName::GitFork
            } else {
                IconName::GitBranch
            })
            .size_3()
            .flex_none(),
        )
        .child(
            div()
                .max_w(px(160.))
                .truncate()
                .font_family(mono.clone())
                .text_size(rems(0.6875))
                .child(name.to_owned()),
        )
}

/// What an entry shows, next to its mark. `icon` is the project's icon when it has one.
pub fn row(
    row: &Row,
    icon: Option<PathBuf>,
    colors: &Colors,
    mono: &SharedString,
    open_setup: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> Vec<AnyElement> {
    let muted = colors.muted_foreground;
    let trailing = || {
        div()
            .ml_auto()
            .pl_2()
            .flex()
            .flex_shrink(1.)
            .min_w_0()
            .items_center()
            .gap_1p5()
            .text_xs()
            .text_color(muted)
    };
    match row {
        Row::Thread {
            title,
            provider,
            archived,
            project,
            branch: thread_branch,
            worktree,
            pull_request,
        } => vec![
            svg()
                .path(crate::composer::provider_mark(provider))
                .size(px(14.))
                .flex_none()
                .text_color(crate::composer::provider_tint(provider, colors))
                .into_any_element(),
            div()
                .min_w_0()
                .truncate()
                .when(*archived, |this| this.text_color(muted))
                .child(if title.is_empty() {
                    crate::m::untitled_thread()
                } else {
                    title.clone()
                })
                .into_any_element(),
            trailing()
                .when_some(project.clone(), |this, project| {
                    this.child(div().flex_none().child(project))
                })
                .when_some(thread_branch.clone(), |this, name| {
                    this.child(branch(&name, *worktree, mono))
                })
                .when_some(*pull_request, |this, (state, problem, number)| {
                    this.child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(pull_request_icon(state, problem, colors).size_3())
                            .child(format!("#{number}")),
                    )
                })
                .into_any_element(),
        ],
        Row::Project { id, title, root } => vec![
            mark(&Mark::Project(id.clone()), icon, colors),
            div()
                .min_w_0()
                .truncate()
                .child(title.clone())
                .into_any_element(),
            trailing()
                .child(div().min_w_0().truncate().child(root.clone()))
                .into_any_element(),
        ],
        Row::Plain { mark: shown, label } => vec![
            mark(shown, icon, colors),
            div()
                .min_w_0()
                .truncate()
                .child(label.clone())
                .into_any_element(),
        ],
        Row::TwoLine {
            mark: shown,
            title,
            description,
            disabled,
            setup,
        } => vec![
            div()
                .flex_none()
                .when(*disabled, |this| this.opacity(0.6))
                .child(mark(shown, icon, colors))
                .into_any_element(),
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .map(|this| {
                            if *disabled {
                                this.text_color(muted)
                            } else {
                                this.font_weight(FontWeight::MEDIUM)
                            }
                        })
                        .child(title.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(description.clone()),
                )
                .into_any_element(),
        ]
        .into_iter()
        .chain(setup.then(|| {
            div()
                .id("setup-required")
                .debug_selector(|| "palette-setup-required".into())
                .flex_none()
                .cursor_pointer()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .px_1p5()
                .py(px(2.))
                .text_size(rems(0.6875))
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .hover(|this| this.bg(colors.muted).text_color(colors.foreground))
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    open_setup(event, window, cx)
                })
                .child(crate::m::add_project_setup_required())
                .with_tooltip(Tooltip::new(crate::m::add_project_setup_hint()))
                .into_any_element()
        }))
        .collect(),
        Row::Folder(name) => match name {
            Some(name) => vec![
                mark(&Mark::Icon(IconName::Folder), None, colors),
                div()
                    .min_w_0()
                    .truncate()
                    .child(name.clone())
                    .into_any_element(),
            ],
            None => vec![
                mark(&Mark::Icon(IconName::CornerUpLeft), None, colors),
                div()
                    .font_family(mono.clone())
                    .text_xs()
                    .child("..")
                    .into_any_element(),
            ],
        },
        Row::PullRequest {
            state,
            problem,
            number,
            title,
            branch: head,
            existing,
        } => vec![
            pull_request_icon(*state, *problem, colors)
                .size_4()
                .flex_none()
                .into_any_element(),
            div()
                .flex_none()
                .text_color(muted)
                .child(format!("#{number}"))
                .into_any_element(),
            div()
                .min_w_0()
                .truncate()
                .child(title.clone())
                .into_any_element(),
            trailing()
                .map(|this| {
                    if *existing {
                        this.child(div().flex_none().child(crate::m::open_thread()))
                    } else {
                        this.child(branch(head, false, mono))
                    }
                })
                .into_any_element(),
        ],
    }
}

pub fn heading(label: &str, colors: &Colors) -> AnyElement {
    div()
        .px(px(10.))
        .pt_2()
        .pb_1()
        .text_size(rems(0.6875))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.muted_foreground)
        .child(SharedString::from(label.to_uppercase()))
        .into_any_element()
}

pub fn divider(colors: &Colors) -> AnyElement {
    div()
        .mx(px(10.))
        .my_1p5()
        .h_px()
        .bg(colors.border)
        .into_any_element()
}

pub fn hint(keys: &[&str], label: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .children(keys.iter().map(|key| Kbd::new(key.to_string())))
        .child(label)
}

pub fn summary(summary: &Summary, colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .size_8()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(colors.muted)
                .child(mark(&summary.mark, None, colors)),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(summary.title.clone()),
                )
                .when_some(summary.subtitle.clone(), |this, subtitle| {
                    this.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(subtitle),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::{Entry, Frame, Group, Row, Target};
    use crate::palette::search::Action;

    fn entry(key: &str) -> Entry {
        Entry::new(
            key,
            Target::Action(Action::Inbox),
            Row::Folder(Some(key.into())),
        )
    }

    #[test]
    fn headings_and_dividers_count_as_list_children() {
        let mut first = Group::new("", vec![entry("a")]);
        first.divider = true;
        let frame = Frame::new(
            vec![first, Group::new("Projects", vec![entry("b"), entry("c")])],
            None,
        );
        assert_eq!(frame.len(), 3);
        assert_eq!(frame.child_index(0), 0);
        assert_eq!(frame.child_index(1), 3);
        assert_eq!(frame.child_index(2), 4);
    }
}
