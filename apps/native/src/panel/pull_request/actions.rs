use crate::m;
use hoshi_core::{
    CommandError, ForgeKind, MergeMethod, Mergeability, PullRequest, PullRequestAction,
    PullRequestDetail, PullRequestLabel, PullRequestState, ReviewState,
};
use hoshi_engine::Engine;

/// The forge calls the pull request surface makes; the engine in the app, a fake in tests.
pub trait PullRequestForge: Send + Sync + 'static {
    fn act(
        &self,
        thread_id: &str,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), CommandError>;

    fn reviewer_candidates(
        &self,
        thread_id: &str,
        query: &str,
    ) -> Result<Vec<String>, CommandError>;

    fn labels(&self, thread_id: &str) -> Result<Vec<PullRequestLabel>, CommandError>;
}

impl PullRequestForge for Engine {
    fn act(
        &self,
        thread_id: &str,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), CommandError> {
        self.run_pull_request_action(thread_id, forge_id, action)
    }

    fn reviewer_candidates(
        &self,
        thread_id: &str,
        query: &str,
    ) -> Result<Vec<String>, CommandError> {
        Engine::reviewer_candidates(self, thread_id, query)
    }

    fn labels(&self, thread_id: &str) -> Result<Vec<PullRequestLabel>, CommandError> {
        self.repository_labels(thread_id)
    }
}

/// What a failed forge call reads as: the forge's own words when it refused.
pub fn forge_error_message(error: &CommandError) -> String {
    match error {
        CommandError::ForgeUnauthorized => m::forge_error_unauthorized(),
        CommandError::ForgeRateLimited => m::forge_error_rate_limited(),
        CommandError::ForgeRejected { detail } => detail.clone(),
        CommandError::ForgeUnreachable { .. } => m::forge_error_unreachable(),
        _ => m::forge_error_unavailable(),
    }
}

pub fn checkout_command(pull_request: &PullRequest) -> String {
    match pull_request.forge {
        ForgeKind::Github => format!("gh pr checkout {}", pull_request.number),
        ForgeKind::Bitbucket => format!(
            "git fetch origin {0} && git switch {0}",
            pull_request.head_branch
        ),
    }
}

pub fn merge_method_label(method: MergeMethod) -> String {
    match method {
        MergeMethod::Merge => m::merge_method_merge(),
        MergeMethod::Squash => m::merge_method_squash(),
        MergeMethod::Rebase => m::merge_method_rebase(),
    }
}

/// Why the pull request can't be merged now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeBlocker {
    Permission,
    Draft,
    Conflicts,
}

impl MergeBlocker {
    pub fn label(self) -> String {
        match self {
            Self::Permission => m::merge_blocked_permission(),
            Self::Draft => m::merge_blocked_draft(),
            Self::Conflicts => m::merge_blocked_conflicts(),
        }
    }
}

pub fn merge_blocker(
    pull_request: &PullRequest,
    detail: &PullRequestDetail,
) -> Option<MergeBlocker> {
    if !detail.can_edit {
        Some(MergeBlocker::Permission)
    } else if pull_request.state == PullRequestState::Draft {
        Some(MergeBlocker::Draft)
    } else if detail.mergeability == Mergeability::Conflicting {
        Some(MergeBlocker::Conflicts)
    } else {
        None
    }
}

/// The merge menu shows while the pull request can still be merged some way.
pub fn shows_merge(pull_request: &PullRequest, detail: &PullRequestDetail) -> bool {
    !matches!(
        pull_request.state,
        PullRequestState::Merged | PullRequestState::Closed
    ) && !detail.merge_methods.is_empty()
}

/// "ana, bo requested changes", when someone did.
pub fn changes_requested_warning(detail: &PullRequestDetail) -> Option<String> {
    let names: Vec<&str> = detail
        .reviewers
        .iter()
        .filter(|reviewer| reviewer.state == ReviewState::ChangesRequested)
        .map(|reviewer| reviewer.name.as_str())
        .collect();
    (!names.is_empty()).then(|| m::merge_changes_requested(names.join(", ")))
}

/// The merge confirmation's text, after the warning when there is one.
pub fn merge_question(
    pull_request: &PullRequest,
    method: MergeMethod,
    warning: Option<&str>,
) -> String {
    let question = m::merge_confirm(
        &pull_request.base_branch,
        merge_method_label(method),
        pull_request.number,
    );
    match warning {
        Some(warning) => format!("{warning}. {question}"),
        None => question,
    }
}

/// What the "…" menu offers beyond opening and copying.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateActions {
    /// Ready for review or back to draft.
    pub toggle_draft: bool,
    /// Close when open, reopen when closed.
    pub close_or_reopen: bool,
    pub open: bool,
}

pub fn state_actions(
    pull_request: &PullRequest,
    detail: Option<&PullRequestDetail>,
) -> StateActions {
    let open = matches!(
        pull_request.state,
        PullRequestState::Open | PullRequestState::Draft
    );
    let can_edit = detail.is_some_and(|detail| detail.can_edit)
        && pull_request.state != PullRequestState::Merged;
    if !can_edit {
        return StateActions {
            open,
            ..StateActions::default()
        };
    }
    StateActions {
        toggle_draft: open && detail.is_some_and(|detail| detail.supports.draft),
        close_or_reopen: open || detail.is_some_and(|detail| detail.supports.reopen),
        open,
    }
}

/// Reviewers, labels and the description can change.
pub fn editable(pull_request: &PullRequest, detail: &PullRequestDetail) -> bool {
    detail.can_edit && pull_request.state != PullRequestState::Merged
}

/// Comments can be added while the pull request is open.
pub fn can_comment(pull_request: &PullRequest) -> bool {
    matches!(
        pull_request.state,
        PullRequestState::Open | PullRequestState::Draft
    )
}

/// The candidates a review can be asked of: not the author, not who is asked already.
pub fn reviewer_choices(
    candidates: &[String],
    author: Option<&str>,
    current: &[&str],
) -> Vec<String> {
    candidates
        .iter()
        .filter(|login| Some(login.as_str()) != author && !current.contains(&login.as_str()))
        .cloned()
        .collect()
}

/// The labels whose name holds `query`, ignoring case.
pub fn matching_labels<'a>(
    labels: &'a [PullRequestLabel],
    query: &str,
) -> Vec<&'a PullRequestLabel> {
    let needle = query.trim().to_lowercase();
    labels
        .iter()
        .filter(|label| label.name.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MergeBlocker, StateActions, can_comment, changes_requested_warning, checkout_command,
        editable, forge_error_message, matching_labels, merge_blocker, merge_question,
        reviewer_choices, shows_merge, state_actions,
    };
    use hoshi_core::{
        ChecksState, CommandError, ForgeKind, MergeMethod, Mergeability, PullRequest,
        PullRequestDetail, PullRequestLabel, PullRequestState, PullRequestSupport, ReviewState,
        Reviewer,
    };

    fn pull_request(state: PullRequestState) -> PullRequest {
        PullRequest {
            forge: ForgeKind::Github,
            number: 7,
            title: "t".into(),
            url: "u".into(),
            author: Some("me".into()),
            head_branch: "feature/x".into(),
            head_sha: None,
            base_branch: "main".into(),
            state,
            checks: Some(ChecksState::Passing),
            unresolved_comments: 0,
            open_tasks: 0,
            changes_requested: false,
            turn: None,
            conflicts: false,
            additions: 0,
            deletions: 0,
            updated_at: "2026-11-12T10:00:00Z".into(),
        }
    }

    fn detail() -> PullRequestDetail {
        PullRequestDetail {
            supports: PullRequestSupport::all(),
            forge_id: "id".into(),
            repository: "a/b".into(),
            mergeability: Mergeability::Mergeable,
            merge_methods: vec![MergeMethod::Squash],
            can_edit: true,
            body: String::new(),
            changed_files: 0,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: Vec::new(),
            review_threads: Vec::new(),
            timeline: Vec::new(),
        }
    }

    #[test]
    fn merging_is_blocked_by_permission_then_draft_then_conflicts() {
        let open = pull_request(PullRequestState::Open);
        assert_eq!(merge_blocker(&open, &detail()), None);
        let conflicting = PullRequestDetail {
            mergeability: Mergeability::Conflicting,
            ..detail()
        };
        assert_eq!(
            merge_blocker(&open, &conflicting),
            Some(MergeBlocker::Conflicts)
        );
        let draft = pull_request(PullRequestState::Draft);
        assert_eq!(
            merge_blocker(&draft, &conflicting),
            Some(MergeBlocker::Draft)
        );
        let read_only = PullRequestDetail {
            can_edit: false,
            ..conflicting
        };
        assert_eq!(
            merge_blocker(&draft, &read_only),
            Some(MergeBlocker::Permission)
        );
    }

    #[test]
    fn the_merge_menu_hides_once_finished_or_without_methods() {
        assert!(shows_merge(
            &pull_request(PullRequestState::Open),
            &detail()
        ));
        assert!(shows_merge(
            &pull_request(PullRequestState::Draft),
            &detail()
        ));
        assert!(!shows_merge(
            &pull_request(PullRequestState::Merged),
            &detail()
        ));
        assert!(!shows_merge(
            &pull_request(PullRequestState::Closed),
            &detail()
        ));
        let none = PullRequestDetail {
            merge_methods: Vec::new(),
            ..detail()
        };
        assert!(!shows_merge(&pull_request(PullRequestState::Open), &none));
    }

    #[test]
    fn requested_changes_warn_before_the_question() {
        let mut detail = detail();
        let open = pull_request(PullRequestState::Open);
        assert_eq!(changes_requested_warning(&detail), None);
        assert_eq!(
            merge_question(&open, MergeMethod::Squash, None),
            "Merge #7 into main (Squash and merge)?"
        );
        detail.reviewers = ["ana", "bo", "cy"]
            .into_iter()
            .zip([
                ReviewState::ChangesRequested,
                ReviewState::Approved,
                ReviewState::ChangesRequested,
            ])
            .map(|(name, state)| Reviewer {
                name: name.into(),
                state,
            })
            .collect();
        let warning = changes_requested_warning(&detail).unwrap();
        assert_eq!(warning, "ana, cy requested changes");
        assert_eq!(
            merge_question(&open, MergeMethod::Rebase, Some(&warning)),
            "ana, cy requested changes. Merge #7 into main (Rebase and merge)?"
        );
    }

    #[test]
    fn the_menu_offers_state_changes_only_to_editors() {
        let open = pull_request(PullRequestState::Open);
        assert_eq!(
            state_actions(&open, Some(&detail())),
            StateActions {
                toggle_draft: true,
                close_or_reopen: true,
                open: true,
            }
        );
        assert_eq!(
            state_actions(&open, None),
            StateActions {
                open: true,
                ..StateActions::default()
            }
        );
        let closed = pull_request(PullRequestState::Closed);
        let bitbucket = PullRequestDetail {
            supports: PullRequestSupport {
                labels: false,
                draft: false,
                reopen: false,
            },
            ..detail()
        };
        assert_eq!(
            state_actions(&closed, Some(&bitbucket)),
            StateActions::default()
        );
        assert!(state_actions(&closed, Some(&detail())).close_or_reopen);
        let merged = pull_request(PullRequestState::Merged);
        assert_eq!(
            state_actions(&merged, Some(&detail())),
            StateActions::default()
        );
        assert!(!editable(&merged, &detail()));
        assert!(editable(&open, &detail()));
        assert!(can_comment(&pull_request(PullRequestState::Draft)));
        assert!(!can_comment(&closed));
    }

    #[test]
    fn checkout_follows_the_forge() {
        let mut open = pull_request(PullRequestState::Open);
        assert_eq!(checkout_command(&open), "gh pr checkout 7");
        open.forge = ForgeKind::Bitbucket;
        assert_eq!(
            checkout_command(&open),
            "git fetch origin feature/x && git switch feature/x"
        );
    }

    #[test]
    fn errors_read_as_the_web_shows_them() {
        assert_eq!(
            forge_error_message(&CommandError::ForgeRejected {
                detail: "Base branch was modified".into()
            }),
            "Base branch was modified"
        );
        assert_eq!(
            forge_error_message(&CommandError::ForgeUnreachable {
                detail: "dns".into()
            }),
            "Couldn't reach the forge. Check your connection."
        );
        assert_eq!(
            forge_error_message(&CommandError::ForgeUnavailable),
            "This pull request isn't available right now."
        );
    }

    #[test]
    fn reviewers_leave_out_the_author_and_who_is_asked() {
        let candidates = ["me", "ana", "bo"].map(String::from);
        assert_eq!(reviewer_choices(&candidates, Some("me"), &["bo"]), ["ana"]);
        let labels = ["Bug", "ui", "debug"].map(|name| PullRequestLabel {
            name: name.into(),
            color: "fff000".into(),
        });
        let names = |query| {
            matching_labels(&labels, query)
                .into_iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(" BUG "), ["Bug", "debug"]);
        assert_eq!(names(""), ["Bug", "ui", "debug"]);
    }
}
