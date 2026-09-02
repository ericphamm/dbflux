//! Asking the release repository whether a newer version exists.
//!
//! The decision itself lives in [`dbflux_core::update_check`]; this is only the
//! HTTP call around it, kept here so the app layer owns the one place that
//! touches the network for this feature.
//!
//! # When it runs
//!
//! Once per launch, and only when the user has left the check enabled. There is
//! no timer and no cached timestamp: a single unauthenticated request per
//! launch sits far inside GitHub's 60-per-hour limit, and a launch is exactly
//! the moment the answer is worth having. Adding a schedule would mean
//! persisting state to avoid asking too often — machinery for a problem that
//! does not exist at this rate.
//!
//! # Blocking, on purpose
//!
//! `reqwest`'s blocking client, run on the background executor, matching the
//! ClickHouse and InfluxDB drivers. The async client needs a tokio reactor,
//! and gpui's executor is not one — an async call from a gpui task would
//! panic at runtime rather than fail a compile.
//!
//! # Privacy
//!
//! The request tells GitHub this machine's IP and that it runs DBFlux. That is
//! why it is a setting rather than unconditional behaviour: with
//! `check_for_updates` off, no request is made at all.

use std::time::Duration;

use dbflux_core::update_check::{self as decide, AvailableUpdate, GitHubRelease};

/// How long to wait before giving up. An update notice is not worth stalling
/// anything, so this is short and a timeout is simply "no answer".
const TIMEOUT: Duration = Duration::from_secs(10);

/// Ask the release repository for anything newer than this build.
///
/// Blocking: call it from a background thread, never the foreground.
///
/// Returns `Ok(None)` for "nothing to report", which covers a current build, a
/// disabled check, a channel that does not want one, and a repository whose
/// metadata is not addressable. Errors are reserved for a request that was
/// attempted and failed, so the caller can log them without treating a quiet
/// answer as a fault.
pub fn check_for_update(
    enabled: bool,
    skipped_version: &str,
) -> Result<Option<AvailableUpdate>, String> {
    let channel = dbflux_core::ReleaseChannel::current();

    if !enabled || !decide::channel_wants_update_check(channel) {
        return Ok(None);
    }

    let Some(url) = decide::releases_api_url() else {
        return Ok(None);
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        // GitHub rejects requests without a user agent.
        .user_agent(dbflux_core::client_identity())
        .build()
        .map_err(|error| format!("could not build the HTTP client: {error}"))?;

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|error| format!("could not reach {url}: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }

    let releases: Vec<GitHubRelease> = response
        .json()
        .map_err(|error| format!("could not read the release list: {error}"))?;

    let skipped = Some(skipped_version).filter(|value| !value.is_empty());

    Ok(decide::newest_update(
        env!("CARGO_PKG_VERSION"),
        channel,
        &releases,
        skipped,
    ))
}
