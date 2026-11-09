//! The privilege scopes of an Atlassian API token, as Bitbucket reports them in
//! `x-oauth-scopes` and in the refusals of requests that lack one.

use serde_json::Value;
use std::collections::BTreeSet;

/// What hoshi reads and does on Bitbucket.
pub const REQUIRED_SCOPES: [&str; 5] = [
    "read:user:bitbucket",
    "read:repository:bitbucket",
    "read:pullrequest:bitbucket",
    "write:pullrequest:bitbucket",
    "read:workspace:bitbucket",
];

const SUFFIX: &str = ":bitbucket";

/// A scope in the API token's form, `read:pullrequest:bitbucket`, followed by what it also
/// grants. Older credentials name them `pullrequest:write`, which grants reading too, and
/// the repository with it.
fn canonical(scope: &str) -> Vec<String> {
    let scope = scope.trim().trim_matches(['"', '\'']);
    if scope.is_empty() {
        return Vec::new();
    }
    if scope.ends_with(SUFFIX) {
        return vec![scope.to_owned()];
    }
    let new = |names: &[&str]| names.iter().map(|name| format!("{name}{SUFFIX}")).collect();
    match scope {
        "account" => new(&["read:user"]),
        "account:write" => new(&["write:user", "read:user"]),
        "team" | "workspace" => new(&["read:workspace"]),
        "team:write" | "workspace:write" => new(&["write:workspace", "read:workspace"]),
        "repository" => new(&["read:repository"]),
        "repository:write" => new(&["write:repository", "read:repository"]),
        "repository:admin" => new(&["admin:repository", "read:repository"]),
        "pullrequest" => new(&["read:pullrequest", "read:repository"]),
        "pullrequest:write" => new(&[
            "write:pullrequest",
            "read:pullrequest",
            "read:repository",
            "write:repository",
        ]),
        _ if ["read:", "write:", "admin:", "delete:"]
            .iter()
            .any(|verb| scope.starts_with(verb))
            && scope.matches(':').count() == 1 =>
        {
            vec![format!("{scope}{SUFFIX}")]
        }
        _ => Vec::new(),
    }
}

/// Every scope `scopes` grant.
fn parse<'a>(scopes: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    scopes.into_iter().flat_map(canonical).collect()
}

/// The scopes `scopes` name, without what they also grant.
fn parse_named<'a>(scopes: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    scopes
        .into_iter()
        .filter_map(|scope| canonical(scope).into_iter().next())
        .collect()
}

/// A header's scopes, separated by commas or spaces. `None` when it names none hoshi knows,
/// so it says nothing about them.
pub fn from_header(header: &str) -> Option<BTreeSet<String>> {
    let scopes = parse(header.split([',', ' ']));
    (!scopes.is_empty()).then_some(scopes)
}

/// The scopes `required` lists that `granted` lacks, in `required`'s order.
pub fn missing(granted: &BTreeSet<String>, required: &[&str]) -> Vec<String> {
    required
        .iter()
        .filter(|scope| !granted.contains(**scope))
        .map(|scope| (*scope).to_owned())
        .collect()
}

/// The scopes a 403 says the request lacked: the body's `error.detail.required` less what
/// it says is granted, or else the endpoint's accepted scopes less the token's.
pub fn lacking(body: &str, accepted: Option<&str>, granted: Option<&str>) -> Vec<String> {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let detail = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/detail"))
        .filter(|detail| detail.is_object());
    let list = |key: &str| {
        detail
            .and_then(|detail| detail.get(key))
            .and_then(Value::as_array)
            .map(|scopes| scopes.iter().filter_map(Value::as_str).collect::<Vec<_>>())
    };
    let (required, granted) = match list("required") {
        Some(required) => (
            parse_named(required),
            parse(list("granted").unwrap_or_default()),
        ),
        None => match (
            accepted
                .map(|header| parse_named(header.split([',', ' '])))
                .filter(|scopes| !scopes.is_empty()),
            granted.and_then(from_header),
        ) {
            (Some(accepted), Some(granted)) => (accepted, granted),
            _ => return Vec::new(),
        },
    };
    required.difference(&granted).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_new_and_old_scope_names() {
        let granted = from_header(
            "read:user:bitbucket, read:repository:bitbucket read:pullrequest:bitbucket",
        )
        .unwrap();
        assert_eq!(
            missing(&granted, &REQUIRED_SCOPES),
            ["write:pullrequest:bitbucket", "read:workspace:bitbucket"]
        );

        let old = from_header("account team pullrequest:write").unwrap();
        assert!(missing(&old, &REQUIRED_SCOPES).is_empty());

        assert_eq!(from_header("email something-else"), None);
        assert_eq!(from_header(""), None);
    }

    #[test]
    fn a_refusal_names_the_scopes_it_lacked() {
        let body = r#"{"type":"error","error":{"message":"Your credentials lack one or more required privilege scopes.","detail":{"granted":["read:user:bitbucket","read:pullrequest:bitbucket"],"required":["write:pullrequest:bitbucket"]}}}"#;
        assert_eq!(lacking(body, None, None), ["write:pullrequest:bitbucket"]);

        let old = r#"{"type":"error","error":{"message":"Your credentials lack one or more required privilege scopes.","detail":{"granted":["account","repository"],"required":["pullrequest:write"]}}}"#;
        assert_eq!(lacking(old, None, None), ["write:pullrequest:bitbucket"]);

        assert_eq!(
            lacking(
                r#"{"type":"error","error":{"message":"Forbidden"}}"#,
                Some("pullrequest:write"),
                Some(
                    "read:pullrequest:bitbucket, write:pullrequest:bitbucket, read:repository:bitbucket, write:repository:bitbucket"
                ),
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            lacking(
                "<html>",
                Some("write:pullrequest:bitbucket"),
                Some("read:user:bitbucket")
            ),
            ["write:pullrequest:bitbucket"]
        );
        assert!(lacking("<html>", None, None).is_empty());
    }
}
