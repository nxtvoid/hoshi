use hoshi_core::ForgeKind;

/// A repository on a forge, as named by a git remote.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryRef {
    pub forge: ForgeKind,
    pub host: String,
    pub owner: String,
    pub name: String,
}

/// Reads a remote URL in any form git accepts: `https://host/owner/repo.git`,
/// `git@host:owner/repo.git` or `ssh://git@host:22/owner/repo`. `None` for hosts no
/// forge adapter serves.
pub fn parse_remote(url: &str) -> Option<RepositoryRef> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?.to_ascii_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    let forge = match host.as_str() {
        "github.com" => ForgeKind::Github,
        "bitbucket.org" => ForgeKind::Bitbucket,
        _ => return None,
    };
    Some(RepositoryRef {
        forge,
        host,
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// A repository on `forge` named as `owner/name`, or by any URL [`parse_remote`] reads,
/// its web page included. `None` when it names nothing there.
pub fn parse_repository_name(forge: ForgeKind, input: &str) -> Option<RepositoryRef> {
    let input = input.trim();
    if input.contains("://") || input.contains('@') {
        return parse_remote(input).filter(|repository| repository.forge == forge);
    }
    let host = match forge {
        ForgeKind::Github => "github.com",
        ForgeKind::Bitbucket => "bitbucket.org",
    };
    let path = input.strip_prefix(host).unwrap_or(input);
    parse_remote(&format!("https://{host}/{}", path.trim_matches('/')))
}

/// The repository and number of a pull request's web page, such as
/// `https://github.com/owner/repo/pull/3` or
/// `https://bitbucket.org/workspace/repo/pull-requests/3`. `None` for any other link.
pub fn parse_pull_request_url(url: &str) -> Option<(RepositoryRef, u32)> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let path = rest.split(['?', '#']).next()?;
    let mut parts = path.split('/');
    let (host, owner, name) = (parts.next()?, parts.next()?, parts.next()?);
    let segment = parts.next()?;
    let number = parts.next()?.parse().ok()?;
    let repository = parse_remote(&format!("https://{host}/{owner}/{name}"))?;
    let expected = match repository.forge {
        ForgeKind::Github => "pull",
        ForgeKind::Bitbucket => "pull-requests",
    };
    (segment == expected).then_some((repository, number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github(owner: &str, name: &str) -> Option<RepositoryRef> {
        Some(RepositoryRef {
            forge: ForgeKind::Github,
            host: "github.com".into(),
            owner: owner.into(),
            name: name.into(),
        })
    }

    #[test]
    fn reads_every_url_form_git_accepts() {
        for url in [
            "https://github.com/nxtvoid/hoshi.git",
            "https://github.com/nxtvoid/hoshi",
            "https://user@github.com/nxtvoid/hoshi/",
            "git@github.com:nxtvoid/hoshi.git",
            "ssh://git@github.com/nxtvoid/hoshi.git",
            "ssh://git@GitHub.com:22/nxtvoid/hoshi",
        ] {
            assert_eq!(parse_remote(url), github("nxtvoid", "hoshi"), "{url}");
        }
    }

    #[test]
    fn reads_pull_request_links() {
        for url in [
            "https://github.com/nxtvoid/avzx/pull/3",
            "https://github.com/nxtvoid/avzx/pull/3/files",
            "https://github.com/nxtvoid/avzx/pull/3#issuecomment-1",
            "http://GitHub.com/nxtvoid/avzx/pull/3?w=1",
        ] {
            let expected = github("nxtvoid", "avzx").map(|repository| (repository, 3));
            assert_eq!(parse_pull_request_url(url), expected, "{url}");
        }
        for url in [
            "https://github.com/nxtvoid/avzx/issues/3",
            "https://github.com/nxtvoid/avzx/pull/x",
            "https://github.com/nxtvoid/avzx",
            "https://example.com/a/b/pull/3",
        ] {
            assert_eq!(parse_pull_request_url(url), None, "{url}");
        }
    }

    #[test]
    fn reads_bitbucket_remotes_and_links() {
        let expected = Some(RepositoryRef {
            forge: ForgeKind::Bitbucket,
            host: "bitbucket.org".into(),
            owner: "acme".into(),
            name: "web-app".into(),
        });
        for url in [
            "https://nxtvoid@bitbucket.org/acme/web-app.git",
            "git@bitbucket.org:acme/web-app.git",
            "ssh://git@bitbucket.org/acme/web-app",
        ] {
            assert_eq!(parse_remote(url), expected, "{url}");
        }
        assert_eq!(
            parse_pull_request_url("https://bitbucket.org/acme/web-app/pull-requests/12/diff"),
            expected.clone().map(|repository| (repository, 12))
        );
        assert_eq!(
            parse_pull_request_url("https://bitbucket.org/acme/web-app/pull/12"),
            None
        );
        assert_eq!(
            parse_pull_request_url("https://github.com/acme/web-app/pull-requests/12"),
            None
        );
    }

    #[test]
    fn reads_repository_names_typed_by_hand() {
        for input in [
            "nxtvoid/hoshi",
            " nxtvoid/hoshi/ ",
            "nxtvoid/hoshi.git",
            "github.com/nxtvoid/hoshi",
            "https://github.com/nxtvoid/hoshi",
            "git@github.com:nxtvoid/hoshi.git",
        ] {
            assert_eq!(
                parse_repository_name(ForgeKind::Github, input),
                github("nxtvoid", "hoshi"),
                "{input}"
            );
        }
        assert_eq!(
            parse_repository_name(ForgeKind::Bitbucket, "acme/web-app").map(|repo| repo.host),
            Some("bitbucket.org".into())
        );
        for input in [
            "hoshi",
            "a/b/c",
            "/hoshi",
            "https://bitbucket.org/acme/web-app",
        ] {
            assert_eq!(
                parse_repository_name(ForgeKind::Github, input),
                None,
                "{input}"
            );
        }
    }

    #[test]
    fn ignores_unknown_hosts_and_odd_paths() {
        assert_eq!(parse_remote("https://example.com/a/b.git"), None);
        assert_eq!(parse_remote("https://github.com/only-owner"), None);
        assert_eq!(parse_remote("https://github.com/a/b/c"), None);
        assert_eq!(parse_remote("/local/path/repo"), None);
    }
}
