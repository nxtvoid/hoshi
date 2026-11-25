use hoshi_core::{AppRelease, ProviderUpdate, ProviderVersion};
use std::path::PathBuf;
use std::time::Duration;

/// A check is due on focus once the last one is this old.
pub const RECHECK_ON_FOCUS_AFTER: Duration = Duration::from_secs(2 * 60);

/// An update the sidebar's button points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateBadge {
    Available,
    Running,
    Ready,
    Failed,
    Done,
}

impl UpdateBadge {
    fn weight(self) -> u8 {
        match self {
            Self::Running => 5,
            Self::Ready => 4,
            Self::Failed => 3,
            Self::Done => 2,
            Self::Available => 1,
        }
    }

    pub fn strongest(self, other: Self) -> Self {
        if other.weight() > self.weight() {
            other
        } else {
            self
        }
    }
}

/// hoshi's own update, from its release feed. A release without an installer for this machine
/// is downloaded from its release page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppUpdate {
    Idle,
    Checking,
    UpToDate {
        version: String,
    },
    CheckFailed {
        message: String,
    },
    Available {
        release: AppRelease,
    },
    /// `percent` is `None` while the size is unknown.
    Downloading {
        release: AppRelease,
        percent: Option<u8>,
    },
    /// Downloaded and verified; restarting installs it.
    Ready {
        release: AppRelease,
        installer: PathBuf,
    },
    Failed {
        release: AppRelease,
    },
}

impl AppUpdate {
    pub fn release(&self) -> Option<&AppRelease> {
        match self {
            Self::Available { release }
            | Self::Downloading { release, .. }
            | Self::Ready { release, .. }
            | Self::Failed { release } => Some(release),
            _ => None,
        }
    }

    /// A download or install is under way, which a check must not replace.
    pub fn busy(&self) -> bool {
        matches!(self, Self::Downloading { .. } | Self::Ready { .. })
    }
}

/// What the sidebar's update button does: check the feed, or open the update details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateButton {
    Off,
    Check { checking: bool },
    UpToDate { version: String },
    CheckFailed { message: String },
    Details { badge: UpdateBadge },
}

pub fn update_button(
    app: &AppUpdate,
    provider: Option<UpdateBadge>,
    enabled: bool,
) -> UpdateButton {
    if let Some(badge) = strongest_badge(app_badge(app), provider) {
        return UpdateButton::Details { badge };
    }
    if !enabled {
        return UpdateButton::Off;
    }
    match app {
        AppUpdate::Checking => UpdateButton::Check { checking: true },
        AppUpdate::UpToDate { version } => UpdateButton::UpToDate {
            version: version.clone(),
        },
        AppUpdate::CheckFailed { message } => UpdateButton::CheckFailed {
            message: message.clone(),
        },
        _ => UpdateButton::Check { checking: false },
    }
}

pub fn check_due(last_checked: Option<Duration>) -> bool {
    last_checked.is_none_or(|elapsed| elapsed >= RECHECK_ON_FOCUS_AFTER)
}

/// An update hoshi can't install isn't pointed at. `seen`: the user already looked at how the last provider update ended.
pub fn provider_badge(version: Option<&ProviderVersion>, seen: bool) -> Option<UpdateBadge> {
    let version = version?;
    match version.update {
        ProviderUpdate::Running { .. } => Some(UpdateBadge::Running),
        ProviderUpdate::Failed { .. } | ProviderUpdate::Unchanged if !seen => {
            Some(UpdateBadge::Failed)
        }
        ProviderUpdate::Succeeded if !seen => Some(UpdateBadge::Done),
        _ => (version.update_available && version.update_command.is_some())
            .then_some(UpdateBadge::Available),
    }
}

pub fn app_badge(update: &AppUpdate) -> Option<UpdateBadge> {
    match update {
        AppUpdate::Available { .. } => Some(UpdateBadge::Available),
        AppUpdate::Downloading { .. } => Some(UpdateBadge::Running),
        AppUpdate::Ready { .. } => Some(UpdateBadge::Ready),
        AppUpdate::Failed { .. } => Some(UpdateBadge::Failed),
        _ => None,
    }
}

/// The badge that says the most, so a running update is never hidden.
pub fn strongest_badge(a: Option<UpdateBadge>, b: Option<UpdateBadge>) -> Option<UpdateBadge> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.strongest(b)),
        (a, b) => a.or(b),
    }
}

/// An update ended since `previous`, so its outcome is worth showing again.
pub fn update_ended(previous: Option<&ProviderUpdate>, next: &ProviderUpdate) -> bool {
    matches!(previous, Some(ProviderUpdate::Running { .. }))
        && !matches!(next, ProviderUpdate::Running { .. })
}

/// Where a check of the release feed leaves the update; `manual` checks show how they ended.
pub fn after_check(
    current: &AppUpdate,
    found: Result<Option<AppRelease>, String>,
    installed: &str,
) -> AppUpdate {
    if current.busy() {
        return current.clone();
    }
    let checking = matches!(current, AppUpdate::Checking);
    match found {
        Ok(Some(release)) => AppUpdate::Available { release },
        Ok(None) if checking => AppUpdate::UpToDate {
            version: installed.to_owned(),
        },
        Ok(None) => AppUpdate::Idle,
        Err(message) if checking => AppUpdate::CheckFailed { message },
        Err(_) => current.clone(),
    }
}

/// Caches a release range's notes for the session.
pub fn release_notes_key(installed: &str, available: &str) -> String {
    format!("{installed}...{available}")
}

/// The version whose release notes the update details show.
pub fn release_notes_target(update: &AppUpdate) -> Option<&str> {
    update.release().map(|release| release.version.as_str())
}

/// Whole percent of a download; `None` while its size is unknown.
pub fn download_percent(received: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (received.min(total) * 100 / total) as u8)
}

/// A release body's bullet points, as plain text.
pub fn release_changes(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix(['-', '*', '+'])?;
            rest.starts_with(char::is_whitespace)
                .then(|| rest.trim())
                .filter(|change| !change.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

pub fn release_compare_url(installed: &str, available: &str) -> String {
    format!("https://github.com/nxtvoid/hoshi/compare/v{installed}...v{available}")
}

#[cfg(test)]
mod tests {
    use super::{
        AppUpdate, UpdateBadge, UpdateButton, after_check, check_due, download_percent,
        provider_badge, release_changes, release_compare_url, release_notes_target,
        strongest_badge, update_button, update_ended,
    };
    use hoshi_core::{AppRelease, ProviderUpdate, ProviderUpdateFailure, ProviderVersion};
    use std::time::Duration;

    fn release(version: &str) -> AppRelease {
        AppRelease {
            version: version.into(),
            published_at: "2026-11-01T00:00:00Z".into(),
            body: String::new(),
            url: format!("https://github.com/nxtvoid/hoshi/releases/tag/v{version}"),
            installer: None,
        }
    }

    fn version(update: ProviderUpdate, update_available: bool) -> ProviderVersion {
        ProviderVersion {
            installed: Some("2.0.0".into()),
            latest: Some("2.1.0".into()),
            update_available,
            update_command: Some("claude update".into()),
            update,
        }
    }

    #[test]
    fn updates_button_checks_until_an_update_shows_up() {
        assert_eq!(
            update_button(&AppUpdate::Idle, None, true),
            UpdateButton::Check { checking: false }
        );
        assert_eq!(
            update_button(&AppUpdate::Checking, None, true),
            UpdateButton::Check { checking: true }
        );
        assert_eq!(
            update_button(&AppUpdate::Idle, None, false),
            UpdateButton::Off
        );
        assert_eq!(
            update_button(
                &AppUpdate::Available {
                    release: release("0.1.0")
                },
                None,
                true
            ),
            UpdateButton::Details {
                badge: UpdateBadge::Available
            }
        );
    }

    #[test]
    fn a_provider_update_opens_details_even_in_dev_builds() {
        assert_eq!(
            update_button(&AppUpdate::Idle, Some(UpdateBadge::Running), false),
            UpdateButton::Details {
                badge: UpdateBadge::Running
            }
        );
    }

    #[test]
    fn the_strongest_badge_wins() {
        assert_eq!(
            strongest_badge(Some(UpdateBadge::Available), Some(UpdateBadge::Running)),
            Some(UpdateBadge::Running)
        );
        assert_eq!(
            strongest_badge(Some(UpdateBadge::Failed), Some(UpdateBadge::Done)),
            Some(UpdateBadge::Failed)
        );
        assert_eq!(strongest_badge(None, None), None);
    }

    #[test]
    fn provider_outcomes_badge_until_seen() {
        let failed = version(
            ProviderUpdate::Failed {
                reason: ProviderUpdateFailure::TimedOut,
                output: None,
            },
            true,
        );
        assert_eq!(
            provider_badge(Some(&failed), false),
            Some(UpdateBadge::Failed)
        );
        assert_eq!(
            provider_badge(Some(&failed), true),
            Some(UpdateBadge::Available)
        );
        let done = version(ProviderUpdate::Succeeded, false);
        assert_eq!(provider_badge(Some(&done), false), Some(UpdateBadge::Done));
        assert_eq!(provider_badge(Some(&done), true), None);
        let running = version(
            ProviderUpdate::Running {
                line: None,
                step: None,
            },
            true,
        );
        assert_eq!(
            provider_badge(Some(&running), true),
            Some(UpdateBadge::Running)
        );
        assert_eq!(provider_badge(None, false), None);
    }

    #[test]
    fn an_update_ends_when_it_stops_running() {
        let running = ProviderUpdate::Running {
            line: None,
            step: None,
        };
        assert!(update_ended(Some(&running), &ProviderUpdate::Succeeded));
        assert!(!update_ended(Some(&running), &running));
        assert!(!update_ended(None, &ProviderUpdate::Succeeded));
        assert!(!update_ended(
            Some(&ProviderUpdate::Idle),
            &ProviderUpdate::Succeeded
        ));
    }

    #[test]
    fn checks_show_how_they_ended_only_when_asked() {
        assert_eq!(
            after_check(&AppUpdate::Checking, Ok(None), "0.0.9"),
            AppUpdate::UpToDate {
                version: "0.0.9".into()
            }
        );
        assert_eq!(
            after_check(&AppUpdate::Idle, Ok(None), "0.0.9"),
            AppUpdate::Idle
        );
        assert_eq!(
            after_check(&AppUpdate::Checking, Err("offline".into()), "0.0.9"),
            AppUpdate::CheckFailed {
                message: "offline".into()
            }
        );
        assert_eq!(
            after_check(&AppUpdate::Idle, Err("offline".into()), "0.0.9"),
            AppUpdate::Idle
        );
        let found = after_check(&AppUpdate::Idle, Ok(Some(release("0.1.0"))), "0.0.9");
        assert_eq!(release_notes_target(&found), Some("0.1.0"));
    }

    #[test]
    fn a_check_never_replaces_a_download_or_a_ready_update() {
        let downloading = AppUpdate::Downloading {
            release: release("1.0.1"),
            percent: Some(40),
        };
        assert_eq!(
            after_check(&downloading, Ok(Some(release("1.0.2"))), "1.0.0"),
            downloading
        );
        let ready = AppUpdate::Ready {
            release: release("1.0.1"),
            installer: "setup.exe".into(),
        };
        assert_eq!(after_check(&ready, Ok(None), "1.0.0"), ready);
        let failed = AppUpdate::Failed {
            release: release("1.0.1"),
        };
        assert_eq!(
            after_check(&failed, Ok(Some(release("1.0.2"))), "1.0.0"),
            AppUpdate::Available {
                release: release("1.0.2")
            }
        );
    }

    #[test]
    fn the_app_update_badges_what_it_is_doing() {
        let badge = |update| update_button(&update, None, true);
        assert_eq!(
            badge(AppUpdate::Downloading {
                release: release("1.0.1"),
                percent: None
            }),
            UpdateButton::Details {
                badge: UpdateBadge::Running
            }
        );
        assert_eq!(
            badge(AppUpdate::Ready {
                release: release("1.0.1"),
                installer: "setup.exe".into()
            }),
            UpdateButton::Details {
                badge: UpdateBadge::Ready
            }
        );
        assert_eq!(
            strongest_badge(Some(UpdateBadge::Ready), Some(UpdateBadge::Failed)),
            Some(UpdateBadge::Ready)
        );
    }

    #[test]
    fn download_percent_is_whole_and_unknown_without_a_size() {
        assert_eq!(download_percent(0, 0), None);
        assert_eq!(download_percent(1, 3), Some(33));
        assert_eq!(download_percent(5, 3), Some(100));
    }

    #[test]
    fn focus_rechecks_after_two_minutes() {
        assert!(check_due(None));
        assert!(!check_due(Some(Duration::from_secs(60))));
        assert!(check_due(Some(Duration::from_secs(120))));
    }

    #[test]
    fn release_changes_are_the_bullets() {
        let body = "## What's new\r\n- Faster sidebar (#12)\n  * Nested fix\n+ Plus\n-not a bullet\n-   \nText";
        assert_eq!(
            release_changes(body),
            ["Faster sidebar (#12)", "Nested fix", "Plus"]
        );
    }

    #[test]
    fn compares_tags() {
        assert_eq!(
            release_compare_url("0.0.8", "0.0.9"),
            "https://github.com/nxtvoid/hoshi/compare/v0.0.8...v0.0.9"
        );
    }
}
