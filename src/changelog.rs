//! What changed in each version, from `changelog.json` at the top of the repository.
//!
//! The file is built into the binary for the version that is running, and
//! read from GitHub when a newer release exists, to show everything an
//! update would bring.

use crate::update::newer;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const BUNDLED: &str = include_str!("../changelog.json");
const LATEST: &str = "https://raw.githubusercontent.com/PirxcyFinal/rbxcache/main/changelog.json";

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Version {
    /// Without a leading `v`, e.g. `1.1.0`.
    pub version: String,
    /// The release date, `YYYY-MM-DD`.
    #[serde(default)]
    pub date: String,
    pub changes: Vec<String>,
}

#[derive(Deserialize)]
struct File {
    versions: Vec<Version>,
}

/// Reads a changelog, newest version first whatever order the file has.
fn parse(text: &str) -> Result<Vec<Version>> {
    let mut versions = serde_json::from_str::<File>(text).context("the changelog is not in the expected form")?.versions;
    versions.sort_by(|a, b| match (newer(&a.version, &b.version), newer(&b.version, &a.version)) {
        (true, _) => std::cmp::Ordering::Less,
        (_, true) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });
    Ok(versions)
}

/// The changelog this binary was built with.
pub fn bundled() -> Vec<Version> {
    parse(BUNDLED).unwrap_or_default()
}

/// The changelog as it is on GitHub now.
pub fn fetch() -> Result<Vec<Version>> {
    let response =
        ureq::get(LATEST).set("User-Agent", "rbxcache").timeout(Duration::from_secs(10)).call().context("could not fetch the changelog")?;
    parse(&response.into_string().context("could not read the changelog")?)
}

/// The versions after `current` up to and including `latest`: what updating brings.
pub fn between(versions: Vec<Version>, current: &str, latest: &str) -> Vec<Version> {
    versions.into_iter().filter(|v| newer(&v.version, current) && !newer(&v.version, latest)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_changelog_covers_this_version() {
        let versions = bundled();
        let newest = versions.first().expect("changelog.json lists no versions");
        assert_eq!(newest.version, crate::update::CURRENT, "changelog.json has no entry for the version being built");
        assert!(versions.iter().all(|v| !v.changes.is_empty() && v.changes.iter().all(|c| !c.trim().is_empty())));
    }

    #[test]
    fn an_update_shows_every_version_in_between() {
        let text = r#"{"versions": [
            {"version": "1.0.0", "changes": ["first"]},
            {"version": "2.0.0", "date": "2027-01-01", "changes": ["big"]},
            {"version": "1.10.0", "changes": ["later"]},
            {"version": "1.2.0", "changes": ["early"]},
            {"version": "2.1.0", "changes": ["not out yet"]}
        ]}"#;
        let versions = parse(text).unwrap();
        assert_eq!(versions.iter().map(|v| v.version.as_str()).collect::<Vec<_>>(), ["2.1.0", "2.0.0", "1.10.0", "1.2.0", "1.0.0"]);
        let news = between(versions, "1.2.0", "2.0.0");
        assert_eq!(news.iter().map(|v| v.version.as_str()).collect::<Vec<_>>(), ["2.0.0", "1.10.0"]);
        assert_eq!(news[0].date, "2027-01-01");
        assert!(parse("[]").is_err());
    }
}
