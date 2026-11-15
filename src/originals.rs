//! What entries held before a config replaced them, kept outside the cache
//! so that turning a config or a rule off can put it back.
//!
//! For each replaced entry the original content is saved once, along with the
//! fingerprint of what was written in its place. That second fingerprint tells
//! a replacement of ours from content Roblox has written since: only the
//! former is ever reverted, and only the latter is ever saved as an original.

use crate::config::{fingerprint, Fingerprint};
use crate::protection::data_dir;
use crate::store::{hex, parse_id, Entry, Id, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The content the entry had before it was replaced, and what kind of content it was.
    pub original: Fingerprint,
    pub kind: String,
    /// The replacement that was written over it.
    pub written: Fingerprint,
}

pub struct Originals {
    root: PathBuf,
    records: BTreeMap<String, Record>,
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

impl Originals {
    /// The originals kept in the app's data folder.
    pub fn load_default() -> Result<Self> {
        Self::load(data_dir()?.join("originals"))
    }

    pub fn load(root: PathBuf) -> Result<Self> {
        let path = root.join("manifest.json");
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not a valid list of originals", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { root, records })
    }

    fn body_path(&self, id_hex: &str) -> PathBuf {
        self.root.join(format!("{id_hex}.body"))
    }

    /// The record for an entry whose content was replaced, if `current` (the
    /// fingerprint of what it holds now) is still that replacement.
    pub fn replaced(&self, id: &Id, current: &Fingerprint) -> Option<&Record> {
        self.records.get(&hex(id)).filter(|record| record.written.md5 == current.md5 && record.written.size == current.size)
    }

    /// The content that was saved from an entry before it was replaced.
    pub fn original_body(&self, id: &Id) -> Result<Vec<u8>> {
        let path = self.body_path(&hex(id));
        std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
    }

    /// The record for an entry, if it is of the size the replacement had.
    /// Cheaper than [`Originals::replaced`], for when the content has not been read.
    pub fn probably_replaced(&self, entry: &Entry) -> Option<&Record> {
        self.records.get(&entry.id_hex).filter(|record| record.written.size == entry.body_len)
    }

    /// Called before `current`, the content of an entry of this `kind`, is
    /// overwritten with `new`. The content is saved unless it is itself a
    /// replacement, in which case the original saved earlier stays.
    pub fn keep(&mut self, id: &Id, kind: &str, current: &[u8], new: &[u8]) -> Result<()> {
        let id_hex = hex(id);
        let (original, kind) = match self.replaced(id, &fingerprint(current)) {
            Some(record) => (record.original.clone(), record.kind.clone()),
            None => {
                std::fs::create_dir_all(&self.root).with_context(|| format!("creating {}", self.root.display()))?;
                write_atomically(&self.body_path(&id_hex), current)?;
                (fingerprint(current), kind.to_string())
            }
        };
        self.records.insert(id_hex, Record { original, kind, written: fingerprint(new) });
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root).with_context(|| format!("creating {}", self.root.display()))?;
        write_atomically(&self.root.join("manifest.json"), &serde_json::to_vec_pretty(&self.records)?)
    }

    /// Puts back the original of every replaced entry that is not in
    /// `claimed`, and forgets it. An entry Roblox has written again already
    /// holds its original, and one that is gone has nothing to restore; both
    /// are only forgotten. Returns the entries as they are after being restored.
    pub fn restore_unclaimed(&mut self, store: &Store, claimed: &HashSet<Id>) -> Vec<Entry> {
        let mut restored = Vec::new();
        let claimed: HashSet<String> = claimed.iter().map(|id| hex(id)).collect();
        let unclaimed: Vec<String> = self.records.keys().filter(|id_hex| !claimed.contains(*id_hex)).cloned().collect();
        for id_hex in unclaimed {
            let Some(id) = parse_id(&id_hex) else { continue };
            let ours = store.read_body(&id).is_ok_and(|current| self.replaced(&id, &fingerprint(&current)).is_some());
            if ours {
                let put_back = std::fs::read(self.body_path(&id_hex)).map_err(anyhow::Error::from).and_then(|body| store.replace_body(&id, &body));
                match put_back {
                    Ok(entry) => restored.push(entry),
                    // Kept for another try: the original is still on disk and the entry still replaced.
                    Err(_) => continue,
                }
            }
            self.records.remove(&id_hex);
            let _ = std::fs::remove_file(self.body_path(&id_hex));
        }
        restored
    }
}
