//! Forge adapters. Each one reads pull requests from one code host and returns them in
//! hoshi-core's shape, so nothing above this crate knows which host it talks to.
//! One module per forge; no forge module imports another.

mod bitbucket;
mod github;
mod remote;

pub use bitbucket::{Bitbucket, BitbucketCredentials};
pub use github::GitHub;
use hoshi_core::{
    DiffFile, ForgeKind, PullRequest, PullRequestAction, PullRequestDetail, PullRequestLabel,
    RemoteRepository,
};
pub use remote::{RepositoryRef, parse_pull_request_url, parse_remote, parse_repository_name};

/// A file a pull request changes, with its patch when the forge sends one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestFile {
    pub file: DiffFile,
    /// Hunks only, without file headers. `None` for binaries and very large diffs.
    pub patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    #[error("not signed in to the forge")]
    Unauthorized,
    #[error("rate limited for {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("repository not found or not visible")]
    NotFound,
    /// The forge understood the request and refused it, with its reason.
    #[error("refused: {0}")]
    Rejected(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Unexpected(String),
}

/// A pull request to open, from `head` into `base`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub draft: bool,
}

pub trait Forge: Send + Sync {
    fn kind(&self) -> ForgeKind;

    /// The signed-in account's login.
    fn viewer(&self) -> Result<String, ForgeError>;

    /// Drops credentials read earlier, so the next request reads them again.
    fn forget_credentials(&self) {}

    /// The most recently updated pull request whose head is `branch`. `None` when there is
    /// none, or when `branch` is the repository's default branch.
    fn pull_request_for_branch(
        &self,
        repository: &RepositoryRef,
        branch: &str,
    ) -> Result<Option<PullRequest>, ForgeError>;

    /// The pull request numbered `number`.
    fn pull_request(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequest, ForgeError>;

    /// Pull requests whose number, title or branch match `query`, most recently updated
    /// first; with an empty query, the most recently updated ones.
    fn search_pull_requests(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<PullRequest>, ForgeError>;

    /// Description, reviewers, labels, checks, review threads and timeline.
    fn pull_request_detail(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<PullRequestDetail, ForgeError>;

    /// The files a pull request changes, with their patches.
    fn pull_request_files(
        &self,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Vec<PullRequestFile>, ForgeError>;

    /// Makes a change the user asked for. `forge_id` is [`PullRequestDetail::forge_id`].
    fn act(
        &self,
        repository: &RepositoryRef,
        number: u32,
        forge_id: &str,
        action: &PullRequestAction,
    ) -> Result<(), ForgeError>;

    /// Opens a pull request; `head` must be pushed already.
    fn create_pull_request(
        &self,
        repository: &RepositoryRef,
        pull_request: &NewPullRequest,
    ) -> Result<PullRequest, ForgeError>;

    /// Accounts that can be asked to review, matching `query`.
    fn reviewer_candidates(
        &self,
        repository: &RepositoryRef,
        query: &str,
    ) -> Result<Vec<String>, ForgeError>;

    /// Every label the repository defines.
    fn labels(&self, repository: &RepositoryRef) -> Result<Vec<PullRequestLabel>, ForgeError>;

    /// The repository's name, description and clone URL.
    fn repository(&self, repository: &RepositoryRef) -> Result<RemoteRepository, ForgeError>;
}
