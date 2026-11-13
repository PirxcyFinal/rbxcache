//! Durable replacement storage, kept outside Roblox's volatile cache.

use crate::store::{self, Entry, Id, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    body: String,
    #[serde(default)]
    restored: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    replacements: BTreeMap<String, Saved>,
}

pub struct Protections {
    root: PathBuf,
    manifest: Manifest,
}

impl Protections {
    pub fn load_default() -> Result<Self> {
        Self::load(data_dir()?.join("protections"))
    }

    fn load(root: PathBuf) -> Result<Self> {
        let path = root.join("manifest.json");
        let manifest = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
                .with_context(|| format!("parsing {}", path.display()))?
        } else {
            Manifest::default()
        };
        Ok(Self { root, manifest })
    }

    pub fn remember(&mut self, id: &Id, body: &[u8]) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let hex = store::hex(id);
        let body_name = format!("{hex}.body");
        atomic_write(&self.root.join(&body_name), body)?;
        let restored = self.manifest.replacements.get(&hex).map_or(0, |v| v.restored);
        self.manifest.replacements.insert(hex, Saved { body: body_name, restored });
        self.save()
    }

    pub fn restore(&mut self, store: &Store, ids: impl IntoIterator<Item = Id>) -> Result<Vec<Entry>> {
        let mut restored = Vec::new();
        for id in ids {
            let hex = store::hex(&id);
            let Some(saved) = self.manifest.replacements.get(&hex).cloned() else { continue };
            let body = std::fs::read(self.root.join(&saved.body))?;
            let current = store.read_body(&id).ok();
            if current.as_deref() == Some(body.as_slice()) {
                continue;
            }
            let entry = store.replace_body(&id, &body)?;
            if let Some(meta) = self.manifest.replacements.get_mut(&hex) {
                meta.restored += 1;
            }
            restored.push(entry);
        }
        if !restored.is_empty() {
            self.save()?;
        }
        Ok(restored)
    }

    pub fn entries(&self) -> impl Iterator<Item = Id> + '_ {
        self.manifest.replacements.keys().filter_map(|s| parse_id(s))
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        atomic_write(&self.root.join("manifest.json"), &serde_json::to_vec_pretty(&self.manifest)?)
    }
}

pub fn data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base =
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.map(|p| p.join("rbxcache")).context("no application data directory is available")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

fn parse_id(s: &str) -> Option<Id> {
    if s.len() != 32 {
        return None;
    }
    let mut id = [0; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_path_ends_in_app_name() {
        assert!(data_dir().unwrap().ends_with("rbxcache"));
    }

    #[test]
    fn ids_round_trip() {
        let id = [0xab; 16];
        assert_eq!(parse_id(&store::hex(&id)), Some(id));
    }
}
