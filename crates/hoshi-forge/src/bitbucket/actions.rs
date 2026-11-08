//! What each pull request change is on Bitbucket Cloud's REST API.

use super::Method;
use hoshi_core::{MergeMethod, PullRequestAction};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// One request under the pull request's own path (`""` is the pull request itself).
    Request {
        method: Method,
        path: String,
        body: Option<Value>,
    },
    /// Bitbucket replaces the whole reviewer list, so the current one is read first.
    Reviewers {
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// The forge has no way to do it.
    Unsupported(&'static str),
}

fn request(method: Method, path: impl Into<String>, body: Option<Value>) -> Plan {
    Plan::Request {
        method,
        path: path.into(),
        body,
    }
}

pub fn plan(action: &PullRequestAction) -> Plan {
    match action {
        PullRequestAction::Merge { method } => request(
            Method::Post,
            "/merge",
            Some(json!({ "merge_strategy": merge_strategy(*method) })),
        ),
        PullRequestAction::SetOpen { open: false } => request(Method::Post, "/decline", None),
        PullRequestAction::SetOpen { open: true } => {
            Plan::Unsupported("Bitbucket can't reopen a declined pull request")
        }
        PullRequestAction::SetDraft { .. } => {
            Plan::Unsupported("Bitbucket's API can't change a pull request's draft state")
        }
        PullRequestAction::Edit { title, body } => request(
            Method::Put,
            "",
            Some(json!({ "title": title, "description": body })),
        ),
        PullRequestAction::RequestReviewers { names } => Plan::Reviewers {
            add: names.clone(),
            remove: Vec::new(),
        },
        PullRequestAction::RemoveReviewers { names } => Plan::Reviewers {
            add: Vec::new(),
            remove: names.clone(),
        },
        PullRequestAction::AddLabels { .. } | PullRequestAction::RemoveLabel { .. } => {
            Plan::Unsupported("Bitbucket pull requests have no labels")
        }
        PullRequestAction::Comment { body } => request(
            Method::Post,
            "/comments",
            Some(json!({ "content": { "raw": body } })),
        ),
        PullRequestAction::ReplyToThread { thread_id, body } => match thread_id.parse::<u64>() {
            Ok(parent) => request(
                Method::Post,
                "/comments",
                Some(json!({ "content": { "raw": body }, "parent": { "id": parent } })),
            ),
            Err(_) => Plan::Unsupported("not a Bitbucket comment"),
        },
        PullRequestAction::SetThreadResolved {
            thread_id,
            resolved,
        } => match thread_id.parse::<u64>() {
            Ok(id) => request(
                if *resolved {
                    Method::Post
                } else {
                    Method::Delete
                },
                format!("/comments/{id}/resolve"),
                None,
            ),
            Err(_) => Plan::Unsupported("not a Bitbucket comment"),
        },
    }
}

fn merge_strategy(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "merge_commit",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase_fast_forward",
    }
}

/// The reviewer list to send: the current one, plus the added, minus the removed, by UUID.
pub fn reviewer_list(current: &[String], add: &[String], remove: &[String]) -> Value {
    let mut uuids: Vec<&String> = current
        .iter()
        .filter(|uuid| !remove.contains(uuid))
        .collect();
    for uuid in add {
        if !uuids.contains(&uuid) {
            uuids.push(uuid);
        }
    }
    json!({ "reviewers": uuids.iter().map(|uuid| json!({ "uuid": uuid })).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_with_bitbucket_strategy_names() {
        assert_eq!(
            plan(&PullRequestAction::Merge {
                method: MergeMethod::Rebase
            }),
            request(
                Method::Post,
                "/merge",
                Some(json!({ "merge_strategy": "rebase_fast_forward" }))
            )
        );
    }

    #[test]
    fn replies_and_resolves_by_comment_id() {
        assert_eq!(
            plan(&PullRequestAction::ReplyToThread {
                thread_id: "41".into(),
                body: "ok".into()
            }),
            request(
                Method::Post,
                "/comments",
                Some(json!({ "content": { "raw": "ok" }, "parent": { "id": 41 } }))
            )
        );
        assert_eq!(
            plan(&PullRequestAction::SetThreadResolved {
                thread_id: "41".into(),
                resolved: false
            }),
            request(Method::Delete, "/comments/41/resolve", None)
        );
    }

    #[test]
    fn refuses_what_bitbucket_lacks() {
        for action in [
            PullRequestAction::SetOpen { open: true },
            PullRequestAction::SetDraft { draft: true },
            PullRequestAction::RemoveLabel { name: "bug".into() },
        ] {
            assert!(matches!(plan(&action), Plan::Unsupported(_)), "{action:?}");
        }
    }

    #[test]
    fn keeps_the_reviewer_list_whole() {
        let current = ["{a}".to_owned(), "{b}".to_owned()];
        assert_eq!(
            reviewer_list(&current, &["{c}".into(), "{a}".into()], &["{b}".into()]),
            json!({ "reviewers": [{ "uuid": "{a}" }, { "uuid": "{c}" }] })
        );
    }
}
