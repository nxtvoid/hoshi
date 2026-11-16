//! CLI versions, as `x.y.z`.

pub(crate) type Version = (u32, u32, u32);

/// The first `x.y.z` in `text`, as `claude --version` prints it.
pub(crate) fn parse_version(text: &str) -> Option<Version> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|word| {
            let mut parts = word.split('.').map(str::parse::<u32>);
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => {
                    Some((major, minor, patch))
                }
                _ => None,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions() {
        assert_eq!(parse_version("2.1.318 (Claude Code)"), Some((2, 1, 318)));
        assert_eq!(parse_version("claude v10.0.1"), Some((10, 0, 1)));
        assert_eq!(parse_version("no version"), None);
    }
}
