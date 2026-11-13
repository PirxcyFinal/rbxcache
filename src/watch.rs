//! Following the cache while Roblox uses it.
//!
//! The index is polled rather than notified: SQLite's `data_version` says
//! whether anything was committed since the last look, and only then are the
//! rows compared with what was seen before.

use crate::store::{Entry, Id, Stamp, Store};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// What happened to the cache between two polls.
#[derive(Default)]
pub struct Change {
    /// Entries that are new or whose content was rewritten.
    pub written: Vec<Entry>,
    pub removed: Vec<Id>,
    /// Entries that were only read: their hit count or access time moved.
    pub touched: Vec<Stamp>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.written.is_empty() && self.removed.is_empty() && self.touched.is_empty()
    }
}

pub struct Watcher {
    store: Store,
    version: Option<i64>,
    known: HashMap<Id, Stamp>,
}

impl Watcher {
    /// Watches the cache in `dir`, reporting changes relative to `known`.
    pub fn new(dir: &Path, known: impl Iterator<Item = Stamp>) -> Result<Self> {
        Ok(Self { store: Store::open(dir, false)?, version: None, known: known.map(|s| (s.id, s)).collect() })
    }

    /// Returns what changed since the last call, or `None` if nothing was committed.
    pub fn poll(&mut self) -> Result<Option<Change>> {
        let version = self.store.data_version()?;
        if self.version == Some(version) {
            return Ok(None);
        }
        let mut change = Change::default();
        let mut seen: HashMap<Id, Stamp> = HashMap::with_capacity(self.known.len());
        for stamp in self.store.stamps()? {
            match self.known.get(&stamp.id) {
                Some(old) if old.size == stamp.size => {
                    if *old != stamp {
                        change.touched.push(stamp);
                    }
                }
                // New, or rewritten with a different size. An entry deleted between
                // the listing and this read is picked up as removed on the next poll.
                _ => match self.store.entry(&stamp.id) {
                    Ok(entry) => change.written.push(entry),
                    Err(_) => continue,
                },
            }
            seen.insert(stamp.id, stamp);
        }
        change.removed = self.known.keys().filter(|id| !seen.contains_key(*id)).copied().collect();
        self.known = seen;
        self.version = Some(version);
        Ok((!change.is_empty()).then_some(change))
    }
}

/// Applies a change to a list of entries, keeping it in step with the cache.
pub fn merge(entries: &mut Vec<Entry>, change: &Change) {
    let index: HashMap<Id, usize> = entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
    for stamp in &change.touched {
        if let Some(&i) = index.get(&stamp.id) {
            (entries[i].hits, entries[i].atime) = (stamp.hits, stamp.atime);
        }
    }
    for entry in &change.written {
        match index.get(&entry.id) {
            Some(&i) => entries[i] = entry.clone(),
            None => entries.push(entry.clone()),
        }
    }
    if !change.removed.is_empty() {
        let gone: std::collections::HashSet<&Id> = change.removed.iter().collect();
        entries.retain(|e| !gone.contains(&e.id));
    }
}

/// What Roblox did with an entry this tool replaced.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Roblox read the replaced entry and left it alone; `reads` counts every read so far.
    Read { id: Id, reads: i64 },
    /// Roblox wrote the entry again, discarding the replacement.
    Reverted { id: Id, times: u32 },
}

struct Applied {
    size: u64,
    hits: i64,
    reads: i64,
    reverts: u32,
}

/// Remembers which entries were replaced and watches what becomes of them.
#[derive(Default)]
pub struct Tracker {
    applied: HashMap<Id, Applied>,
}

impl Tracker {
    /// Notes that `entry` now holds a replacement.
    pub fn record(&mut self, entry: &Entry) {
        let reverts = self.applied.get(&entry.id).map_or(0, |a| a.reverts);
        self.applied.insert(entry.id, Applied { size: entry.size, hits: entry.hits, reads: 0, reverts });
    }

    /// True when `entry` is still the replacement that was recorded for it.
    pub fn holds(&self, entry: &Entry) -> bool {
        self.applied.get(&entry.id).is_some_and(|a| a.size == entry.size)
    }

    /// Protected replacements are retried indefinitely.
    pub fn may_replace(&self, id: &Id) -> bool {
        let _ = id;
        true
    }

    /// How often Roblox has read the replacement in `id`, if it holds one.
    pub fn reads(&self, id: &Id) -> Option<i64> {
        self.applied.get(id).map(|a| a.reads)
    }

    pub fn observe(&mut self, change: &Change) -> Vec<Verdict> {
        let mut verdicts = Vec::new();
        for stamp in &change.touched {
            if let Some(a) = self.applied.get_mut(&stamp.id) {
                if stamp.hits > a.hits {
                    a.reads += stamp.hits - a.hits;
                    a.hits = stamp.hits;
                    verdicts.push(Verdict::Read { id: stamp.id, reads: a.reads });
                }
            }
        }
        for entry in &change.written {
            if let Some(a) = self.applied.get_mut(&entry.id) {
                // The poll after our own write reports it too; that one has our size.
                if entry.size != a.size {
                    a.reverts += 1;
                    a.size = entry.size;
                    verdicts.push(Verdict::Reverted { id: entry.id, times: a.reverts });
                }
            }
        }
        for id in &change.removed {
            self.applied.remove(id);
        }
        verdicts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u8, size: u64, hits: i64) -> Entry {
        Entry {
            id: [id; 16],
            id_hex: crate::store::hex(&[id; 16]),
            size,
            hits,
            atime: 0,
            category: 10,
            ttl: 0,
            inline: true,
            kind: "png",
            wrapped: true,
            url: String::new(),
            status: 200,
            asset_type: None,
            body_len: size,
        }
    }

    #[test]
    fn merge_follows_writes_reads_and_removals() {
        let mut entries = vec![entry(1, 10, 0), entry(2, 20, 0)];
        let change = Change {
            written: vec![entry(2, 25, 0), entry(3, 30, 0)],
            removed: vec![[1; 16]],
            touched: vec![Stamp { id: [2; 16], size: 20, hits: 7, atime: 99 }],
        };
        merge(&mut entries, &change);
        let seen: Vec<(u8, u64)> = entries.iter().map(|e| (e.id[0], e.size)).collect();
        assert_eq!(seen, [(2, 25), (3, 30)]);
    }

    #[test]
    fn tracker_tells_reads_from_reverts() {
        let mut tracker = Tracker::default();
        tracker.record(&entry(1, 50, 4));
        // Our own write showing up in the next poll is not a revert.
        assert!(tracker.observe(&Change { written: vec![entry(1, 50, 4)], ..Default::default() }).is_empty());
        let read = Change { touched: vec![Stamp { id: [1; 16], size: 50, hits: 6, atime: 1 }], ..Default::default() };
        assert_eq!(tracker.observe(&read), [Verdict::Read { id: [1; 16], reads: 2 }]);
        for times in 1..=5 {
            assert!(tracker.may_replace(&[1; 16]));
            let rewritten = Change { written: vec![entry(1, 900 + times as u64, 6)], ..Default::default() };
            assert_eq!(tracker.observe(&rewritten), [Verdict::Reverted { id: [1; 16], times }]);
            tracker.record(&entry(1, 50, 6));
        }
        assert!(tracker.may_replace(&[1; 16]));
    }
}
