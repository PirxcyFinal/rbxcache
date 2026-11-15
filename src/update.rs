//! Checking GitHub for a newer release.

use crate::changelog::{self, Version};
use anyhow::{anyhow, Context, Result};
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/PirxcyFinal/rbxcache/releases/latest";

/// The version this binary was built as.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Version without the leading `v`, e.g. `0.5.0`.
    pub version: String,
    /// The release page.
    pub url: String,
    /// What changed in each version after this one, newest first. Empty when
    /// the release is not newer or the changelog could not be fetched.
    pub news: Vec<Version>,
}

impl Release {
    pub fn is_newer(&self) -> bool {
        newer(&self.version, CURRENT)
    }
}

/// Asks GitHub for the latest published release.
pub fn latest() -> Result<Release> {
    let response =
        ureq::get(LATEST).set("User-Agent", "rbxcache").set("Accept", "application/vnd.github+json").timeout(Duration::from_secs(10)).call();
    let response = match response {
        Ok(response) => response,
        // GitHub answers 404 for a repository that is private or has no releases.
        Err(ureq::Error::Status(404, _)) => return Err(anyhow!("no public release was found")),
        Err(e) => return Err(e).context("could not reach GitHub"),
    };
    let mut release = parse(&serde_json::from_reader(response.into_reader()).context("GitHub sent an unreadable answer")?)?;
    if release.is_newer() {
        // The update is worth reporting even when its changelog cannot be had.
        release.news = changelog::fetch().map(|all| changelog::between(all, CURRENT, &release.version)).unwrap_or_default();
    }
    Ok(release)
}

/// Reads the fields this tool needs from GitHub's description of a release.
fn parse(body: &serde_json::Value) -> Result<Release> {
    let tag = body["tag_name"].as_str().context("the release has no tag")?;
    let url = body["html_url"].as_str().context("the release has no page")?;
    Ok(Release { version: tag.trim_start_matches('v').to_string(), url: url.to_string(), news: Vec::new() })
}

/// True when version `a` is later than `b`. Numbers are compared part by part;
/// anything after a `-` or `+` (a pre-release or build label) is ignored.
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        let core = v.trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
        core.split('.').map(|n| n.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parts(a), parts(b));
    (0..a.len().max(b.len()))
        .map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0)))
        .find(|(x, y)| x != y)
        .is_some_and(|(x, y)| x > y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_release() {
        let body = serde_json::json!({"tag_name": "v9.9.9", "html_url": "https://example.com/r", "draft": false});
        let release = parse(&body).unwrap();
        assert_eq!(release, Release { version: "9.9.9".into(), url: "https://example.com/r".into(), news: Vec::new() });
        assert!(release.is_newer());
        assert!(parse(&serde_json::json!({"message": "Not Found"})).is_err());
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("v1.0", "0.99.99"));
        assert!(newer("0.5.1", "0.5"));
        assert!(!newer("0.5.0", "0.5.0"));
        assert!(!newer("0.5", "0.5.0"));
        assert!(!newer("0.4.9", "0.5.0"));
        assert!(!newer("0.5.0-beta", "0.5.0"));
        assert!(!newer("garbage", "0.1.0"));
    }
}
