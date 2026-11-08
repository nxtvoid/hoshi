//! Talks to the real GitHub API with the GitHub CLI's credentials.
//! Run with `cargo test -p hoshi-forge -- --ignored`.

use hoshi_core::PullRequestState;
use hoshi_forge::{Forge, GitHub, parse_remote};

#[test]
#[ignore = "needs network and `gh auth login`"]
fn reads_a_public_pull_request_by_branch() {
    let github = GitHub::with_cli_credentials();
    let repository = parse_remote("https://github.com/rust-lang/rust.git").unwrap();

    assert!(!github.viewer().unwrap().is_empty());
    assert_eq!(
        github.pull_request_for_branch(&repository, "main").unwrap(),
        None
    );
    let merged = github
        .pull_request_for_branch(&repository, "rollup-ad1nzsi")
        .unwrap();
    if let Some(pull_request) = merged {
        assert_eq!(pull_request.state, PullRequestState::Merged);
    }
}

#[test]
#[ignore = "needs network and `gh auth login`"]
fn reads_the_checks_and_review_threads_of_a_public_pull_request() {
    let github = GitHub::with_cli_credentials();
    let repository = parse_remote("https://github.com/cli/cli.git").unwrap();

    let detail = github.pull_request_detail(&repository, 9000).unwrap();

    assert!(detail.checks.iter().all(|check| !check.name.is_empty()));
    assert_eq!(detail.repository, "cli/cli");
    assert!(!detail.timeline.is_empty());
    assert!(!detail.forge_id.is_empty());
    assert!(!detail.merge_methods.is_empty());
    assert!(!github.labels(&repository).unwrap().is_empty());
    let files = github.pull_request_files(&repository, 9000).unwrap();
    assert_eq!(files.len() as u32, detail.changed_files.min(100));
}

#[test]
#[ignore = "needs network and `gh auth login`"]
fn searches_pull_requests_by_number_and_words() {
    let github = GitHub::with_cli_credentials();
    let repository = parse_remote("https://github.com/cli/cli.git").unwrap();

    let by_number = github.search_pull_requests(&repository, "9000").unwrap();
    let recent = github.search_pull_requests(&repository, "").unwrap();

    assert_eq!(by_number.first().map(|found| found.number), Some(9000));
    assert!(!recent.is_empty());
}
