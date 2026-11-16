use super::actions::{matching_labels, reviewer_choices};
use super::{PullRequestSurface, label_color};
use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use hoshi_core::{PullRequest, PullRequestAction, PullRequestDetail, PullRequestLabel};
use hoshi_ui::prelude::*;
use std::time::Duration;

const SEARCH_DELAY: Duration = Duration::from_millis(250);

/// Searches the accounts that can review; the chosen one is asked.
pub struct ReviewerPicker {
    pub open: bool,
    query: Entity<InputState>,
    candidates: Option<Vec<String>>,
    /// The query `candidates` answered; another one is being searched while they differ.
    answered: Option<String>,
    search: Option<Task<()>>,
    _changes: Subscription,
}

/// Every label of the repository, toggled on the pull request with a click.
pub struct LabelPicker {
    pub open: bool,
    query: Entity<InputState>,
    labels: Option<Vec<PullRequestLabel>>,
    load: Option<Task<()>>,
    _changes: Subscription,
}

impl ReviewerPicker {
    pub fn new(window: &mut Window, cx: &mut Context<PullRequestSurface>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(m::search_people()));
        let changes = cx.subscribe(&query, |this: &mut PullRequestSurface, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_reviewers(cx);
            }
        });
        Self {
            open: false,
            query,
            candidates: None,
            answered: None,
            search: None,
            _changes: changes,
        }
    }
}

impl LabelPicker {
    pub fn new(window: &mut Window, cx: &mut Context<PullRequestSurface>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(m::filter_labels()));
        let changes = cx.subscribe(&query, |_: &mut PullRequestSurface, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            open: false,
            query,
            labels: None,
            load: None,
            _changes: changes,
        }
    }
}

/// What a picker's list says when it has nothing to show.
fn list_note(loading: bool, text: String, colors: &Colors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .text_xs()
        .text_color(colors.muted_foreground)
        .when(loading, |note| {
            note.child(spinner(px(14.), colors.muted_foreground))
        })
        .child(text)
}

fn edit_button(
    id: &'static str,
    icon: IconName,
    label: String,
    open: bool,
    colors: &Colors,
) -> impl IntoElement + use<> {
    let (accent, foreground) = (colors.accent, colors.foreground);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .size_6()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .text_color(colors.muted_foreground)
        .when(open, |button| button.bg(accent).text_color(foreground))
        .hover(move |button| button.bg(accent).text_color(foreground))
        .child(Icon::new(icon).size_3p5())
        .with_tooltip(label)
}

fn picker_frame(query: &Entity<InputState>, list: Vec<AnyElement>, colors: &Colors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_1p5()
        .child(
            div()
                .rounded_md()
                .border_1()
                .border_color(colors.input)
                .px_2()
                .text_xs()
                .child(Input::new(query).appearance(false).small()),
        )
        .child(
            div()
                .id("picker-list")
                .flex()
                .flex_col()
                .max_h(px(240.))
                .overflow_y_scroll()
                .children(list),
        )
}

impl PullRequestSurface {
    pub(super) fn search_reviewers(&mut self, cx: &mut Context<Self>) {
        if !self.reviewers.open {
            return;
        }
        let query = self.reviewers.query.read(cx).value().to_string();
        let (forge, thread_id) = (self.forge.clone(), self.thread_id.clone());
        self.reviewers.search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let asked = query.clone();
            let found = cx
                .background_spawn(async move {
                    forge
                        .reviewer_candidates(&thread_id, &asked)
                        .unwrap_or_default()
                })
                .await;
            this.update(cx, |this, cx| {
                this.reviewers.candidates = Some(found);
                this.reviewers.answered = Some(query);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_reviewers_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.reviewers.open = open;
        if open {
            self.search_reviewers(cx);
        } else {
            self.reviewers.search = None;
        }
        cx.notify();
    }

    fn set_labels_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.labels.open = open;
        if open && self.labels.labels.is_none() && self.labels.load.is_none() {
            let (forge, thread_id) = (self.forge.clone(), self.thread_id.clone());
            self.labels.load = Some(cx.spawn(async move |this, cx| {
                let labels = cx
                    .background_spawn(async move { forge.labels(&thread_id).unwrap_or_default() })
                    .await;
                this.update(cx, |this, cx| {
                    this.labels.labels = Some(labels);
                    this.labels.load = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn request_reviewer(&mut self, login: String, window: &mut Window, cx: &mut Context<Self>) {
        let action = PullRequestAction::RequestReviewers { names: vec![login] };
        self.run(action, window, cx, |this, _, cx| {
            this.set_reviewers_open(false, cx)
        });
    }

    fn toggle_label(
        &mut self,
        name: String,
        applied: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = if applied {
            PullRequestAction::RemoveLabel { name }
        } else {
            PullRequestAction::AddLabels { names: vec![name] }
        };
        self.run(action, window, cx, |_, _, _| {});
    }

    pub(super) fn reviewer_picker(
        &self,
        pull_request: &PullRequest,
        detail: &PullRequestDetail,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let picker = &self.reviewers;
        let query = picker.query.read(cx).value().to_string();
        let searching = picker.answered.as_deref() != Some(query.as_str());
        let current: Vec<&str> = detail
            .reviewers
            .iter()
            .map(|reviewer| reviewer.name.as_str())
            .collect();
        let shown = picker
            .candidates
            .as_deref()
            .map(|candidates| {
                reviewer_choices(candidates, pull_request.author.as_deref(), &current)
            })
            .unwrap_or_default();
        let note = if !shown.is_empty() {
            None
        } else if searching {
            Some((true, m::searching_people()))
        } else if query.trim().is_empty() {
            Some((false, m::no_reviewers_available()))
        } else {
            Some((false, m::no_matches()))
        };
        let input = picker.query.clone();
        let focus = input.focus_handle(cx);
        let this = cx.weak_entity();
        let pending = self.pending;
        let trigger_colors = colors.clone();
        Popover::new("reviewer-picker")
            .align(Align::Start)
            .w(px(256.))
            .modal(true)
            .open(picker.open)
            .initial_focus(PopoverFocus::Handle(focus))
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.set_reviewers_open(change.open, cx)
            }))
            .trigger(move |open| {
                edit_button(
                    "request-review",
                    IconName::UserPlus,
                    m::request_review(),
                    open,
                    &trigger_colors,
                )
            })
            .content(move |_, _, cx| {
                let colors = cx.hoshi().clone();
                let mut list: Vec<AnyElement> = note
                    .map(|(loading, text)| list_note(loading, text, &colors).into_any_element())
                    .into_iter()
                    .collect();
                list.extend(shown.into_iter().enumerate().map(|(index, login)| {
                    let this = this.clone();
                    let chosen = login.clone();
                    div()
                        .id(("reviewer-candidate", index))
                        .debug_selector(move || format!("reviewer-{index}"))
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .text_xs()
                        .when(!pending, |row| {
                            row.cursor_pointer()
                                .hover(|row| row.bg(colors.accent))
                                .on_click(move |_, window, cx| {
                                    let chosen = chosen.clone();
                                    this.update(cx, |this, cx| {
                                        this.request_reviewer(chosen, window, cx)
                                    })
                                    .ok();
                                })
                        })
                        .when(pending, |row| row.opacity(0.5))
                        .child(login)
                        .into_any_element()
                }));
                picker_frame(&input, list, &colors)
            })
    }

    pub(super) fn label_picker(
        &self,
        detail: &PullRequestDetail,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let picker = &self.labels;
        let query = picker.query.read(cx).value().to_string();
        let current: Vec<String> = detail
            .labels
            .iter()
            .map(|label| label.name.clone())
            .collect();
        let shown: Vec<PullRequestLabel> = picker
            .labels
            .as_deref()
            .map(|labels| {
                matching_labels(labels, &query)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let note = match &picker.labels {
            None => Some((true, m::loading_labels())),
            Some(labels) if labels.is_empty() => Some((false, m::no_labels_available())),
            Some(_) if shown.is_empty() => Some((false, m::no_matches())),
            Some(_) => None,
        };
        let input = picker.query.clone();
        let focus = input.focus_handle(cx);
        let this = cx.weak_entity();
        let pending = self.pending;
        let trigger_colors = colors.clone();
        Popover::new("label-picker")
            .align(Align::Start)
            .w(px(256.))
            .modal(true)
            .open(picker.open)
            .initial_focus(PopoverFocus::Handle(focus))
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.set_labels_open(change.open, cx)
            }))
            .trigger(move |open| {
                edit_button(
                    "edit-labels",
                    IconName::Tag,
                    m::edit_labels(),
                    open,
                    &trigger_colors,
                )
            })
            .content(move |_, _, cx| {
                let colors = cx.hoshi().clone();
                let mut list: Vec<AnyElement> = note
                    .map(|(loading, text)| list_note(loading, text, &colors).into_any_element())
                    .into_iter()
                    .collect();
                list.extend(shown.into_iter().enumerate().map(|(index, label)| {
                    let this = this.clone();
                    let applied = current.contains(&label.name);
                    let name = label.name.clone();
                    div()
                        .id(("repository-label", index))
                        .debug_selector(move || format!("label-{index}"))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .text_xs()
                        .when(!pending, |row| {
                            row.cursor_pointer()
                                .hover(|row| row.bg(colors.accent))
                                .on_click(move |_, window, cx| {
                                    let name = name.clone();
                                    this.update(cx, |this, cx| {
                                        this.toggle_label(name, applied, window, cx)
                                    })
                                    .ok();
                                })
                        })
                        .when(pending, |row| row.opacity(0.5))
                        .child(
                            div()
                                .flex_none()
                                .size_2p5()
                                .rounded_full()
                                .when_some(label_color(&label.color), |dot, color| dot.bg(color)),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(label.name))
                        .child(
                            Icon::new(IconName::Check)
                                .size_3p5()
                                .when(!applied, |icon| icon.text_color(transparent_black())),
                        )
                        .into_any_element()
                }));
                picker_frame(&input, list, &colors)
            })
    }
}
