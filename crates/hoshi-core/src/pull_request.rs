use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A code host that serves pull requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ForgeKind {
    Github,
    /// Bitbucket Cloud (bitbucket.org).
    Bitbucket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PullRequestState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Merged | Self::Closed)
    }
}

/// The combined result of a pull request's CI checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChecksState {
    Passing,
    Failing,
    Pending,
}

/// A pull request as every forge reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequest {
    pub forge: ForgeKind,
    pub number: u32,
    pub title: String,
    pub url: String,
    /// `None` when the author's account was deleted.
    pub author: Option<String>,
    pub head_branch: String,
    /// The commit its head branch was at when the forge was last asked. `None` in reads saved
    /// before hoshi kept it.
    #[serde(default)]
    pub head_sha: Option<String>,
    pub base_branch: String,
    pub state: PullRequestState,
    /// `None` when the pull request has no checks.
    pub checks: Option<ChecksState>,
    /// Open review threads whose last word isn't the author's.
    pub unresolved_comments: u32,
    /// Tasks still to do, which some repositories add to every pull request; only Bitbucket
    /// has them.
    #[serde(default)]
    pub open_tasks: u32,
    /// A reviewer asked for changes and hasn't approved since.
    #[serde(default)]
    pub changes_requested: bool,
    /// Whose move it waits on; `None` while nobody else has spoken, or it wasn't read.
    #[serde(default)]
    pub turn: Option<ReviewTurn>,
    /// It conflicts with its base branch; `false` while the forge hasn't worked that out.
    #[serde(default)]
    pub conflicts: bool,
    pub additions: u32,
    pub deletions: u32,
    /// RFC 3339.
    pub updated_at: String,
}

/// Whose move a pull request waits on, told by who spoke last. Commits never pass the
/// turn: a merge from the base branch or a partial fix answers nobody.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ReviewTurn {
    /// Someone asked something of the author after the author last answered.
    Author {
        by: String,
        ask: ReviewAsk,
        /// RFC 3339.
        at: String,
    },
    /// The author answered last.
    Reviewers {
        /// Who asked for changes, or else who spoke last before the answer.
        waiting_on: Vec<String>,
        /// RFC 3339.
        replied_at: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewAsk {
    Comment,
    ChangesRequested,
}

/// A comment, reply or request for changes by a person; approvals and bots are left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewAction {
    pub by: String,
    pub ask: ReviewAsk,
    /// RFC 3339.
    pub at: String,
}

/// A reviewer's standing approval, which settles what they asked before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewApproval {
    pub by: String,
    /// RFC 3339.
    pub at: String,
}

impl ReviewTurn {
    /// `changes_requested_by` are the reviewers whose request for changes still stands.
    pub fn of(
        author: Option<&str>,
        actions: &[ReviewAction],
        changes_requested_by: &[String],
        approvals: &[ReviewApproval],
    ) -> Option<Self> {
        let author = author?;
        let dated: Vec<(i64, &ReviewAction)> = actions
            .iter()
            .filter_map(|action| Some((timestamp_millis(&action.at)?, action)))
            .filter(|(at, action)| {
                !approvals.iter().any(|approval| {
                    approval.by.eq_ignore_ascii_case(&action.by)
                        && timestamp_millis(&approval.at).is_some_and(|approved| approved >= *at)
                })
            })
            .collect();
        let latest = |mine: bool| {
            dated
                .iter()
                .filter(|(_, action)| action.by.eq_ignore_ascii_case(author) == mine)
                .max_by_key(|(at, _)| *at)
                .copied()
        };
        let (asked_at, ask) = latest(false)?;
        match latest(true) {
            Some((replied_at, reply)) if replied_at >= asked_at => Some(Self::Reviewers {
                waiting_on: if changes_requested_by.is_empty() {
                    vec![ask.by.clone()]
                } else {
                    changes_requested_by.to_vec()
                },
                replied_at: reply.at.clone(),
            }),
            replied => {
                let replied_at = replied.map(|(at, _)| at);
                let (_, ask) = dated
                    .iter()
                    .filter(|(at, action)| {
                        action.ask == ReviewAsk::ChangesRequested
                            && replied_at.is_none_or(|replied| *at > replied)
                    })
                    .max_by_key(|(at, _)| *at)
                    .copied()
                    .unwrap_or((asked_at, ask));
                Some(Self::Author {
                    by: ask.by.clone(),
                    ask: ask.ask,
                    at: ask.at.clone(),
                })
            }
        }
    }
}

/// Milliseconds since the Unix epoch for an RFC 3339 timestamp, whatever its offset.
fn timestamp_millis(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let rest = text.get(19..)?;
    let fraction_len = rest.strip_prefix('.').map_or(0, |digits| {
        digits.bytes().take_while(u8::is_ascii_digit).count()
    });
    let millis = match fraction_len {
        0 => 0,
        len => {
            let digits: String = rest[1..=len].chars().chain("000".chars()).take(3).collect();
            digits.parse::<i64>().ok()?
        }
    };
    let zone = &rest[if fraction_len > 0 {
        fraction_len + 1
    } else {
        0
    }..];
    let offset_minutes = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = match zone.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hours = zone.get(1..3)?.parse::<i64>().ok()?;
            let minutes = zone.get(4..6)?.parse::<i64>().ok()?;
            sign * (hours * 60 + minutes)
        }
    };
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

/// What an update on a watched pull request told the agent, for the UI to name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestUpdate {
    pub number: u32,
    /// Checks that failed since the last update.
    pub failed_checks: Vec<String>,
    pub checks_passed: bool,
    /// Comments and reviews since the last update.
    pub comments: u32,
    pub conflicting: bool,
    pub closed: bool,
    /// The watch ended with this update.
    pub stopped: bool,
}

/// What an agent watching its pull request was told so far. Kept across restarts, and
/// after the watch ends, so news from while hoshi was closed still wakes the agent and
/// watching again never repeats it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestWatch {
    pub number: u32,
    /// The commit the checks below belong to; `None` until first read.
    pub head_sha: Option<String>,
    pub failed_checks: Vec<String>,
    pub passed: bool,
    pub conflicting: bool,
    /// Comments up to this moment were told. RFC 3339.
    pub remarks_through: String,
    /// The comments at `remarks_through` already told, since forges stamp them to the second.
    pub remark_ids: Vec<String>,
    /// Comment-only wakes in a row.
    pub wakes: u32,
}

/// One CI check of a pull request's latest commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestCheck {
    pub name: String,
    pub state: ChecksState,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewComment {
    pub author: Option<String>,
    pub body: String,
    /// RFC 3339.
    pub created_at: String,
    pub url: String,
}

/// A conversation on a line of the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewThread {
    /// The forge's own id, to reply to or resolve the thread.
    pub id: String,
    pub path: String,
    /// `None` when the line is no longer in the diff.
    pub line: Option<u32>,
    pub resolved: bool,
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewState {
    /// Asked to review, no review yet.
    Requested,
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reviewer {
    /// A user's login or a team's name.
    pub name: String,
    pub state: ReviewState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestLabel {
    pub name: String,
    /// Hex without `#`, as the forge stores it.
    pub color: String,
}

/// What happened on a pull request, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TimelineItem {
    Comment {
        author: Option<String>,
        body: String,
        created_at: String,
        url: String,
    },
    Review {
        author: Option<String>,
        state: ReviewState,
        body: String,
        created_at: String,
        url: String,
    },
    Commit {
        sha: String,
        message: String,
        author: Option<String>,
        created_at: String,
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// Whether the forge would merge the pull request as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    /// Still being computed by the forge, or blocked by rules it does not explain.
    Unknown,
}

/// A change the user asks the forge to make to a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PullRequestAction {
    Merge {
        method: MergeMethod,
    },
    /// Closes or reopens it.
    SetOpen {
        open: bool,
    },
    SetDraft {
        draft: bool,
    },
    Edit {
        title: String,
        body: String,
    },
    RequestReviewers {
        names: Vec<String>,
    },
    RemoveReviewers {
        names: Vec<String>,
    },
    AddLabels {
        names: Vec<String>,
    },
    RemoveLabel {
        name: String,
    },
    Comment {
        body: String,
    },
    ReplyToThread {
        thread_id: String,
        body: String,
    },
    SetThreadResolved {
        thread_id: String,
        resolved: bool,
    },
}

/// What the side panel shows of a pull request, read on demand and not cached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestDetail {
    /// What the forge lets hoshi change; older cached details predate it and are GitHub's.
    #[serde(default = "PullRequestSupport::all")]
    pub supports: PullRequestSupport,
    /// The forge's own id, needed by some actions.
    pub forge_id: String,
    /// `owner/name` or the forge's equivalent.
    pub repository: String,
    pub mergeability: Mergeability,
    /// The merge methods the repository allows.
    pub merge_methods: Vec<MergeMethod>,
    /// The signed-in account may edit it: title, labels, reviewers, state.
    pub can_edit: bool,
    /// Markdown.
    pub body: String,
    pub changed_files: u32,
    pub reviewers: Vec<Reviewer>,
    pub labels: Vec<PullRequestLabel>,
    pub checks: Vec<PullRequestCheck>,
    pub review_threads: Vec<ReviewThread>,
    pub timeline: Vec<TimelineItem>,
}

/// The account a forge is signed in with; `None` when it is not signed in.
/// Parts of a pull request some forges have and others don't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequestSupport {
    pub labels: bool,
    /// Moving it in and out of draft.
    pub draft: bool,
    /// Reopening it once closed.
    pub reopen: bool,
}

impl PullRequestSupport {
    pub fn all() -> Self {
        Self {
            labels: true,
            draft: true,
            reopen: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForgeAccount {
    pub forge: ForgeKind,
    pub login: Option<String>,
}

/// A fresh read of a thread's pull request detail; `None` when the read failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadPullRequestDetail {
    pub thread_id: String,
    pub detail: Option<PullRequestDetail>,
}

/// The pull request of a thread's branch; `None` when the branch has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadPullRequest {
    pub thread_id: String,
    pub pull_request: Option<PullRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(by: &str, ask: ReviewAsk, at: &str) -> ReviewAction {
        ReviewAction {
            by: by.into(),
            ask,
            at: at.into(),
        }
    }

    #[test]
    fn reads_timestamps_in_any_offset() {
        assert_eq!(timestamp_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            timestamp_millis("2026-11-11T12:00:00Z"),
            timestamp_millis("2026-11-11T09:00:00.000000-03:00")
        );
        assert_eq!(
            timestamp_millis("2026-11-11T12:00:00.250+00:00"),
            timestamp_millis("2026-11-11T12:00:00Z").map(|at| at + 250)
        );
        assert_eq!(timestamp_millis("2026-11-11"), None);
    }

    #[test]
    fn a_request_for_changes_waits_on_the_author_until_they_answer() {
        let ask = action(
            "gabriel",
            ReviewAsk::ChangesRequested,
            "2026-11-10T10:00:00.123+00:00",
        );
        let reviewers = ["gabriel".to_owned()];

        assert_eq!(
            ReviewTurn::of(Some("ana"), std::slice::from_ref(&ask), &reviewers, &[]),
            Some(ReviewTurn::Author {
                by: "gabriel".into(),
                ask: ReviewAsk::ChangesRequested,
                at: ask.at.clone(),
            })
        );

        let reply = action("Ana", ReviewAsk::Comment, "2026-11-10T08:00:00-03:00");
        assert_eq!(
            ReviewTurn::of(Some("ana"), &[ask, reply.clone()], &reviewers, &[]),
            Some(ReviewTurn::Reviewers {
                waiting_on: vec!["gabriel".into()],
                replied_at: reply.at,
            })
        );
    }

    #[test]
    fn a_later_comment_from_anyone_else_passes_the_turn_back() {
        let actions = [
            action("ana", ReviewAsk::Comment, "2026-11-10T10:00:00Z"),
            action("luis", ReviewAsk::Comment, "2026-11-10T11:00:00Z"),
        ];
        assert_eq!(
            ReviewTurn::of(Some("ana"), &actions, &[], &[]),
            Some(ReviewTurn::Author {
                by: "luis".into(),
                ask: ReviewAsk::Comment,
                at: "2026-11-10T11:00:00Z".into(),
            })
        );
    }

    #[test]
    fn an_unanswered_request_for_changes_outranks_later_comments() {
        let actions = [
            action("ana", ReviewAsk::Comment, "2026-11-10T09:00:00Z"),
            action("luis", ReviewAsk::ChangesRequested, "2026-11-10T10:00:00Z"),
            action("luis", ReviewAsk::Comment, "2026-11-10T10:00:01Z"),
        ];
        assert_eq!(
            ReviewTurn::of(Some("ana"), &actions, &["luis".to_owned()], &[]),
            Some(ReviewTurn::Author {
                by: "luis".into(),
                ask: ReviewAsk::ChangesRequested,
                at: "2026-11-10T10:00:00Z".into(),
            })
        );
    }

    #[test]
    fn an_approval_settles_what_its_reviewer_asked_before() {
        let actions = [
            action("ana", ReviewAsk::Comment, "2026-11-10T09:00:00Z"),
            action("luis", ReviewAsk::Comment, "2026-11-10T10:00:00Z"),
            action("eva", ReviewAsk::Comment, "2026-11-10T10:30:00Z"),
        ];
        let luis = [ReviewApproval {
            by: "luis".into(),
            at: "2026-11-10T11:00:00Z".into(),
        }];
        assert_eq!(ReviewTurn::of(Some("ana"), &actions[..2], &[], &luis), None);
        assert_eq!(
            ReviewTurn::of(Some("ana"), &actions, &[], &luis),
            Some(ReviewTurn::Author {
                by: "eva".into(),
                ask: ReviewAsk::Comment,
                at: "2026-11-10T10:30:00Z".into(),
            })
        );
    }

    #[test]
    fn no_turn_until_someone_else_speaks() {
        let mine = [action("ana", ReviewAsk::Comment, "2026-11-10T10:00:00Z")];
        assert_eq!(ReviewTurn::of(Some("ana"), &mine, &[], &[]), None);
        assert_eq!(ReviewTurn::of(None, &mine, &[], &[]), None);
    }
}
