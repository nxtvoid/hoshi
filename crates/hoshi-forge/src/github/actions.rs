//! Changes the user asks GitHub to make to a pull request, and the lists their pickers show.

use super::Method;
use crate::{ForgeError, RepositoryRef};
use hoshi_core::{MergeMethod, PullRequestAction};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const ASSIGNABLE_USERS_QUERY: &str = "
query($owner: String!, $name: String!, $query: String!) {
  repository(owner: $owner, name: $name) {
    assignableUsers(first: 20, query: $query) { nodes { login } }
  }
}";

const CONVERT_TO_DRAFT: &str = "
mutation($id: ID!) { convertPullRequestToDraft(input: { pullRequestId: $id }) { clientMutationId } }";
const MARK_READY: &str = "
mutation($id: ID!) { markPullRequestReadyForReview(input: { pullRequestId: $id }) { clientMutationId } }";
const REPLY_TO_THREAD: &str = "
mutation($id: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: { pullRequestReviewThreadId: $id, body: $body }) { clientMutationId }
}";
const RESOLVE_THREAD: &str = "
mutation($id: ID!) { resolveReviewThread(input: { threadId: $id }) { clientMutationId } }";
const UNRESOLVE_THREAD: &str = "
mutation($id: ID!) { unresolveReviewThread(input: { threadId: $id }) { clientMutationId } }";

/// How one action reaches GitHub.
#[derive(Debug, PartialEq)]
pub(super) enum Call {
    Rest {
        method: Method,
        path: String,
        body: Option<Value>,
    },
    Mutation {
        query: &'static str,
        variables: Value,
    },
}

/// The request that carries out `action` on pull request `number`, whose GraphQL id is
/// `forge_id`.
pub(super) fn plan(
    repository: &RepositoryRef,
    number: u32,
    forge_id: &str,
    action: &PullRequestAction,
) -> Call {
    let repo = format!("/repos/{}/{}", repository.owner, repository.name);
    let rest = |method, path: String, body: Option<Value>| Call::Rest { method, path, body };
    match action {
        PullRequestAction::Merge { method } => rest(
            Method::Put,
            format!("{repo}/pulls/{number}/merge"),
            Some(json!({ "merge_method": merge_method(*method) })),
        ),
        PullRequestAction::SetOpen { open } => rest(
            Method::Patch,
            format!("{repo}/pulls/{number}"),
            Some(json!({ "state": if *open { "open" } else { "closed" } })),
        ),
        PullRequestAction::SetDraft { draft } => Call::Mutation {
            query: if *draft { CONVERT_TO_DRAFT } else { MARK_READY },
            variables: json!({ "id": forge_id }),
        },
        PullRequestAction::Edit { title, body } => rest(
            Method::Patch,
            format!("{repo}/pulls/{number}"),
            Some(json!({ "title": title, "body": body })),
        ),
        PullRequestAction::RequestReviewers { names } => rest(
            Method::Post,
            format!("{repo}/pulls/{number}/requested_reviewers"),
            Some(json!({ "reviewers": names })),
        ),
        PullRequestAction::RemoveReviewers { names } => rest(
            Method::Delete,
            format!("{repo}/pulls/{number}/requested_reviewers"),
            Some(json!({ "reviewers": names })),
        ),
        PullRequestAction::AddLabels { names } => rest(
            Method::Post,
            format!("{repo}/issues/{number}/labels"),
            Some(json!({ "labels": names })),
        ),
        PullRequestAction::RemoveLabel { name } => rest(
            Method::Delete,
            format!("{repo}/issues/{number}/labels/{}", percent_encode(name)),
            None,
        ),
        PullRequestAction::Comment { body } => rest(
            Method::Post,
            format!("{repo}/issues/{number}/comments"),
            Some(json!({ "body": body })),
        ),
        PullRequestAction::ReplyToThread { thread_id, body } => Call::Mutation {
            query: REPLY_TO_THREAD,
            variables: json!({ "id": thread_id, "body": body }),
        },
        PullRequestAction::SetThreadResolved {
            thread_id,
            resolved,
        } => Call::Mutation {
            query: if *resolved {
                RESOLVE_THREAD
            } else {
                UNRESOLVE_THREAD
            },
            variables: json!({ "id": thread_id }),
        },
    }
}

/// A mutation answers 200 even when it fails; the reason is in `errors`.
pub(super) fn check_mutation(text: &str) -> Result<(), ForgeError> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        errors: Vec<Message>,
    }
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    let answer: Answer =
        serde_json::from_str(text).map_err(|error| ForgeError::Unexpected(error.to_string()))?;
    match answer.errors.into_iter().next() {
        Some(error) => Err(ForgeError::Rejected(error.message)),
        None => Ok(()),
    }
}

#[derive(Deserialize)]
pub(super) struct AssignableData {
    repository: Option<AssignableRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignableRepository {
    assignable_users: Nodes<Login>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

pub(super) fn logins_from(data: AssignableData) -> Vec<String> {
    data.repository
        .map(|repository| {
            repository
                .assignable_users
                .nodes
                .into_iter()
                .map(|user| user.login)
                .collect()
        })
        .unwrap_or_default()
}

fn merge_method(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "merge",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    }
}

/// Label names go in the URL path, where spaces and symbols must be escaped.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::ForgeKind;

    fn repository() -> RepositoryRef {
        RepositoryRef {
            forge: ForgeKind::Github,
            host: "github.com".into(),
            owner: "o".into(),
            name: "r".into(),
        }
    }

    fn rest(call: Call) -> (Method, String, Option<Value>) {
        match call {
            Call::Rest { method, path, body } => (method, path, body),
            Call::Mutation { .. } => panic!("expected a REST call"),
        }
    }

    #[test]
    fn merging_uses_the_chosen_method() {
        let action = PullRequestAction::Merge {
            method: MergeMethod::Squash,
        };

        let (method, path, body) = rest(plan(&repository(), 3, "PR_1", &action));

        assert_eq!(method, Method::Put);
        assert_eq!(path, "/repos/o/r/pulls/3/merge");
        assert_eq!(body, Some(json!({ "merge_method": "squash" })));
    }

    #[test]
    fn closing_and_editing_patch_the_pull_request() {
        let close = rest(plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::SetOpen { open: false },
        ));
        let edit = rest(plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::Edit {
                title: "New".into(),
                body: "Text".into(),
            },
        ));

        assert_eq!(close.1, "/repos/o/r/pulls/3");
        assert_eq!(close.2, Some(json!({ "state": "closed" })));
        assert_eq!(edit.0, Method::Patch);
        assert_eq!(edit.2, Some(json!({ "title": "New", "body": "Text" })));
    }

    #[test]
    fn labels_go_through_the_issue_and_names_are_escaped() {
        let (method, path, body) = rest(plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::RemoveLabel {
                name: "good first issue/ä".into(),
            },
        ));

        assert_eq!(method, Method::Delete);
        assert_eq!(
            path,
            "/repos/o/r/issues/3/labels/good%20first%20issue%2F%C3%A4"
        );
        assert_eq!(body, None);
    }

    #[test]
    fn removing_reviewers_sends_them_in_the_body() {
        let (method, path, body) = rest(plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::RemoveReviewers {
                names: vec!["ana".into()],
            },
        ));

        assert_eq!(method, Method::Delete);
        assert_eq!(path, "/repos/o/r/pulls/3/requested_reviewers");
        assert_eq!(body, Some(json!({ "reviewers": ["ana"] })));
    }

    #[test]
    fn drafts_and_threads_use_mutations_with_forge_ids() {
        let draft = plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::SetDraft { draft: false },
        );
        let reply = plan(
            &repository(),
            3,
            "PR_1",
            &PullRequestAction::ReplyToThread {
                thread_id: "PRRT_9".into(),
                body: "Done".into(),
            },
        );

        assert_eq!(
            draft,
            Call::Mutation {
                query: MARK_READY,
                variables: json!({ "id": "PR_1" })
            }
        );
        assert_eq!(
            reply,
            Call::Mutation {
                query: REPLY_TO_THREAD,
                variables: json!({ "id": "PRRT_9", "body": "Done" })
            }
        );
    }

    #[test]
    fn a_failed_mutation_is_a_rejection_with_its_reason() {
        let text = r#"{"data":null,"errors":[{"message":"Pull request is already a draft"}]}"#;

        assert_eq!(
            check_mutation(text),
            Err(ForgeError::Rejected(
                "Pull request is already a draft".into()
            ))
        );
        assert_eq!(check_mutation(r#"{"data":{}}"#), Ok(()));
    }
}
