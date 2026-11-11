//! Bitbucket Cloud's JSON, and how it maps onto hoshi-core's pull request types.

use hoshi_core::{
    ChecksState, DiffFile, DiffFileStatus, ForgeKind, PullRequest, PullRequestCheck,
    PullRequestState, ReviewComment, ReviewState, ReviewThread, Reviewer, TimelineItem,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub values: Vec<T>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub uuid: Option<String>,
    pub account_id: Option<String>,
    pub nickname: Option<String>,
    pub display_name: Option<String>,
}

impl User {
    /// App accounts have no nickname, only a display name.
    pub fn login(&self) -> Option<String> {
        self.nickname.clone().or_else(|| self.display_name.clone())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Link {
    pub href: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Links {
    pub html: Option<Link>,
}

impl Links {
    fn html(&self) -> Option<String> {
        self.html.as_ref().map(|link| link.href.clone())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitRef {
    pub hash: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Endpoint {
    pub branch: Branch,
    pub commit: Option<CommitRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Participant {
    pub user: Option<User>,
    pub role: Option<String>,
    #[serde(default)]
    pub approved: bool,
    pub state: Option<String>,
    pub participated_on: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPullRequest {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The description again, with Bitbucket's rendering.
    pub summary: Option<Content>,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    pub author: Option<User>,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub updated_on: String,
    #[serde(default)]
    pub links: Links,
    #[serde(default)]
    pub reviewers: Vec<User>,
    #[serde(default)]
    pub participants: Vec<Participant>,
}

impl RawPullRequest {
    pub fn state(&self) -> PullRequestState {
        match self.state.trim().to_ascii_uppercase().as_str() {
            "MERGED" => PullRequestState::Merged,
            "DECLINED" | "SUPERSEDED" => PullRequestState::Closed,
            _ if self.draft => PullRequestState::Draft,
            _ => PullRequestState::Open,
        }
    }

    pub fn head_commit(&self) -> Option<&str> {
        self.source
            .commit
            .as_ref()
            .map(|commit| commit.hash.as_str())
    }

    pub fn url(&self) -> String {
        self.links.html().unwrap_or_default()
    }

    /// The description as markdown hoshi can show.
    pub fn body(&self) -> String {
        match &self.summary {
            Some(summary) if summary.raw == self.description => summary.readable(),
            _ => self.description.clone(),
        }
    }
}

/// What a listing leaves out and other endpoints add: checks, open threads and line counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub checks: Option<ChecksState>,
    pub unresolved_comments: u32,
    pub additions: u32,
    pub deletions: u32,
}

pub fn pull_request(raw: &RawPullRequest, summary: Summary) -> PullRequest {
    PullRequest {
        forge: ForgeKind::Bitbucket,
        number: raw.id,
        title: raw.title.clone(),
        url: raw.url(),
        author: raw.author.as_ref().and_then(User::login),
        head_branch: raw.source.branch.name.clone(),
        head_sha: raw.head_commit().map(str::to_owned),
        base_branch: raw.destination.branch.name.clone(),
        state: raw.state(),
        checks: summary.checks,
        unresolved_comments: summary.unresolved_comments,
        additions: summary.additions,
        deletions: summary.deletions,
        updated_at: raw.updated_on.clone(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawStatus {
    pub key: Option<String>,
    pub name: Option<String>,
    pub state: String,
    pub url: Option<String>,
}

fn status_state(state: &str) -> ChecksState {
    match state {
        "SUCCESSFUL" => ChecksState::Passing,
        "INPROGRESS" => ChecksState::Pending,
        _ => ChecksState::Failing,
    }
}

/// One check per key; a rerun reuses its key and the later report wins.
pub fn checks(statuses: &[RawStatus]) -> Vec<PullRequestCheck> {
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, PullRequestCheck> = HashMap::new();
    for status in statuses {
        let name = status
            .name
            .clone()
            .or_else(|| status.key.clone())
            .unwrap_or_default();
        let key = status.key.clone().unwrap_or_else(|| name.clone());
        if !latest.contains_key(&key) {
            order.push(key.clone());
        }
        latest.insert(
            key,
            PullRequestCheck {
                name,
                state: status_state(&status.state),
                url: status.url.clone(),
            },
        );
    }
    order
        .into_iter()
        .filter_map(|key| latest.remove(&key))
        .collect()
}

pub fn checks_state(checks: &[PullRequestCheck]) -> Option<ChecksState> {
    if checks.is_empty() {
        return None;
    }
    let any = |state| checks.iter().any(|check| check.state == state);
    Some(if any(ChecksState::Failing) {
        ChecksState::Failing
    } else if any(ChecksState::Pending) {
        ChecksState::Pending
    } else {
        ChecksState::Passing
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct Content {
    #[serde(default)]
    pub raw: String,
    /// Bitbucket's rendering, the only place a mention's name appears.
    pub html: Option<String>,
}

impl Content {
    /// The markdown with mentions named and Bitbucket's attribute lists left out.
    pub fn readable(&self) -> String {
        let named = name_mentions(&self.raw, self.html.as_deref().unwrap_or_default());
        strip_attribute_lists(&named)
    }
}

/// `@{account_id}` as `@Display Name`, for the accounts the rendering names.
fn name_mentions(raw: &str, html: &str) -> String {
    const MARK: &str = "data-atlassian-id=\"";
    let mut names = HashMap::new();
    let mut rest = html;
    while let Some(start) = rest.find(MARK) {
        rest = &rest[start + MARK.len()..];
        let Some(end) = rest.find('"') else { break };
        let id = &rest[..end];
        let Some(text) = rest[end..]
            .split_once('>')
            .and_then(|(_, after)| after.split_once('<'))
            .map(|(text, _)| text)
        else {
            break;
        };
        if let Some(name) = text.strip_prefix('@').filter(|name| !name.is_empty()) {
            names.insert(format!("@{{{id}}}"), format!("@{}", unescape_html(name)));
        }
    }
    let mut text = raw.to_owned();
    for (mention, name) in names {
        text = text.replace(&mention, &name);
    }
    text
}

fn unescape_html(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// `@Display Name` or `@nickname` of one of `users` as the `@{account_id}` Bitbucket
/// notifies; code spans and blocks stay as written.
pub fn encode_mentions(text: &str, users: &[User]) -> String {
    let mut names: Vec<(String, &str)> = Vec::new();
    for user in users {
        let Some(id) = user.account_id.as_deref() else {
            continue;
        };
        for name in [&user.display_name, &user.nickname].into_iter().flatten() {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|(known, _)| known == &name.to_lowercase()) {
                names.push((name.to_lowercase(), id));
            }
        }
    }
    if names.is_empty() || !text.contains('@') {
        return text.to_owned();
    }
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.';
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    let mut previous: Option<char> = None;
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let c = rest.chars().next().unwrap_or_default();
        if c == '`' {
            let ticks = rest.chars().take_while(|tick| *tick == '`').count();
            out.push_str(&rest[..ticks]);
            index += ticks;
            in_code = !in_code;
            previous = Some('`');
            continue;
        }
        if c == '@' && !in_code && !previous.is_some_and(word) {
            let after = &rest[1..];
            let found = names.iter().find(|(name, _)| {
                after
                    .get(..name.len())
                    .is_some_and(|head| head.to_lowercase() == *name)
                    && !after[name.len()..]
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_alphanumeric() || next == '_')
            });
            if let Some((name, id)) = found {
                out.push_str(&format!("@{{{id}}}"));
                index += 1 + name.len();
                previous = Some('}');
                continue;
            }
        }
        out.push(c);
        index += c.len_utf8();
        previous = Some(c);
    }
    out
}

/// Drops the `{: data-layout='center' }` Bitbucket writes after images.
fn strip_attribute_lists(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("){: ") {
        let after = &rest[start + 1..];
        match after.find('}').filter(|end| !after[..*end].contains('\n')) {
            Some(end) => {
                out.push_str(&rest[..=start]);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..start + 4]);
                rest = &rest[start + 4..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct Inline {
    pub path: String,
    pub from: Option<u32>,
    pub to: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Parent {
    pub id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawComment {
    pub id: u64,
    pub content: Content,
    pub user: Option<User>,
    pub created_on: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub pending: bool,
    pub inline: Option<Inline>,
    pub parent: Option<Parent>,
    pub resolution: Option<serde_json::Value>,
    #[serde(default)]
    pub links: Links,
}

impl RawComment {
    fn shown(&self) -> bool {
        !self.deleted && !self.pending && !self.content.raw.trim().is_empty()
    }

    fn review_comment(&self, fallback_url: &str) -> ReviewComment {
        ReviewComment {
            author: self.user.as_ref().and_then(User::login),
            body: self.content.readable(),
            created_at: self.created_on.clone(),
            url: self.links.html().unwrap_or_else(|| fallback_url.to_owned()),
        }
    }
}

/// The first comment of each reply chain, found through the parents of every comment,
/// shown or not.
fn roots(comments: &[RawComment]) -> HashMap<u64, u64> {
    let parent: HashMap<u64, Option<u64>> = comments
        .iter()
        .map(|comment| (comment.id, comment.parent.as_ref().map(|parent| parent.id)))
        .collect();
    comments
        .iter()
        .map(|comment| {
            let mut root = comment.id;
            for _ in 0..comments.len() {
                match parent.get(&root).copied().flatten() {
                    Some(up) if parent.contains_key(&up) => root = up,
                    _ => break,
                }
            }
            (comment.id, root)
        })
        .collect()
}

/// Comments on a line of code, one thread per reply chain, oldest first.
pub fn review_threads(comments: &[RawComment], pull_request_url: &str) -> Vec<ReviewThread> {
    let roots = roots(comments);
    let by_id: HashMap<u64, &RawComment> = comments.iter().map(|each| (each.id, each)).collect();
    let mut order: Vec<u64> = Vec::new();
    let mut threads: HashMap<u64, Vec<&RawComment>> = HashMap::new();
    for comment in comments {
        let root = roots[&comment.id];
        let Some(head) = by_id.get(&root) else {
            continue;
        };
        if head.inline.is_none() || !comment.shown() {
            continue;
        }
        threads
            .entry(root)
            .or_insert_with(|| {
                order.push(root);
                Vec::new()
            })
            .push(comment);
    }
    order
        .into_iter()
        .map(|root| {
            let head = by_id[&root];
            let inline = head.inline.as_ref();
            let mut chain = threads.remove(&root).unwrap_or_default();
            chain.sort_by(|a, b| a.created_on.cmp(&b.created_on));
            ReviewThread {
                id: root.to_string(),
                path: inline.map(|inline| inline.path.clone()).unwrap_or_default(),
                line: inline.and_then(|inline| inline.to.or(inline.from)),
                resolved: head
                    .resolution
                    .as_ref()
                    .is_some_and(|value| !value.is_null()),
                comments: chain
                    .iter()
                    .map(|comment| comment.review_comment(pull_request_url))
                    .collect(),
            }
        })
        .collect()
}

pub fn unresolved_threads(comments: &[RawComment]) -> u32 {
    let threads = review_threads(comments, "");
    threads.iter().filter(|thread| !thread.resolved).count() as u32
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAuthor {
    pub raw: Option<String>,
    pub user: Option<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawCommit {
    pub hash: String,
    #[serde(default)]
    pub message: String,
    pub date: String,
    pub author: Option<CommitAuthor>,
    #[serde(default)]
    pub links: Links,
}

/// `Name <email>` without the address.
fn author_name(raw: &str) -> String {
    raw.split('<').next().unwrap_or(raw).trim().to_owned()
}

/// Comments outside code, votes and commits, oldest first.
pub fn timeline(
    pull_request: &RawPullRequest,
    comments: &[RawComment],
    commits: &[RawCommit],
) -> Vec<TimelineItem> {
    let url = pull_request.url();
    let roots = roots(comments);
    let by_id: HashMap<u64, &RawComment> = comments.iter().map(|each| (each.id, each)).collect();
    let mut items: Vec<TimelineItem> = comments
        .iter()
        .filter(|comment| comment.shown())
        .filter(|comment| {
            by_id
                .get(&roots[&comment.id])
                .is_none_or(|root| root.inline.is_none())
        })
        .map(|comment| {
            let review = comment.review_comment(&url);
            TimelineItem::Comment {
                author: review.author,
                body: review.body,
                created_at: review.created_at,
                url: review.url,
            }
        })
        .collect();
    items.extend(pull_request.participants.iter().filter_map(|participant| {
        let state = vote(participant)?;
        Some(TimelineItem::Review {
            author: participant.user.as_ref().and_then(User::login),
            state,
            body: String::new(),
            created_at: participant.participated_on.clone()?,
            url: url.clone(),
        })
    }));
    items.extend(commits.iter().map(|commit| {
        let author = commit.author.as_ref().and_then(|author| {
            author
                .user
                .as_ref()
                .and_then(User::login)
                .or_else(|| author.raw.as_deref().map(author_name))
        });
        TimelineItem::Commit {
            sha: commit.hash.chars().take(7).collect(),
            message: commit.message.lines().next().unwrap_or_default().to_owned(),
            author,
            created_at: commit.date.clone(),
            url: commit.links.html().unwrap_or_else(|| url.clone()),
        }
    }));
    items.sort_by(|a, b| created_at(a).cmp(created_at(b)));
    items
}

fn created_at(item: &TimelineItem) -> &str {
    match item {
        TimelineItem::Comment { created_at, .. }
        | TimelineItem::Review { created_at, .. }
        | TimelineItem::Commit { created_at, .. } => created_at,
    }
}

fn vote(participant: &Participant) -> Option<ReviewState> {
    if participant.approved {
        return Some(ReviewState::Approved);
    }
    match participant.state.as_deref() {
        Some("approved") => Some(ReviewState::Approved),
        Some("changes_requested") => Some(ReviewState::ChangesRequested),
        _ => None,
    }
}

/// Requested reviewers with their vote, then anyone else who voted.
pub fn reviewers(pull_request: &RawPullRequest) -> Vec<Reviewer> {
    let votes: HashMap<String, ReviewState> = pull_request
        .participants
        .iter()
        .filter_map(|participant| {
            Some((
                participant.user.as_ref().and_then(User::login)?,
                vote(participant)?,
            ))
        })
        .collect();
    let mut reviewers: Vec<Reviewer> = pull_request
        .reviewers
        .iter()
        .filter_map(User::login)
        .map(|name| Reviewer {
            state: votes.get(&name).copied().unwrap_or(ReviewState::Requested),
            name,
        })
        .collect();
    for participant in &pull_request.participants {
        let (Some(name), Some(state)) = (
            participant.user.as_ref().and_then(User::login),
            vote(participant),
        ) else {
            continue;
        };
        if participant.role.as_deref() != Some("REVIEWER")
            && !reviewers.iter().any(|reviewer| reviewer.name == name)
        {
            reviewers.push(Reviewer { name, state });
        }
    }
    reviewers
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiffPath {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawDiffStat {
    pub status: String,
    pub lines_added: Option<u32>,
    pub lines_removed: Option<u32>,
    pub old: Option<DiffPath>,
    pub new: Option<DiffPath>,
}

pub fn diff_files(stats: &[RawDiffStat]) -> Vec<DiffFile> {
    stats
        .iter()
        .filter_map(|stat| {
            let old = stat.old.as_ref().map(|old| old.path.clone());
            let path = stat
                .new
                .as_ref()
                .map(|new| new.path.clone())
                .or_else(|| old.clone())?;
            let status = match stat.status.as_str() {
                "added" => DiffFileStatus::Added,
                "removed" => DiffFileStatus::Deleted,
                "renamed" => DiffFileStatus::Renamed,
                _ => DiffFileStatus::Modified,
            };
            Some(DiffFile {
                old_path: (status == DiffFileStatus::Renamed).then_some(old).flatten(),
                path,
                status,
                additions: stat.lines_added,
                deletions: stat.lines_removed,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PULL_REQUEST: &str = r#"{
        "id": 12, "title": "Fix login", "description": "Body", "state": "OPEN", "draft": false,
        "author": {"uuid": "{a}", "nickname": "ana", "display_name": "Ana"},
        "source": {"branch": {"name": "fix/login"}, "commit": {"hash": "abc123"}},
        "destination": {"branch": {"name": "main"}},
        "updated_on": "2026-11-07T10:00:00.000000+00:00",
        "links": {"html": {"href": "https://bitbucket.org/acme/web/pull-requests/12"}},
        "reviewers": [{"uuid": "{b}", "nickname": "bo"}, {"uuid": "{c}", "display_name": "Cy Bot"}],
        "participants": [
            {"user": {"nickname": "bo"}, "role": "REVIEWER", "approved": true, "state": "approved", "participated_on": "2026-11-07T09:00:00+00:00"},
            {"user": {"nickname": "dee"}, "role": "PARTICIPANT", "approved": false, "state": "changes_requested", "participated_on": "2026-11-07T09:30:00+00:00"},
            {"user": {"nickname": "eve"}, "role": "PARTICIPANT", "approved": false, "state": null, "participated_on": null}
        ]
    }"#;

    fn raw() -> RawPullRequest {
        serde_json::from_str(PULL_REQUEST).unwrap()
    }

    fn comment(id: u64, parent: Option<u64>, inline: bool, created: &str) -> RawComment {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "content": {"raw": format!("comment {id}")},
            "user": {"nickname": "bo"},
            "created_on": created,
            "inline": inline.then(|| serde_json::json!({"path": "src/a.rs", "from": null, "to": 4})),
            "parent": parent.map(|id| serde_json::json!({"id": id})),
            "resolution": null
        }))
        .unwrap()
    }

    #[test]
    fn maps_a_pull_request() {
        let pull_request = pull_request(&raw(), Summary::default());
        assert_eq!(pull_request.number, 12);
        assert_eq!(pull_request.author.as_deref(), Some("ana"));
        assert_eq!(pull_request.head_branch, "fix/login");
        assert_eq!(pull_request.base_branch, "main");
        assert_eq!(pull_request.state, PullRequestState::Open);
        assert_eq!(
            pull_request.url,
            "https://bitbucket.org/acme/web/pull-requests/12"
        );
    }

    #[test]
    fn reads_every_state() {
        let mut raw = raw();
        for (state, draft, expected) in [
            ("OPEN", true, PullRequestState::Draft),
            ("MERGED", false, PullRequestState::Merged),
            ("DECLINED", false, PullRequestState::Closed),
            ("SUPERSEDED", false, PullRequestState::Closed),
        ] {
            raw.state = state.into();
            raw.draft = draft;
            assert_eq!(raw.state(), expected, "{state}");
        }
    }

    #[test]
    fn reviewers_carry_their_votes() {
        let reviewers = reviewers(&raw());
        let pairs: Vec<_> = reviewers
            .iter()
            .map(|reviewer| (reviewer.name.as_str(), reviewer.state))
            .collect();
        assert_eq!(
            pairs,
            [
                ("bo", ReviewState::Approved),
                ("Cy Bot", ReviewState::Requested),
                ("dee", ReviewState::ChangesRequested),
            ]
        );
    }

    #[test]
    fn later_reruns_of_a_check_win() {
        let statuses: Vec<RawStatus> = serde_json::from_str(
            r#"[
                {"key": "build", "name": "Build", "state": "FAILED", "url": null},
                {"key": "lint", "name": null, "state": "SUCCESSFUL", "url": null},
                {"key": "build", "name": "Build", "state": "INPROGRESS", "url": "https://ci/1"}
            ]"#,
        )
        .unwrap();
        let checks = checks(&statuses);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].state, ChecksState::Pending);
        assert_eq!(checks[1].name, "lint");
        assert_eq!(checks_state(&checks), Some(ChecksState::Pending));
        assert_eq!(checks_state(&[]), None);
    }

    #[test]
    fn groups_inline_comments_into_threads() {
        let mut resolved = comment(5, None, true, "2026-11-07T08:00:00+00:00");
        resolved.resolution = Some(serde_json::json!({"type": "comment_resolution"}));
        let comments = vec![
            comment(1, None, true, "2026-11-07T07:00:00+00:00"),
            comment(3, Some(2), true, "2026-11-07T07:20:00+00:00"),
            comment(2, Some(1), true, "2026-11-07T07:10:00+00:00"),
            comment(4, None, false, "2026-11-07T07:30:00+00:00"),
            resolved,
        ];
        let threads = review_threads(&comments, "");
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "1");
        assert_eq!(threads[0].line, Some(4));
        let bodies: Vec<_> = threads[0]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(bodies, ["comment 1", "comment 2", "comment 3"]);
        assert!(threads[1].resolved);
        assert_eq!(unresolved_threads(&comments), 1);
    }

    #[test]
    fn the_timeline_has_general_comments_votes_and_commits_in_order() {
        let commits: Vec<RawCommit> = serde_json::from_str(
            r#"[{"hash": "abcdef123", "message": "Fix\n\nmore", "date": "2026-11-07T08:30:00+00:00",
                 "author": {"raw": "Ana <ana@example.com>"}}]"#,
        )
        .unwrap();
        let comments = vec![
            comment(1, None, true, "2026-11-07T07:00:00+00:00"),
            comment(4, None, false, "2026-11-07T10:30:00+00:00"),
        ];
        let items = timeline(&raw(), &comments, &commits);
        let kinds: Vec<_> = items
            .iter()
            .map(|item| match item {
                TimelineItem::Comment { .. } => "comment",
                TimelineItem::Review { .. } => "review",
                TimelineItem::Commit { .. } => "commit",
            })
            .collect();
        assert_eq!(kinds, ["commit", "review", "review", "comment"]);
        let TimelineItem::Commit {
            sha,
            message,
            author,
            ..
        } = &items[0]
        else {
            panic!("expected a commit");
        };
        assert_eq!((sha.as_str(), message.as_str()), ("abcdef1", "Fix"));
        assert_eq!(author.as_deref(), Some("Ana"));
    }

    #[test]
    fn reads_the_diffstat() {
        let stats: Vec<RawDiffStat> = serde_json::from_str(
            r#"[
                {"status": "added", "lines_added": 3, "lines_removed": 0, "old": null, "new": {"path": "a.rs"}},
                {"status": "removed", "lines_added": 0, "lines_removed": 2, "old": {"path": "b.rs"}, "new": null},
                {"status": "renamed", "lines_added": 1, "lines_removed": 1, "old": {"path": "c.rs"}, "new": {"path": "d.rs"}}
            ]"#,
        )
        .unwrap();
        let files = diff_files(&stats);
        assert_eq!(files[0].status, DiffFileStatus::Added);
        assert_eq!(files[1].path, "b.rs");
        assert_eq!(files[1].status, DiffFileStatus::Deleted);
        assert_eq!(files[2].old_path.as_deref(), Some("c.rs"));
    }

    #[test]
    fn mentions_read_as_the_names_bitbucket_renders() {
        let content: Content = serde_json::from_value(serde_json::json!({
            "raw": "@{70121:32bd} ready to review, cc @{1:unknown}",
            "html": "<p><span class=\"ap-mention\" data-atlassian-id=\"70121:32bd\">@Ana O&#39;Neil</span> ready to review, cc @{1:unknown}</p>"
        }))
        .unwrap();

        assert_eq!(
            content.readable(),
            "@Ana O'Neil ready to review, cc @{1:unknown}"
        );
    }

    #[test]
    fn images_lose_the_attribute_list_bitbucket_adds() {
        let content: Content = serde_json::from_value(serde_json::json!({
            "raw": "error:
![](https://bitbucket.org/x/images/1-image.png){: data-layout='center' }
then `f(){:x}`"
        }))
        .unwrap();

        assert_eq!(
            content.readable(),
            "error:
![](https://bitbucket.org/x/images/1-image.png)
then `f(){:x}`"
        );
    }

    #[test]
    fn the_description_names_its_mentions() {
        let mut pull_request = raw();
        pull_request.description = "for @{7:a}".into();
        pull_request.summary = Some(Content {
            raw: "for @{7:a}".into(),
            html: Some("<p>for <span data-atlassian-id=\"7:a\">@Bo</span></p>".into()),
        });

        assert_eq!(pull_request.body(), "for @Bo");
    }

    fn account(id: &str, display_name: &str, nickname: Option<&str>) -> User {
        User {
            uuid: None,
            account_id: Some(id.into()),
            nickname: nickname.map(Into::into),
            display_name: Some(display_name.into()),
        }
    }

    #[test]
    fn names_written_after_an_at_become_mentions_bitbucket_notifies() {
        let users = [
            account("1:a", "Gabriel Jara", Some("gjara")),
            account("2:b", "Gabriel", None),
        ];

        assert_eq!(
            encode_mentions("@Gabriel Jara - ready, cc @gjara, @gabriel.", &users),
            "@{1:a} - ready, cc @{1:a}, @{2:b}."
        );
    }

    #[test]
    fn mentions_skip_code_emails_and_unknown_names() {
        let users = [account("1:a", "Gabriel Jara", None)];

        assert_eq!(
            encode_mentions(
                "`@Gabriel Jara` me@Gabriel Jara.com @Gabriel Jaramillo @Nobody",
                &users
            ),
            "`@Gabriel Jara` me@Gabriel Jara.com @Gabriel Jaramillo @Nobody"
        );
    }
}
