//! The cards that rest on the sidebar's rows: a thread's summary, and a pull request's from its
//! pill. One thread card serves every row, as the web's shared handle does: once it shows, moving
//! to another row moves it there at once.

use super::{Problem, Sidebar, checks_color, pull_request_problem};
use crate::m;
use crate::sections::{self, Logins, relative_time};
use chrono::Utc;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{ChecksState, PullRequest, PullRequestState, ReviewAsk, ReviewTurn, Thread};
use hoshi_ui::prelude::*;
use hoshi_ui::theme::Colors;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// Resting this long on a row or a pill shows its card.
const OPEN_DELAY: Duration = Duration::from_millis(400);
const CLOSE_DELAY: Duration = Duration::from_millis(100);
/// Resting this long on a pill starts reading the pull request's detail.
const PREFETCH_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum CardTarget {
    Thread(String),
    PullRequest(String),
}

impl CardTarget {
    fn thread_id(&self) -> &str {
        match self {
            Self::Thread(id) | Self::PullRequest(id) => id,
        }
    }
}

/// Where the pointer is and which card shows.
#[derive(Default)]
pub(super) struct Cards {
    pub shown: Option<CardTarget>,
    row: Option<String>,
    pill: Option<String>,
    over_card: bool,
    /// A row clicked keeps its card closed until the pointer leaves it.
    quiet: Option<String>,
    timer: Option<Task<()>>,
    prefetch: Option<Task<()>>,
    /// Counts the times a card opened from closed, to fade each one in.
    opened: usize,
    /// Where the card was when it moved to another row, and how many times it moved: it slides
    /// from there.
    pub slide: Option<(Pixels, usize)>,
    /// Where each row and pill was drawn last, by target.
    pub anchors: Rc<RefCell<HashMap<CardTarget, Bounds<Pixels>>>>,
}

impl Cards {
    fn wanted(&self) -> Option<CardTarget> {
        if let Some(id) = &self.pill {
            return Some(CardTarget::PullRequest(id.clone()));
        }
        self.row
            .as_ref()
            .filter(|id| self.quiet.as_ref() != Some(*id))
            .map(|id| CardTarget::Thread(id.clone()))
    }

    pub fn close(&mut self) {
        self.shown = None;
        self.slide = None;
        self.timer = None;
    }
}

/// Measures where `target`'s element is drawn, for its card to rest against.
pub(super) fn anchor(cards: &Cards, target: CardTarget) -> impl IntoElement {
    let anchors = cards.anchors.clone();
    canvas(
        move |bounds, _, _| {
            anchors.borrow_mut().insert(target.clone(), bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Sidebar {
    pub(super) fn hover_row(&mut self, thread_id: &str, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.cards.row = Some(thread_id.to_owned());
        } else if self.cards.row.as_deref() == Some(thread_id) {
            self.cards.row = None;
            if self.cards.quiet.as_deref() == Some(thread_id) {
                self.cards.quiet = None;
            }
        }
        self.settle_cards(cx);
    }

    pub(super) fn hover_pill(&mut self, thread_id: &str, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.cards.pill = Some(thread_id.to_owned());
            let engine = self.workspace.read(cx).engine.clone();
            let id = thread_id.to_owned();
            self.cards.prefetch = Some(cx.spawn(async move |_, cx| {
                cx.background_executor().timer(PREFETCH_DELAY).await;
                cx.background_spawn(async move { engine.prefetch_pull_request(&id) })
                    .await;
            }));
        } else if self.cards.pill.as_deref() == Some(thread_id) {
            self.cards.pill = None;
            self.cards.prefetch = None;
        }
        self.settle_cards(cx);
    }

    fn hover_card(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.cards.over_card = hovered;
        self.settle_cards(cx);
    }

    /// The row clicked: its card closes until the pointer comes back.
    pub(super) fn quiet_card(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.cards.quiet = Some(thread_id.to_owned());
        self.cards.close();
        cx.notify();
    }

    fn settle_cards(&mut self, cx: &mut Context<Self>) {
        let wanted = self.cards.wanted();
        if wanted == self.cards.shown {
            self.cards.timer = None;
            return;
        }
        match wanted {
            None => {
                if self.cards.over_card || self.cards.shown.is_none() {
                    self.cards.timer = None;
                    return;
                }
                self.cards.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(CLOSE_DELAY).await;
                    this.update(cx, |this, cx| {
                        if this.cards.wanted().is_none() && !this.cards.over_card {
                            this.cards.shown = None;
                            this.cards.slide = None;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            Some(target) => {
                let moving = matches!(
                    (&self.cards.shown, &target),
                    (Some(CardTarget::Thread(_)), CardTarget::Thread(_))
                );
                if moving {
                    let from = self
                        .cards
                        .shown
                        .as_ref()
                        .and_then(|shown| self.cards.anchors.borrow().get(shown).copied());
                    if let Some(from) = from {
                        let moves = self.cards.slide.map_or(0, |(_, moves)| moves) + 1;
                        self.cards.slide = Some((from.top(), moves));
                    }
                    self.cards.shown = Some(target);
                    self.cards.timer = None;
                    cx.notify();
                    return;
                }
                self.cards.slide = None;
                if self.cards.shown.take().is_some() {
                    cx.notify();
                }
                self.cards.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(OPEN_DELAY).await;
                    this.update(cx, |this, cx| {
                        if this.cards.wanted() == Some(target.clone()) && this.menu.is_none() {
                            this.cards.shown = Some(target);
                            this.cards.slide = None;
                            this.cards.opened += 1;
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
        }
    }

    /// The card showing, placed beside what it is for.
    pub(super) fn card(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.menu.is_some() {
            return None;
        }
        let target = self.cards.shown.clone()?;
        let bounds = *self.cards.anchors.borrow().get(&target)?;
        let colors = cx.hoshi().clone();
        let (body, width, offset) = match &target {
            CardTarget::Thread(id) => {
                let thread = self.workspace.read(cx).model.thread(id)?.clone();
                (self.thread_summary(&thread, &colors, cx), px(256.), px(12.))
            }
            CardTarget::PullRequest(id) => {
                let workspace = self.workspace.read(cx);
                let pull_request = workspace.pull_requests.get(id)?.clone();
                let logins = workspace.logins.clone();
                (
                    pull_request_summary(&pull_request, &logins, &colors, cx),
                    px(320.),
                    px(15.),
                )
            }
        };
        let opened = self.cards.opened;
        let card = div()
            .id(SharedString::from(format!("card-{}", target.thread_id())))
            .debug_selector(|| "sidebar-card".into())
            .w(width)
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(colors.foreground.opacity(0.1))
            .bg(colors.popover)
            .text_color(colors.popover_foreground)
            .text_sm()
            .shadow_md()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.hover_card(*hovered, cx)))
            .child(body)
            .with_animation(
                ElementId::NamedInteger("card-fade".into(), opened as u64),
                Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                |card, delta| card.opacity(delta),
            );
        let (x, top) = (bounds.right() + offset, bounds.top());
        let placed = anchored()
            .position(point(x, top))
            .snap_to_window_with_margin(px(8.))
            .child(card);
        let placed = match self.cards.slide {
            Some((from, moves)) if !cx.reduce_motion() => placed
                .with_animation(
                    ElementId::NamedInteger("card-slide".into(), moves as u64),
                    Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                    move |placed, delta| placed.position(point(x, from + (top - from) * delta)),
                )
                .into_any_element(),
            _ => placed.into_any_element(),
        };
        Some(
            deferred(placed)
                .with_priority(POPOVER_PRIORITY)
                .into_any_element(),
        )
    }

    /// What a thread is at a glance: its whole title, where it works, and with which model.
    fn thread_summary(&mut self, thread: &Thread, colors: &Colors, cx: &mut Context<Self>) -> Div {
        let workspace = self.workspace.read(cx);
        let project = thread
            .project_id
            .as_deref()
            .and_then(|id| workspace.model.project(id))
            .cloned();
        let model = workspace
            .chosen_model(&thread.provider, thread.model.as_deref())
            .map(|model| model.name.clone())
            .or_else(|| thread.model.clone());
        let mono = cx.theme().mono_font_family.clone();
        let muted = colors.muted_foreground;
        let fact = |icon: AnyElement, text: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(div().flex_none().child(icon))
                .child(div().min_w_0().truncate().child(text))
        };
        let lucide = |name: IconName| {
            Icon::new(name)
                .size(px(14.))
                .text_color(muted)
                .into_any_element()
        };
        let place = match &project {
            None if thread.project_id.is_none() => Some(fact(
                lucide(IconName::MessageCircle),
                m::chat().into_any_element(),
            )),
            None => None,
            Some(project) => {
                let icon = self.icon(&project.id, cx);
                Some(fact(
                    match icon {
                        Some(path) => img(path)
                            .size(px(14.))
                            .rounded(px(3.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => lucide(IconName::Folder),
                    },
                    project.title.clone().into_any_element(),
                ))
            }
        };
        let workspace_fact = thread.project_id.is_some().then(|| {
            let (icon, text) = match (&thread.worktree_path, thread.worktree_released) {
                (Some(_), true) => (IconName::GitFork, m::summary_worktree_released()),
                (Some(_), false) => (IconName::GitFork, m::summary_worktree()),
                (None, _) => (IconName::Folder, m::workspace_local()),
            };
            fact(lucide(icon), text.into_any_element())
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                if thread.title.is_empty() {
                    m::untitled_thread()
                } else {
                    thread.title.clone()
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .text_xs()
                    .children(place)
                    .children(workspace_fact)
                    .when_some(thread.branch.clone(), |this, branch| {
                        this.child(fact(
                            lucide(IconName::GitBranch),
                            div()
                                .font_family(mono)
                                .text_size(rems(0.6875))
                                .child(branch)
                                .into_any_element(),
                        ))
                    })
                    .when_some(model, |this, model| {
                        this.child(fact(
                            svg()
                                .path(crate::composer::provider_mark(&thread.provider))
                                .size(px(14.))
                                .text_color(crate::composer::provider_tint(
                                    &thread.provider,
                                    colors,
                                ))
                                .into_any_element(),
                            model.into_any_element(),
                        ))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(m::thread_updated(relative_time(
                        &thread.updated_at,
                        Utc::now(),
                    ))),
            )
    }
}

/// `owner/name` from a pull request's web address, or nothing if it has another shape.
fn repository_of(url: &str) -> Option<String> {
    let path = url.split_once("://")?.1.split_once('/')?.1;
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    Some(format!("{}/{}", parts.next()?, parts.next()?))
}

/// Whose move an open pull request waits on, as its author sees it.
#[derive(Debug, PartialEq)]
enum Turn {
    Author {
        /// What was asked of the author, when the forge told who and when.
        ask: Option<(String, ReviewAsk, String)>,
        threads: u32,
        tasks: u32,
    },
    Reviewers {
        waiting_on: Vec<String>,
        replied_at: String,
        tasks: u32,
    },
}

fn pull_request_turn(pull_request: &PullRequest) -> Option<Turn> {
    if !matches!(
        pull_request.state,
        PullRequestState::Open | PullRequestState::Draft
    ) {
        return None;
    }
    let ask = match &pull_request.turn {
        Some(ReviewTurn::Author { by, ask, at }) => Some((by.clone(), *ask, at.clone())),
        _ => None,
    };
    let unanswered = match &pull_request.turn {
        None => pull_request.changes_requested,
        Some(_) => ask.is_some(),
    };
    if unanswered || pull_request.unresolved_comments > 0 {
        return Some(Turn::Author {
            ask,
            threads: pull_request.unresolved_comments,
            tasks: pull_request.open_tasks,
        });
    }
    match &pull_request.turn {
        Some(ReviewTurn::Reviewers {
            waiting_on,
            replied_at,
        }) => Some(Turn::Reviewers {
            waiting_on: waiting_on.clone(),
            replied_at: replied_at.clone(),
            tasks: pull_request.open_tasks,
        }),
        _ => None,
    }
}

pub(crate) fn checks_label(checks: ChecksState) -> String {
    match checks {
        ChecksState::Passing => m::checks_passing(),
        ChecksState::Failing => m::checks_failing(),
        ChecksState::Pending => m::checks_pending(),
    }
}

pub(crate) fn pull_request_summary(
    pull_request: &PullRequest,
    logins: &Logins,
    colors: &Colors,
    cx: &App,
) -> Div {
    let mono = cx.theme().mono_font_family.clone();
    let muted = colors.muted_foreground;
    let problem = pull_request_problem(pull_request);
    let (icon, state_color, state_label) = match pull_request.state {
        PullRequestState::Open => (IconName::GitPullRequest, colors.pr_open, m::pr_state_open()),
        PullRequestState::Draft => (
            IconName::GitPullRequestDraft,
            colors.muted_foreground,
            m::pr_state_draft(),
        ),
        PullRequestState::Merged => (IconName::GitMerge, colors.pr_merged, m::pr_state_merged()),
        PullRequestState::Closed => (
            IconName::GitPullRequestClosed,
            colors.status_error,
            m::pr_state_closed(),
        ),
    };
    let (color, label) = match problem {
        Some(Problem::Conflicts) => (colors.status_error, m::pr_problem_conflicts()),
        Some(Problem::ChecksFailing) => (colors.status_error, m::checks_failing()),
        Some(Problem::ChangesRequested) => (colors.warning, m::pr_problem_changes_requested()),
        None => (state_color, state_label),
    };
    let checks = pull_request
        .checks
        .map(|checks| (checks_label(checks), checks_color(checks, colors)));
    let others = sections::is_others(pull_request, logins);
    let turn = (!others).then(|| pull_request_turn(pull_request)).flatten();
    let updated = m::pr_updated(relative_time(&pull_request.updated_at, Utc::now()));
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .text_xs()
                .text_color(muted)
                .children(
                    repository_of(&pull_request.url)
                        .map(|repository| div().truncate().child(repository)),
                )
                .child(div().flex_none().child(format!("#{}", pull_request.number)))
                .child("·")
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(icon).size_3().text_color(color))
                        .child(label),
                ),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(pull_request.title.clone()),
        )
        .child(
            div()
                .flex()
                .min_w_0()
                .text_xs()
                .text_color(muted)
                .when_some(pull_request.author.clone(), |this, author| {
                    this.child(
                        div()
                            .flex_none()
                            .text_color(colors.foreground)
                            .child(author),
                    )
                    .child(div().flex_none().child("\u{a0}·\u{a0}"))
                })
                .child(div().truncate().child(updated)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_3()
                .gap_y_1()
                .text_xs()
                .text_color(muted)
                .child(div().min_w_0().truncate().font_family(mono).child(format!(
                    "{} ← {}",
                    pull_request.base_branch, pull_request.head_branch
                )))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            div()
                                .text_color(colors.pr_open)
                                .child(format!("+{}", pull_request.additions)),
                        )
                        .child(
                            div()
                                .text_color(colors.status_error)
                                .child(format!("−{}", pull_request.deletions)),
                        ),
                )
                .when_some(checks, |this, (label, color)| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(color)
                            .child(div().size(px(6.)).rounded_full().bg(color))
                            .child(label),
                    )
                })
                .when(others && pull_request.unresolved_comments > 0, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(IconName::MessageSquare).size_3())
                            .child(m::unresolved_comments(pull_request.unresolved_comments)),
                    )
                }),
        )
        .when_some(turn, |this, turn| this.child(turn_summary(turn, colors)))
}

/// Whose move the pull request waits on, and why.
fn turn_summary(turn: Turn, colors: &Colors) -> Div {
    let muted = colors.muted_foreground;
    let reason = |icon: IconName, icon_color: Hsla, text: String| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .pl_3()
            .text_color(muted)
            .child(Icon::new(icon).size_3().flex_none().text_color(icon_color))
            .child(div().truncate().child(text))
    };
    let relative = |at: &str| relative_time(at, Utc::now());
    let yours = matches!(turn, Turn::Author { .. });
    let headline = match &turn {
        Turn::Author { .. } => m::pr_turn_yours(),
        Turn::Reviewers { waiting_on, .. } => m::pr_turn_waiting_on(waiting_on.join(", ")),
    };
    let mut reasons = Vec::new();
    let tasks = match turn {
        Turn::Reviewers {
            replied_at, tasks, ..
        } => {
            reasons.push(reason(
                IconName::MessageSquare,
                muted,
                m::pr_turn_replied(relative(&replied_at)),
            ));
            tasks
        }
        Turn::Author {
            ask,
            threads,
            tasks,
        } => {
            match ask {
                Some((by, ReviewAsk::ChangesRequested, at)) => reasons.push(reason(
                    IconName::CircleX,
                    colors.warning,
                    m::pr_turn_changes_requested(&by, relative(&at)),
                )),
                Some((by, ReviewAsk::Comment, at)) => reasons.push(reason(
                    IconName::MessageSquare,
                    muted,
                    m::pr_turn_commented(&by, relative(&at)),
                )),
                None => {}
            }
            if threads > 0 {
                reasons.push(reason(
                    IconName::MessageSquare,
                    muted,
                    m::pr_turn_threads(threads as usize),
                ));
            }
            tasks
        }
    };
    if tasks > 0 {
        reasons.push(reason(
            IconName::ListChecks,
            muted,
            m::pr_turn_tasks(tasks as usize),
        ));
    }
    div()
        .mt(px(2.))
        .pt_2()
        .border_t_1()
        .border_color(colors.border)
        .flex()
        .flex_col()
        .gap_1()
        .text_xs()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .font_weight(FontWeight::MEDIUM)
                .child(div().size(px(6.)).flex_none().rounded_full().bg(if yours {
                    colors.warning
                } else {
                    muted
                }))
                .child(headline),
        )
        .children(reasons)
}

#[cfg(test)]
mod tests {
    use super::{Turn, pull_request_turn, repository_of};
    use hoshi_core::{PullRequest, ReviewAsk};

    fn pull_request(extra: serde_json::Value) -> PullRequest {
        let mut value = serde_json::json!({
            "forge": "github",
            "number": 7,
            "title": "Rail",
            "url": "https://github.com/nxtvoid/hoshi/pull/7",
            "headBranch": "feature",
            "baseBranch": "main",
            "state": "open",
            "unresolvedComments": 0,
            "openTasks": 0,
            "changesRequested": false,
            "conflicts": false,
            "additions": 0,
            "deletions": 0,
            "updatedAt": "2026-11-13T10:00:00Z"
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_repository_comes_from_the_address() {
        assert_eq!(
            repository_of("https://github.com/nxtvoid/hoshi/pull/7").as_deref(),
            Some("nxtvoid/hoshi")
        );
        assert_eq!(repository_of("not a url"), None);
    }

    #[test]
    fn the_turn_follows_who_spoke_last() {
        assert_eq!(
            pull_request_turn(&pull_request(serde_json::json!({}))),
            None
        );
        let asked = pull_request(serde_json::json!({
            "turn": {"kind": "author", "by": "ana", "ask": "changesRequested", "at": "2026-11-13T09:00:00Z"},
            "openTasks": 2
        }));
        assert_eq!(
            pull_request_turn(&asked),
            Some(Turn::Author {
                ask: Some((
                    "ana".into(),
                    ReviewAsk::ChangesRequested,
                    "2026-11-13T09:00:00Z".into()
                )),
                threads: 0,
                tasks: 2,
            })
        );
        let replied = pull_request(serde_json::json!({
            "turn": {"kind": "reviewers", "waitingOn": ["ana"], "repliedAt": "2026-11-13T09:00:00Z"}
        }));
        assert!(matches!(
            pull_request_turn(&replied),
            Some(Turn::Reviewers { .. })
        ));
        let merged = pull_request(serde_json::json!({"state": "merged", "unresolvedComments": 3}));
        assert_eq!(pull_request_turn(&merged), None);
    }
}
