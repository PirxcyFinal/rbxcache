//! Cache entries that were deleted on request and may have to be put back.
//!
//! While the Rivals skyboxes are being removed, every entry that holds one is
//! deleted as soon as Roblox writes it. A copy of each is kept here first, so
//! that reverting can put the skyboxes back at once instead of waiting for
//! Roblox to download them again.

use crate::protection::data_dir;
use crate::store::{hex, parse_id, Entry, Id, Kept, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The row of a kept entry; its stored bytes are in a file beside the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    hits: i64,
    atime: i64,
    category: i64,
    #[serde(default)]
    score: i64,
    ttl: i64,
}

pub struct Removed {
    root: PathBuf,
    records: BTreeMap<String, Record>,
}

impl Removed {
    /// The copies kept in the app's data folder.
    pub fn load_default() -> Result<Self> {
        Self::load(data_dir()?.join("removed-skyboxes"))
    }

    pub fn load(root: PathBuf) -> Result<Self> {
        let path = root.join("manifest.json");
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not a valid list of removed entries", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { root, records })
    }

    fn raw_path(&self, id_hex: &str) -> PathBuf {
        self.root.join(format!("{id_hex}.entry"))
    }

    /// How many deleted entries there is a copy of.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root).with_context(|| format!("creating {}", self.root.display()))?;
        crate::originals::write_atomically(&self.root.join("manifest.json"), &serde_json::to_vec_pretty(&self.records)?)
    }

    /// Keeps a copy of an entry and deletes it from the cache. A copy kept
    /// earlier of the same entry is replaced: this one is what Roblox wrote last.
    pub fn remove(&mut self, store: &Store, id: &Id) -> Result<()> {
        let id_hex = hex(id);
        let kept = store.keep(id)?;
        std::fs::create_dir_all(&self.root).with_context(|| format!("creating {}", self.root.display()))?;
        let path = self.raw_path(&id_hex);
        std::fs::write(&path, &kept.raw).with_context(|| format!("writing {}", path.display()))?;
        if let Err(e) = store.delete(id) {
            // Still in the cache, so there is nothing to put back; an older copy stays as it was only if it was not overwritten.
            if !self.records.contains_key(&id_hex) {
                let _ = std::fs::remove_file(&path);
            }
            return Err(e);
        }
        self.records.insert(id_hex, Record { hits: kept.hits, atime: kept.atime, category: kept.category, score: kept.score, ttl: kept.ttl });
        Ok(())
    }

    /// Puts every kept entry back into the cache and forgets it. One that
    /// Roblox has written again since is already there and is only forgotten.
    /// Returns the entries that were put back and how many could not be;
    /// those stay kept for another try.
    pub fn restore_all(&mut self, store: &Store) -> (Vec<Entry>, usize) {
        let mut restored = Vec::new();
        let mut failed = 0;
        for id_hex in self.records.keys().cloned().collect::<Vec<_>>() {
            let Some(id) = parse_id(&id_hex) else { continue };
            let record = &self.records[&id_hex];
            let path = self.raw_path(&id_hex);
            let put_back = std::fs::read(&path).map_err(anyhow::Error::from).and_then(|raw| {
                store.put_back(
                    &id,
                    &Kept { raw, hits: record.hits, atime: record.atime, category: record.category, score: record.score, ttl: record.ttl },
                )
            });
            match put_back {
                Ok(entry) => restored.extend(entry),
                Err(_) => {
                    failed += 1;
                    continue;
                }
            }
            self.records.remove(&id_hex);
            let _ = std::fs::remove_file(&path);
        }
        (restored, failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_entry_is_put_back_as_it_was() {
        // One small enough to live in the index and one that lives in a file of its own.
        let large = [b"OggS".as_slice(), &vec![7u8; crate::store::INLINE_MAX + 500]].concat();
        let (dir, ids) = crate::config::tests::cache_of("removed", &[b"OggS small", &large, b"OggS untouched"]);
        let store = Store::open(&dir, true).unwrap();
        let before: Vec<_> = ids.iter().map(|id| (store.read(id).unwrap(), store.entry(id).unwrap().category)).collect();
        let mut removed = Removed::load(dir.join("removed")).unwrap();
        removed.remove(&store, &ids[0]).unwrap();
        removed.remove(&store, &ids[1]).unwrap();
        removed.save().unwrap();
        assert!(store.entry(&ids[0]).is_err() && store.entry(&ids[1]).is_err());
        assert!(!store.blob_path(&ids[1]).exists());

        // The list survives a restart.
        let mut removed = Removed::load(dir.join("removed")).unwrap();
        assert_eq!(removed.len(), 2);
        let (restored, failed) = removed.restore_all(&store);
        assert_eq!((restored.len(), failed, removed.len()), (2, 0, 0));
        for (id, (raw, category)) in ids.iter().zip(&before) {
            assert_eq!(&store.read(id).unwrap(), raw);
            assert_eq!(store.entry(id).unwrap().category, *category);
        }
        assert!(store.blob_path(&ids[1]).exists() && !store.entry(&ids[0]).unwrap().kind.is_empty());
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_roblox_wrote_again_is_left_alone() {
        let (dir, ids) = crate::config::tests::cache_of("removed-again", &[b"OggS first download"]);
        let store = Store::open(&dir, true).unwrap();
        let mut removed = Removed::load(dir.join("removed")).unwrap();
        let kept = store.keep(&ids[0]).unwrap();
        removed.remove(&store, &ids[0]).unwrap();
        // Roblox downloads the asset again while the copy is kept.
        let again = Kept { raw: kept.raw.iter().copied().chain(*b" again").collect(), ..kept };
        store.put_back(&ids[0], &again).unwrap().unwrap();
        let (restored, failed) = removed.restore_all(&store);
        assert_eq!((restored.len(), failed, removed.len()), (0, 0, 0));
        assert_eq!(store.read(&ids[0]).unwrap(), again.raw);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
