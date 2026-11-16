use crate::m;
use hoshi_core::{CommandError, GitAction, GitActionPhase, GitActionResult, GitStatus};
use hoshi_ui::prelude::IconName;

/// What the main button is named for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Commit,
    CommitPush,
    CommitPushPullRequest,
    Push,
    PushCreatePullRequest,
    CreatePullRequest,
    Pull,
}

/// Why an action can't run now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Detached,
    Clean,
    Uncommitted,
    Behind,
    Diverged,
    NoRemote,
    NoCommits,
    UpToDate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickAction {
    pub label: Label,
    pub action: GitAction,
    /// Set when the button shows, but can't run.
    pub blocker: Option<Blocker>,
}

impl QuickAction {
    fn ready(label: Label, action: GitAction) -> Self {
        Self {
            label,
            action,
            blocker: None,
        }
    }

    fn blocked(blocker: Blocker) -> Self {
        Self {
            label: Label::Commit,
            action: GitAction::Commit,
            blocker: Some(blocker),
        }
    }
}

pub fn on_default_branch(status: &GitStatus) -> bool {
    status.branch.is_some() && status.branch == status.default_branch
}

/// The action the main button runs: commit what changed and take it as far as it can go,
/// push what's ahead, pull what's behind, or nothing when up to date.
pub fn quick_action(status: &GitStatus, pull_request_open: bool) -> QuickAction {
    if status.branch.is_none() {
        return QuickAction::blocked(Blocker::Detached);
    }
    let settled = pull_request_open || on_default_branch(status);
    let push_or_request = || {
        if settled {
            QuickAction::ready(Label::Push, GitAction::Push)
        } else {
            QuickAction::ready(Label::PushCreatePullRequest, GitAction::CreatePullRequest)
        }
    };
    if status.has_changes {
        if status.behind > 0 || (!status.has_upstream && !status.has_remote) {
            return QuickAction::ready(Label::Commit, GitAction::Commit);
        }
        return if settled {
            QuickAction::ready(Label::CommitPush, GitAction::CommitPush)
        } else {
            QuickAction::ready(
                Label::CommitPushPullRequest,
                GitAction::CommitPushPullRequest,
            )
        };
    }
    if !status.has_upstream {
        if !status.has_remote {
            return QuickAction::blocked(Blocker::NoRemote);
        }
        if status.ahead_of_default == 0 {
            return QuickAction {
                label: Label::Push,
                action: GitAction::Push,
                blocker: Some(if pull_request_open {
                    Blocker::UpToDate
                } else {
                    Blocker::NoCommits
                }),
            };
        }
        return push_or_request();
    }
    if status.ahead > 0 && status.behind > 0 {
        return QuickAction {
            label: Label::Pull,
            action: GitAction::Pull,
            blocker: Some(Blocker::Diverged),
        };
    }
    if status.behind > 0 {
        return QuickAction::ready(Label::Pull, GitAction::Pull);
    }
    if status.ahead > 0 {
        return push_or_request();
    }
    if !pull_request_open && status.ahead_of_default > 0 && !on_default_branch(status) {
        return QuickAction::ready(Label::CreatePullRequest, GitAction::CreatePullRequest);
    }
    QuickAction::blocked(Blocker::UpToDate)
}

/// Why each menu entry can't run now; `None` when it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuBlockers {
    pub commit: Option<Blocker>,
    pub push: Option<Blocker>,
    pub create_pull_request: Option<Blocker>,
}

pub fn menu_blockers(status: &GitStatus) -> MenuBlockers {
    let push = if status.branch.is_none() {
        Some(Blocker::Detached)
    } else if status.has_changes {
        Some(Blocker::Uncommitted)
    } else if status.behind > 0 {
        Some(Blocker::Behind)
    } else if !status.has_remote {
        Some(Blocker::NoRemote)
    } else {
        let ahead = if status.has_upstream {
            status.ahead
        } else {
            status.ahead_of_default
        };
        (ahead == 0).then_some(Blocker::NoCommits)
    };
    let create_pull_request = if status.branch.is_none() {
        Some(Blocker::Detached)
    } else if status.behind > 0 {
        Some(Blocker::Behind)
    } else if !status.has_remote {
        Some(Blocker::NoRemote)
    } else {
        (status.ahead_of_default == 0).then_some(Blocker::NoCommits)
    };
    MenuBlockers {
        commit: (!status.has_changes).then_some(Blocker::Clean),
        push,
        create_pull_request,
    }
}

/// Pushing straight to the default branch asks first, as it skips review.
pub fn pushes_to_default_branch(status: &GitStatus, action: GitAction) -> bool {
    !matches!(action, GitAction::Commit | GitAction::Pull) && on_default_branch(status)
}

pub fn label_text(label: Label, draft: bool) -> String {
    match label {
        Label::Commit => m::git_commit(),
        Label::CommitPush => m::git_commit_push(),
        Label::CommitPushPullRequest if draft => m::git_commit_push_draft_pr(),
        Label::CommitPushPullRequest => m::git_commit_push_pr(),
        Label::Push => m::git_push(),
        Label::PushCreatePullRequest if draft => m::git_push_create_draft_pr(),
        Label::PushCreatePullRequest => m::git_push_create_pr(),
        Label::CreatePullRequest if draft => m::git_create_draft_pr(),
        Label::CreatePullRequest => m::git_create_pr(),
        Label::Pull => m::git_pull(),
    }
}

pub fn label_icon(label: Label) -> IconName {
    match label {
        Label::Commit => IconName::GitCommitHorizontal,
        Label::CommitPush | Label::Push => IconName::CloudUpload,
        Label::Pull => IconName::CloudDownload,
        _ => IconName::GitPullRequest,
    }
}

pub fn blocker_text(blocker: Blocker) -> String {
    match blocker {
        Blocker::Detached => m::git_blocked_detached(),
        Blocker::Clean => m::git_blocked_clean(),
        Blocker::Uncommitted => m::git_blocked_uncommitted(),
        Blocker::Behind => m::git_blocked_behind(),
        Blocker::Diverged => m::git_blocked_diverged(),
        Blocker::NoRemote => m::git_blocked_no_remote(),
        Blocker::NoCommits => m::git_blocked_no_commits(),
        Blocker::UpToDate => m::git_blocked_up_to_date(),
    }
}

pub fn phase_text(phase: Option<GitActionPhase>) -> String {
    match phase {
        Some(GitActionPhase::WritingCommitMessage) => m::git_phase_writing_commit_message(),
        Some(GitActionPhase::CreatingBranch) => m::git_phase_creating_branch(),
        Some(GitActionPhase::Committing) => m::git_phase_committing(),
        Some(GitActionPhase::Pushing) => m::git_phase_pushing(),
        Some(GitActionPhase::WritingPullRequest) => m::git_phase_writing_pull_request(),
        Some(GitActionPhase::CreatingPullRequest) => m::git_phase_creating_pull_request(),
        Some(GitActionPhase::Pulling) => m::git_phase_pulling(),
        None => m::git_phase_starting(),
    }
}

/// What a finished action says in place of the button: a title and a muted line.
pub fn done_text(result: &GitActionResult) -> (String, Option<String>) {
    if let Some(pull_request) = &result.pull_request {
        let title = if pull_request.existing {
            m::git_done_opened_pr(pull_request.number)
        } else {
            m::git_done_created_pr(pull_request.number)
        };
        return (title, Some(pull_request.title.clone()));
    }
    if let Some(branch) = &result.pushed {
        let title = match &result.commit {
            Some(commit) => m::git_done_pushed(branch, commit),
            None => m::git_done_pushed_branch(branch),
        };
        return (title, result.subject.clone());
    }
    if let Some(commit) = &result.commit {
        return (m::git_done_committed(commit), result.subject.clone());
    }
    if result.pulled {
        return (m::git_done_pulled(), None);
    }
    (m::git_done(), None)
}

pub fn error_text(error: &CommandError) -> String {
    match error {
        CommandError::NothingToCommit { .. } => m::git_error_nothing_to_commit(),
        CommandError::CommitMessageUnavailable { .. } => m::git_error_commit_message(),
        CommandError::PushBehind { .. } => m::git_error_push_behind(),
        CommandError::NoRemote { .. } => m::git_blocked_no_remote(),
        CommandError::DetachedHead { .. } => m::git_blocked_detached(),
        CommandError::NoBaseBranch { .. } => m::git_error_no_base_branch(),
        CommandError::ForgeUnauthorized => m::git_error_forge_unauthorized(),
        CommandError::ForgeUnavailable => m::git_error_forge_unavailable(),
        CommandError::ThreadBusy { .. } => m::git_error_busy(),
        CommandError::GitFailed { detail }
        | CommandError::ForgeRejected { detail }
        | CommandError::ForgeUnreachable { detail } => detail.clone(),
        _ => m::error_generic(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Blocker, Label, MenuBlockers, menu_blockers, pushes_to_default_branch, quick_action,
    };
    use hoshi_core::{GitAction, GitStatus};

    fn status() -> GitStatus {
        GitStatus {
            branch: Some("hoshi/login".into()),
            default_branch: Some("main".into()),
            has_remote: true,
            ..GitStatus::default()
        }
    }

    fn pick(status: GitStatus, pull_request_open: bool) -> (Label, Option<Blocker>) {
        let quick = quick_action(&status, pull_request_open);
        (quick.label, quick.blocker)
    }

    #[test]
    fn changes_go_as_far_as_they_can() {
        let changed = GitStatus {
            has_changes: true,
            ..status()
        };
        assert_eq!(
            pick(changed.clone(), false),
            (Label::CommitPushPullRequest, None)
        );
        assert_eq!(pick(changed.clone(), true), (Label::CommitPush, None));
        let on_main = GitStatus {
            branch: Some("main".into()),
            ..changed.clone()
        };
        assert_eq!(pick(on_main, false), (Label::CommitPush, None));
        let no_remote = GitStatus {
            has_remote: false,
            ..changed.clone()
        };
        assert_eq!(pick(no_remote, false), (Label::Commit, None));
        let behind = GitStatus {
            has_upstream: true,
            behind: 2,
            ..changed
        };
        assert_eq!(pick(behind, false), (Label::Commit, None));
    }

    #[test]
    fn commits_not_pushed_yet_are_pushed_with_a_pull_request_when_there_is_none() {
        let ahead = GitStatus {
            ahead_of_default: 1,
            ..status()
        };
        assert_eq!(
            pick(ahead.clone(), false),
            (Label::PushCreatePullRequest, None)
        );
        assert_eq!(pick(ahead.clone(), true), (Label::Push, None));
        let upstream = GitStatus {
            has_upstream: true,
            ahead: 1,
            ..ahead
        };
        assert_eq!(
            pick(upstream.clone(), false),
            (Label::PushCreatePullRequest, None)
        );
        assert_eq!(pick(upstream, true), (Label::Push, None));
        assert_eq!(
            quick_action(
                &GitStatus {
                    ahead_of_default: 1,
                    ..status()
                },
                false
            )
            .action,
            GitAction::CreatePullRequest
        );
    }

    #[test]
    fn behind_pulls_diverged_waits_and_pushed_work_only_opens_a_pull_request() {
        let upstream = GitStatus {
            has_upstream: true,
            ..status()
        };
        assert_eq!(
            pick(
                GitStatus {
                    behind: 1,
                    ..upstream.clone()
                },
                false
            ),
            (Label::Pull, None)
        );
        assert_eq!(
            pick(
                GitStatus {
                    behind: 1,
                    ahead: 1,
                    ..upstream.clone()
                },
                false
            ),
            (Label::Pull, Some(Blocker::Diverged))
        );
        let pushed = GitStatus {
            ahead_of_default: 2,
            ..upstream
        };
        assert_eq!(
            pick(pushed.clone(), false),
            (Label::CreatePullRequest, None)
        );
        assert_eq!(pick(pushed, true), (Label::Commit, Some(Blocker::UpToDate)));
    }

    #[test]
    fn nothing_to_do_no_remote_or_no_branch_block_the_button() {
        assert_eq!(
            pick(status(), false),
            (Label::Push, Some(Blocker::NoCommits))
        );
        assert_eq!(
            pick(
                GitStatus {
                    has_remote: false,
                    ..status()
                },
                false
            ),
            (Label::Commit, Some(Blocker::NoRemote))
        );
        assert_eq!(
            pick(
                GitStatus {
                    branch: None,
                    has_changes: true,
                    ..status()
                },
                false
            ),
            (Label::Commit, Some(Blocker::Detached))
        );
    }

    #[test]
    fn the_menu_says_why_each_entry_waits() {
        assert_eq!(
            menu_blockers(&status()),
            MenuBlockers {
                commit: Some(Blocker::Clean),
                push: Some(Blocker::NoCommits),
                create_pull_request: Some(Blocker::NoCommits),
            }
        );
        assert_eq!(
            menu_blockers(&GitStatus {
                has_changes: true,
                ahead_of_default: 1,
                ..status()
            }),
            MenuBlockers {
                commit: None,
                push: Some(Blocker::Uncommitted),
                create_pull_request: None,
            }
        );
    }

    #[test]
    fn pushing_to_the_default_branch_is_the_one_that_asks_first() {
        let main = GitStatus {
            branch: Some("main".into()),
            has_changes: true,
            ..status()
        };
        assert!(pushes_to_default_branch(&main, GitAction::CommitPush));
        assert!(!pushes_to_default_branch(&main, GitAction::Commit));
        assert!(!pushes_to_default_branch(&status(), GitAction::Push));
    }
}
