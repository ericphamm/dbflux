//! Deciding whether a newer release exists.
//!
//! This module is deliberately offline: it turns a GitHub releases payload
//! into a yes/no answer and nothing else. The HTTP call lives in the app layer,
//! which keeps the interesting part — which releases count, and which version
//! wins — testable without a network.
//!
//! # What counts as an update
//!
//! Release channels do not see each other's releases. A stable build must not
//! be told about a release candidate, and an rc build must be told about both
//! newer rcs and the stable release that supersedes it. Nightly is excluded
//! altogether: it publishes one rolling release whose version carries a commit
//! sha, so every launch would report an "update" that is really just today's
//! build.
//!
//! Precedence follows semver, which is the reason this does not compare
//! strings: `0.8.0-rc.2` is newer than `0.8.0-rc.1` but older than `0.8.0`,
//! and hand-rolled comparisons get that backwards.

use serde::Deserialize;

use crate::release_channel::ReleaseChannel;

/// One entry from `GET /repos/{owner}/{repo}/releases`.
///
/// Only the fields the decision needs are declared; the rest of the payload is
/// ignored so a GitHub API addition cannot break deserialization.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
}

/// A release newer than the running build, ready to show to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    /// Version without the leading `v`, as the user should read it.
    pub version: String,
    /// The release page, for "Details".
    pub url: String,
}

/// The repository releases are published to.
///
/// Taken from the package metadata rather than a constant here, so a fork that
/// publishes its own builds points the check at itself by setting
/// `repository` in `Cargo.toml` — the same value the About screen links to.
pub fn repository_url() -> &'static str {
    env!("CARGO_PKG_REPOSITORY")
}

/// The releases endpoint for [`repository_url`], or `None` when the metadata is
/// not a GitHub URL this can address.
pub fn releases_api_url() -> Option<String> {
    let path = repository_url()
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")?;

    // owner/repo and nothing else — a deeper path is not a repository root.
    let mut segments = path.split('/');
    let owner = segments.next().filter(|s| !s.is_empty())?;
    let repo = segments.next().filter(|s| !s.is_empty())?;
    if segments.next().is_some() {
        return None;
    }

    Some(format!(
        "https://api.github.com/repos/{owner}/{repo}/releases?per_page=30"
    ))
}

/// Whether this build should ask about updates at all.
///
/// Nightly is a rolling release: its published version differs from any local
/// build by a commit sha, so the check has nothing meaningful to say.
pub fn channel_wants_update_check(channel: ReleaseChannel) -> bool {
    !matches!(channel, ReleaseChannel::Nightly)
}

/// Whether `channel` should be offered `release`.
fn channel_accepts(channel: ReleaseChannel, release: &GitHubRelease) -> bool {
    if release.draft {
        return false;
    }

    match channel {
        // A stable install is not a testing ground.
        ReleaseChannel::Stable => !release.prerelease,
        // An rc install wants later rcs and the stable that replaces them, but
        // not nightlies, which are not published as releases of this line.
        ReleaseChannel::Rc => !release.tag_name.contains("-nightly"),
        ReleaseChannel::Nightly => false,
    }
}

/// The newest release worth telling the user about, if any.
///
/// `current` is the running version (`CARGO_PKG_VERSION`). `skipped` is the
/// version the user dismissed with "skip this version"; a *newer* release still
/// gets through, so skipping once does not mute the check for good.
///
/// Returns `None` when the running build is current, newer than anything
/// published, or when nothing parses — a malformed tag is skipped rather than
/// treated as an update.
pub fn newest_update(
    current: &str,
    channel: ReleaseChannel,
    releases: &[GitHubRelease],
    skipped: Option<&str>,
) -> Option<AvailableUpdate> {
    if !channel_wants_update_check(channel) {
        return None;
    }

    let current = semver::Version::parse(current).ok()?;
    let skipped =
        skipped.and_then(|value| semver::Version::parse(value.trim_start_matches('v')).ok());

    releases
        .iter()
        .filter(|release| channel_accepts(channel, release))
        .filter_map(|release| {
            let version = semver::Version::parse(release.tag_name.trim_start_matches('v')).ok()?;
            Some((version, release))
        })
        .filter(|(version, _)| *version > current)
        .filter(|(version, _)| skipped.as_ref().is_none_or(|skip| version > skip))
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(version, release)| AvailableUpdate {
            version: version.to_string(),
            url: if release.html_url.is_empty() {
                repository_url().to_string()
            } else {
                release.html_url.clone()
            },
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            html_url: format!("https://example.test/{tag}"),
            prerelease,
            draft: false,
        }
    }

    #[test]
    fn offers_a_newer_stable_to_a_stable_build() {
        let found = newest_update(
            "0.8.0",
            ReleaseChannel::Stable,
            &[release("v0.8.1", false), release("v0.8.0", false)],
            None,
        );

        assert_eq!(found.map(|u| u.version), Some("0.8.1".to_string()));
    }

    #[test]
    fn hides_prereleases_from_a_stable_build() {
        let found = newest_update(
            "0.8.0",
            ReleaseChannel::Stable,
            &[release("v0.9.0-rc.1", true)],
            None,
        );

        assert_eq!(found, None, "a stable install is not a testing ground");
    }

    #[test]
    fn offers_a_later_rc_to_an_rc_build() {
        let found = newest_update(
            "0.8.0-rc.0",
            ReleaseChannel::Rc,
            &[release("v0.8.0-rc.1", true)],
            None,
        );

        assert_eq!(found.map(|u| u.version), Some("0.8.0-rc.1".to_string()));
    }

    #[test]
    fn offers_the_stable_that_supersedes_an_rc() {
        // Semver precedence, the reason this does not compare strings:
        // 0.8.0 is newer than 0.8.0-rc.9 even though it sorts earlier.
        let found = newest_update(
            "0.8.0-rc.9",
            ReleaseChannel::Rc,
            &[release("v0.8.0", false)],
            None,
        );

        assert_eq!(found.map(|u| u.version), Some("0.8.0".to_string()));
    }

    #[test]
    fn picks_the_newest_when_several_are_published() {
        let found = newest_update(
            "0.8.0",
            ReleaseChannel::Stable,
            &[
                release("v0.8.1", false),
                release("v0.10.0", false),
                release("v0.9.3", false),
            ],
            None,
        );

        assert_eq!(
            found.map(|u| u.version),
            Some("0.10.0".to_string()),
            "0.10.0 beats 0.9.3 — numeric, not lexical"
        );
    }

    #[test]
    fn stays_quiet_when_the_build_is_current_or_ahead() {
        for current in ["0.8.1", "0.9.0"] {
            assert_eq!(
                newest_update(
                    current,
                    ReleaseChannel::Stable,
                    &[release("v0.8.1", false)],
                    None
                ),
                None,
                "{current} must not be told about 0.8.1"
            );
        }
    }

    #[test]
    fn skipping_a_version_mutes_only_that_one() {
        let releases = [release("v0.8.1", false), release("v0.8.2", false)];

        assert_eq!(
            newest_update(
                "0.8.0",
                ReleaseChannel::Stable,
                &releases[..1],
                Some("0.8.1")
            ),
            None,
            "the skipped version stays hidden"
        );

        assert_eq!(
            newest_update("0.8.0", ReleaseChannel::Stable, &releases, Some("0.8.1"))
                .map(|u| u.version),
            Some("0.8.2".to_string()),
            "a later release still gets through"
        );
    }

    #[test]
    fn ignores_drafts_and_unparseable_tags() {
        let mut draft = release("v0.9.0", false);
        draft.draft = true;

        let found = newest_update(
            "0.8.0",
            ReleaseChannel::Stable,
            &[draft, release("nightly", false), release("v-broken", false)],
            None,
        );

        assert_eq!(found, None);
    }

    #[test]
    fn nightly_never_asks() {
        assert!(!channel_wants_update_check(ReleaseChannel::Nightly));
        assert_eq!(
            newest_update(
                "0.9.0",
                ReleaseChannel::Nightly,
                &[release("v0.9.1", false)],
                None
            ),
            None
        );
    }

    #[test]
    fn builds_the_api_url_from_a_github_repository() {
        // Guards the real metadata: a repository value this cannot address
        // would silently disable the feature.
        assert!(
            releases_api_url().is_some(),
            "CARGO_PKG_REPOSITORY ({}) must be a github.com/owner/repo URL",
            repository_url()
        );
        let url = releases_api_url().expect("checked above");
        assert!(url.starts_with("https://api.github.com/repos/"), "{url}");
        assert!(url.ends_with("/releases?per_page=30"), "{url}");
    }
}
