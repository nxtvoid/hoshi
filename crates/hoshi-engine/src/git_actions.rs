//! Commit, push and pull request actions on a thread's checkout, as t3code runs them.

use crate::source_control::{self, PushFailure};
use crate::{
    Engine, Notification, PullRequestJob, forge_error, git, lock, read_branches, worktree,
};
use hoshi_core::{
    CommandError, GitAction, GitActionPhase, GitActionProgress, GitActionPullRequest,
    GitActionRequest, GitActionResult, GitStatus, PullRequestState,
};
use hoshi_forge::NewPullRequest;
use hoshi_providers::{CommitMessage, CommitMessageRequest, PullRequestTextRequest};
use std::sync::Arc;

impl Engine {
    /// What the thread's checkout allows: something to commit, push or pull.
    pub fn git_status(&self, thread_id: &str) -> Result<GitStatus, CommandError> {
        let cwd = self.repository_cwd(thread_id)?;
        source_control::status(&cwd).map_err(|detail| CommandError::GitFailed { detail })
    }

    /// Commits, pushes and opens a pull request as `request` asks, reporting each step as a
    /// [`Notification::GitActionProgress`]. Refused while the thread's agent works.
    pub fn run_git_action(
        &self,
        thread_id: &str,
        request: &GitActionRequest,
    ) -> Result<GitActionResult, CommandError> {
        let (cwd, adapter, target, notify) = {
            let state = lock(&self.state);
            let thread =
                state
                    .model
                    .thread(thread_id)
                    .ok_or_else(|| CommandError::ThreadNotFound {
                        thread_id: thread_id.to_owned(),
                    })?;
            if thread.active_turn_id.is_some() {
                return Err(CommandError::ThreadBusy {
                    thread_id: thread_id.to_owned(),
                });
            }
            (
                state.cwd(thread_id),
                state.adapter.clone(),
                state.pull_request_target_of(thread_id),
                state.notify.clone(),
            )
        };
        let cwd = self
            .repository_cwd(thread_id)
            .or_else(|error| cwd.ok_or(error))?;
        let progress = |phase| {
            notify(Notification::GitActionProgress(GitActionProgress {
                thread_id: thread_id.to_owned(),
                phase,
            }));
        };
        let failed = |detail: String| CommandError::GitFailed { detail };
        let owned = || thread_id.to_owned();
        let mut result = GitActionResult::default();
        let action = request.action;

        if action == GitAction::Pull {
            progress(GitActionPhase::Pulling);
            source_control::pull(&cwd).map_err(failed)?;
            result.pulled = true;
        }

        let commits = matches!(
            action,
            GitAction::Commit | GitAction::CommitPush | GitAction::CommitPushPullRequest
        );
        if commits {
            let paths = request.paths.as_deref();
            let branch = git::current_branch(&cwd);
            let message = match request
                .message
                .as_deref()
                .and_then(source_control::split_message)
            {
                Some((subject, body)) => CommitMessage {
                    subject,
                    body,
                    branch: None,
                },
                None => {
                    let staged = source_control::preview(&cwd, paths).map_err(failed)?;
                    if staged.files.trim().is_empty() {
                        return Err(CommandError::NothingToCommit { thread_id: owned() });
                    }
                    progress(GitActionPhase::WritingCommitMessage);
                    let written = adapter.as_ref().and_then(|adapter| {
                        adapter.write_commit_message(&CommitMessageRequest {
                            branch: branch.clone().unwrap_or_default(),
                            files: staged.files,
                            patch: staged.patch,
                            recent_subjects: source_control::recent_subjects(&cwd),
                            wants_branch: request.new_branch,
                        })
                    });
                    written.ok_or_else(|| CommandError::CommitMessageUnavailable {
                        thread_id: owned(),
                    })?
                }
            };
            if request.new_branch {
                progress(GitActionPhase::CreatingBranch);
                let name = message
                    .branch
                    .as_deref()
                    .and_then(worktree::branch_name)
                    .or_else(|| worktree::branch_name(&message.subject))
                    .unwrap_or_else(|| "change".to_owned());
                let branch = free_branch(&cwd, &name);
                source_control::create_branch(&cwd, &branch).map_err(failed)?;
            }
            progress(GitActionPhase::Committing);
            let hash = source_control::commit(&cwd, paths, &message.subject, &message.body)
                .map_err(failed)?
                .ok_or_else(|| CommandError::NothingToCommit { thread_id: owned() })?;
            result.commit = Some(hash);
            result.subject = Some(message.subject);
        }

        let pushes = matches!(
            action,
            GitAction::CommitPush
                | GitAction::CommitPushPullRequest
                | GitAction::Push
                | GitAction::CreatePullRequest
        );
        let opens = matches!(
            action,
            GitAction::CommitPushPullRequest | GitAction::CreatePullRequest
        );
        if pushes {
            let branch = git::current_branch(&cwd)
                .ok_or_else(|| CommandError::DetachedHead { thread_id: owned() })?;
            let status = source_control::status(&cwd).map_err(failed)?;
            if !status.has_remote {
                return Err(CommandError::NoRemote { thread_id: owned() });
            }
            if !status.has_upstream || status.ahead > 0 {
                progress(GitActionPhase::Pushing);
                source_control::push(&cwd, &branch).map_err(|failure| match failure {
                    PushFailure::Behind => CommandError::PushBehind { thread_id: owned() },
                    PushFailure::Other(detail) => CommandError::GitFailed { detail },
                })?;
                result.pushed = Some(branch.clone());
            }
            if opens {
                let (forge, repository) = target.ok_or(CommandError::ForgeUnavailable)?;
                let existing = forge
                    .pull_request_for_branch(&repository, &branch)
                    .map_err(forge_error)?
                    .filter(|pull_request| {
                        matches!(
                            pull_request.state,
                            PullRequestState::Open | PullRequestState::Draft
                        )
                    });
                let (pull_request, existed) = match existing {
                    Some(pull_request) => (pull_request, true),
                    None => {
                        let (base, reference) = source_control::base_branch(&cwd, &branch)
                            .ok_or_else(|| CommandError::NoBaseBranch { thread_id: owned() })?;
                        progress(GitActionPhase::WritingPullRequest);
                        let range = source_control::range(&cwd, &reference).map_err(failed)?;
                        let fallback_title = range
                            .commits
                            .lines()
                            .last()
                            .and_then(|line| line.split_once(' '))
                            .map_or_else(|| branch.clone(), |(_, subject)| subject.to_owned());
                        let text = adapter.as_ref().and_then(|adapter| {
                            adapter.write_pull_request(&PullRequestTextRequest {
                                base: base.clone(),
                                head: branch.clone(),
                                commits: range.commits,
                                stat: range.stat,
                                patch: range.patch,
                                template: source_control::template(&cwd),
                                recent_subjects: source_control::recent_subjects(&cwd),
                            })
                        });
                        let (title, body) = text.map_or((fallback_title, String::new()), |text| {
                            (text.title, text.body)
                        });
                        progress(GitActionPhase::CreatingPullRequest);
                        let created = forge
                            .create_pull_request(
                                &repository,
                                &NewPullRequest {
                                    head: branch.clone(),
                                    base,
                                    title,
                                    body,
                                    draft: request.draft,
                                },
                            )
                            .map_err(forge_error)?;
                        (created, false)
                    }
                };
                result.pull_request = Some(GitActionPullRequest {
                    number: pull_request.number,
                    title: pull_request.title.clone(),
                    url: pull_request.url.clone(),
                    existing: existed,
                });
                let job = PullRequestJob {
                    thread_id: owned(),
                    branch,
                    repository,
                    forge: Arc::clone(&forge),
                };
                lock(&self.state).record_pull_request(&job, Ok(Some(pull_request)));
            }
        }

        read_branches(&Arc::downgrade(&self.state), &[owned()]);
        {
            let mut state = lock(&self.state);
            state.pull_requests.check_soon(thread_id);
            state.wake_sync();
        }
        Ok(result)
    }
}

/// `hoshi/<name>`, or the first of `hoshi/<name>-2`… no branch has taken.
fn free_branch(dir: &str, name: &str) -> String {
    (1..=20)
        .map(|n| match n {
            1 => format!("hoshi/{name}"),
            _ => format!("hoshi/{name}-{n}"),
        })
        .find(|candidate| git::branch_tip(dir, candidate).is_none())
        .unwrap_or_else(|| format!("hoshi/{name}-{}", std::process::id()))
}
