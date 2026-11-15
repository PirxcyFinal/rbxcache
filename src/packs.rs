//! The list of known packs, from `packs.json` at the top of the repository.
//!
//! A pack is a replacement config someone has published. The list names each
//! one and says where its config is; applying a pack downloads that config
//! into the app's data folder and loads it like any other.
//!
//! The list can also carry fingerprints for asset IDs. Roblox refuses to hand
//! out some assets, sounds above all, without a login, so their IDs cannot be
//! looked up here. Someone who has tagged those entries by hand can publish
//! the fingerprints, and everyone else's copies then match them too.

use crate::config::{self, IdMap};
use crate::protection::data_dir;
use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BUNDLED: &str = include_str!("../packs.json");
/// Where the list lives; a pack's `config` is taken relative to this.
const LATEST: &str = "https://raw.githubusercontent.com/PirxcyFinal/rbxcache/main/packs.json";
/// Largest config that is downloaded.
const LIMIT: u64 = 8 << 20;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Pack {
    /// Short name used for the saved file: lower-case letters, digits and dashes.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    /// The experience the pack is made for.
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub description: String,
    /// The pack's config: an `https` address, or a path beside the list.
    pub config: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Catalog {
    pub packs: Vec<Pack>,
    /// Fingerprints for asset IDs that cannot be looked up without a login.
    #[serde(default)]
    pub fingerprints: IdMap,
}

fn parse(text: &str) -> Result<Catalog> {
    let mut catalog: Catalog = serde_json::from_str(text).context("the pack list is not in the expected form")?;
    // The id becomes a file name, so one that is not plainly safe is dropped.
    catalog.packs.retain(|pack| !pack.id.is_empty() && pack.id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'));
    Ok(catalog)
}

/// The list this binary was built with.
pub fn bundled() -> Catalog {
    parse(BUNDLED).unwrap_or_default()
}

fn get(url: &str) -> Result<String> {
    use std::io::Read;
    let response =
        ureq::get(url).set("User-Agent", "rbxcache").timeout(Duration::from_secs(12)).call().with_context(|| format!("downloading {url}"))?;
    let mut text = String::new();
    response.into_reader().take(LIMIT + 1).read_to_string(&mut text).with_context(|| format!("reading {url}"))?;
    ensure!(text.len() as u64 <= LIMIT, "{url} is larger than {} MB", LIMIT >> 20);
    Ok(text)
}

/// The list as it is on GitHub now.
pub fn fetch() -> Result<Catalog> {
    parse(&get(LATEST)?)
}

impl Pack {
    fn config_url(&self) -> Result<String> {
        if self.config.starts_with("https://") {
            return Ok(self.config.clone());
        }
        if self.config.contains("://") || self.config.contains("..") || self.config.starts_with('/') {
            bail!("the pack list gives {} a config address that cannot be used: {}", self.name, self.config);
        }
        let folder = LATEST.rsplit_once('/').map_or(LATEST, |(folder, _)| folder);
        Ok(format!("{folder}/{}", self.config.replace(' ', "%20")))
    }

    /// Where the pack's config is kept once downloaded.
    pub fn path(&self) -> Result<PathBuf> {
        Ok(data_dir()?.join("packs").join(format!("{}.json", self.id)))
    }

    /// Downloads the pack's config and saves it, replacing an older copy.
    /// Returns where it was saved.
    pub fn install(&self) -> Result<PathBuf> {
        self.install_into(&data_dir()?.join("packs"))
    }

    /// [`Pack::install`], into a folder of the caller's choosing.
    pub fn install_into(&self, folder: &Path) -> Result<PathBuf> {
        let text = get(&self.config_url()?)?;
        let rules = config::parse(&text, folder).with_context(|| format!("the config of {} could not be read", self.name))?;
        ensure!(!rules.is_empty(), "the config of {} has no rules", self.name);
        // A config from the internet may fetch files, but not name files on this computer,
        // and what it fetches has to come over an encrypted connection.
        for rule in &rules {
            match &rule.action {
                config::Action::Local(_) => bail!("the config of {} reads a file from this computer in rule '{}'", self.name, rule.name),
                config::Action::Cdn(url) if !url.starts_with("https://") => {
                    bail!("the config of {} downloads from an address that is not https in rule '{}'", self.name, rule.name)
                }
                _ => {}
            }
        }
        std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
        let path = folder.join(format!("{}.json", self.id));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_list_names_configs_in_the_repository() {
        let catalog = bundled();
        assert!(!catalog.packs.is_empty(), "packs.json lists no packs");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for pack in &catalog.packs {
            assert!(!pack.name.is_empty());
            let rules = config::load(&root.join(&pack.config)).unwrap_or_else(|e| panic!("{}: {e:#}", pack.name));
            assert!(!rules.is_empty(), "{} has no rules", pack.name);
            assert!(pack.config_url().unwrap().starts_with("https://raw.githubusercontent.com/PirxcyFinal/rbxcache/main/packs/"));
        }
    }

    #[test]
    fn unsafe_ids_and_addresses_are_refused() {
        let text = r#"{"packs": [
            {"id": "fine-1", "name": "Fine", "config": "packs/fine.json"},
            {"id": "../evil", "name": "Evil", "config": "packs/evil.json"},
            {"id": "", "name": "Nameless", "config": "x.json"}
        ], "fingerprints": {"5": [{"md5": "00", "size": 1}]}}"#;
        let catalog = parse(text).unwrap();
        assert_eq!(catalog.packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["fine-1"]);
        assert_eq!(catalog.fingerprints.assets["5"].len(), 1);
        let pack = |config: &str| Pack { config: config.into(), ..catalog.packs[0].clone() };
        assert!(pack("https://example.com/a.json").config_url().is_ok());
        for bad in ["http://example.com/a.json", "../secrets.json", "/etc/passwd", "file:///c:/x"] {
            assert!(pack(bad).config_url().is_err(), "{bad} was accepted");
        }
        assert!(parse(r#"{"packs": []}"#).unwrap().fingerprints.assets.is_empty());
    }
}
