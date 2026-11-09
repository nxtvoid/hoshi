//! `create_pull_request`: the agent of a thread on Bitbucket opens its branch's pull request
//! with the user's stored credentials, which it never sees. On GitHub, agents use `gh`.

use crate::source_control::{self, PushFailure};
use crate::{PullRequestJob, State, git, lock, read_branches};
use hoshi_core::{ForgeKind, PullRequest, PullRequestState};
use hoshi_forge::{Forge, ForgeError, NewPullRequest, RepositoryRef};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, Weak};

pub(crate) const TOOL: &str = "create_pull_request";

impl State {
    /// Whether the thread's agent opens pull requests through hoshi: its repository is on
    /// Bitbucket, which has no CLI the agent could sign in with.
    pub(crate) fn opens_pull_requests(&self, thread_id: &str) -> bool {
        self.pull_request_target_of(thread_id)
            .is_some_and(|(forge, _)| forge.kind() == ForgeKind::Bitbucket)
    }
}

fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_open(pull_request: &PullRequest) -> bool {
    matches!(
        pull_request.state,
        PullRequestState::Open | PullRequestState::Draft
    )
}

fn already_open(pull_request: &PullRequest, branch: &str) -> String {
    format!(
        "Pull request #{} from {branch} is already open: {}. Pushing to {branch} updates it.",
        pull_request.number, pull_request.url
    )
}

/// Pushes the thread's branch when origin lacks commits of it, then opens its pull request
/// and shows it in the thread as hoshi's own action does.
pub(crate) fn create_pull_request(
    state: &Weak<Mutex<State>>,
    caller: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let title = text(arguments, "title").ok_or("`title` is empty")?;
    let body = arguments["description"].as_str().unwrap_or_default();
    let draft = arguments["draft"].as_bool().unwrap_or(false);
    let shared = state.upgrade().ok_or("hoshi is closing")?;
    let (cwd, target) = {
        let state = lock(&shared);
        (state.cwd(caller), state.pull_request_target_of(caller))
    };
    let cwd = cwd.ok_or("this thread no longer exists")?;
    let (forge, repository) = target
        .filter(|(forge, _)| forge.kind() == ForgeKind::Bitbucket)
        .ok_or("this thread's repository isn't on Bitbucket: open its pull request with the forge's own CLI, such as `gh pr create`")?;
    let branch = git::current_branch(&cwd)
        .ok_or("HEAD is detached: create or check out a branch, commit, then call again")?;
    let status = source_control::status(&cwd).map_err(|detail| format!("git failed: {detail}"))?;
    if !status.has_remote {
        return Err("the repository has no `origin` remote to push the branch to".into());
    }
    let base = match text(arguments, "base") {
        Some(base) => base.to_owned(),
        None => source_control::base_branch(&cwd, &branch)
            .map(|(name, _)| name)
            .ok_or("found no default branch to open the pull request on: pass `base`")?,
    };
    if base == branch {
        return Err(format!(
            "{branch} is the base branch: create a branch for the change, commit, then call again"
        ));
    }
    if let Some(existing) = open_pull_request(forge.as_ref(), &repository, &branch)? {
        return Err(already_open(&existing, &branch));
    }
    let pushed = !status.has_upstream || status.ahead > 0;
    if pushed {
        source_control::push(&cwd, &branch).map_err(|failure| match failure {
            PushFailure::Behind => format!(
                "pushing {branch} was refused: origin/{branch} has commits this branch lacks. Pull or rebase onto it, then call again"
            ),
            PushFailure::Other(detail) => {
                format!("pushing {branch} failed; push it, then call again. git said: {detail}")
            }
        })?;
    }
    let request = NewPullRequest {
        head: branch.clone(),
        base: base.clone(),
        title: title.to_owned(),
        body: body.to_owned(),
        draft,
    };
    let created = match forge.create_pull_request(&repository, &request) {
        Ok(created) => created,
        Err(error @ (ForgeError::Conflict(_) | ForgeError::Rejected(_))) => {
            return Err(
                match open_pull_request(forge.as_ref(), &repository, &branch) {
                    Ok(Some(existing)) => already_open(&existing, &branch),
                    _ => agent_error(&error, &repository),
                },
            );
        }
        Err(error) => return Err(agent_error(&error, &repository)),
    };
    read_branches(state, &[caller.to_owned()]);
    {
        let mut state = lock(&shared);
        let job = PullRequestJob {
            thread_id: caller.to_owned(),
            branch: branch.clone(),
            repository,
            forge: Arc::clone(&forge),
        };
        state.record_pull_request(&job, Ok(Some(created.clone())));
        state.pull_requests.check_soon(caller);
        state.wake_sync();
    }
    let mut answer = json!({
        "number": created.number,
        "url": created.url,
        "state": created.state,
        "head": branch,
        "base": base,
        "pushed": pushed,
    });
    if status.has_changes {
        answer["note"] = json!("Uncommitted changes are not in the pull request.");
    }
    Ok(answer)
}

fn open_pull_request(
    forge: &dyn Forge,
    repository: &RepositoryRef,
    branch: &str,
) -> Result<Option<PullRequest>, String> {
    forge
        .pull_request_for_branch(repository, branch)
        .map(|found| found.filter(is_open))
        .map_err(|error| agent_error(&error, repository))
}

/// What the agent reads of a refusal: what went wrong, and who can fix it.
fn agent_error(error: &ForgeError, repository: &RepositoryRef) -> String {
    let name = format!("{}/{}", repository.owner, repository.name);
    match error {
        ForgeError::Unauthorized => "hoshi isn't signed in to Bitbucket, or the user's API token no longer works. Tell the user to connect Bitbucket in hoshi's Settings › Source control. Never look for, ask for or use a token yourself.".into(),
        ForgeError::Forbidden { scopes, .. } if !scopes.is_empty() => format!(
            "the user's Bitbucket API token lacks the scope {}. Tell the user to create a token with it and hoshi's other Bitbucket scopes, then connect it in hoshi's Settings › Source control.",
            scopes.join(", ")
        ),
        ForgeError::Forbidden { message, .. } => format!(
            "Bitbucket refused: the user's account may not open pull requests in {name} ({message}). Tell the user; someone who administers the repository can grant write access."
        ),
        ForgeError::NotFound => format!(
            "Bitbucket shows the user's account no repository {name}: tell the user to check their access to it."
        ),
        ForgeError::Conflict(message) | ForgeError::Rejected(message) => {
            format!("Bitbucket refused the pull request: {message}")
        }
        ForgeError::RateLimited { retry_after_secs } => format!(
            "Bitbucket is limiting requests: try again in {retry_after_secs} seconds"
        ),
        ForgeError::Network(detail) | ForgeError::Unexpected(detail) => {
            format!("couldn't reach Bitbucket: {detail}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> RepositoryRef {
        RepositoryRef {
            forge: ForgeKind::Bitbucket,
            host: "bitbucket.org".into(),
            owner: "acme".into(),
            name: "shop".into(),
        }
    }

    #[test]
    fn each_refusal_tells_the_agent_who_can_fix_it() {
        let says = |error: ForgeError| agent_error(&error, &repository());

        assert!(says(ForgeError::Unauthorized).contains("Settings › Source control"));
        let scope = says(ForgeError::Forbidden {
            message: "Your credentials lack one or more required privilege scopes.".into(),
            scopes: vec!["write:pullrequest:bitbucket".into()],
        });
        assert!(
            scope.contains("lacks the scope write:pullrequest:bitbucket"),
            "{scope}"
        );
        let access = says(ForgeError::Forbidden {
            message: "You do not have access to this repository.".into(),
            scopes: Vec::new(),
        });
        assert!(
            access.contains("acme/shop") && access.contains("write access"),
            "{access}"
        );
        assert!(says(ForgeError::NotFound).contains("no repository acme/shop"));
        assert!(
            says(ForgeError::RateLimited {
                retry_after_secs: 30
            })
            .contains("30 seconds")
        );
    }
}
