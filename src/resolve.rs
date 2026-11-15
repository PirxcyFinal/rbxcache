//! Working out which cached entries belong to which Roblox asset ID.
//!
//! The cache does not record asset IDs, but Roblox's asset delivery endpoint
//! serves many assets to anyone, with no login. An asset fetched that way is
//! byte-for-byte what the client cached, so its fingerprint identifies the
//! entry. Images are the exception: the client asks for a KTX2 rendition, so
//! that rendition is fetched as well.
//!
//! Both fingerprints go into the ID map whether or not the asset is cached
//! yet, so an entry Roblox downloads later is recognised when it arrives.
//!
//! Assets that need a login or special permission answer 401 or 403 and stay
//! unidentified; nothing here sends credentials.

use crate::config::{fingerprint, Fingerprint, IdMap};
use crate::format;
use crate::store::{Entry, Store};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

/// The rendition list the client sends for images: KTX2 first. Base64 of
/// `[{"format":"ktx2","majorVersion":"8","fidelity":"/////AE="},{"format":"ktx2","majorVersion":"7rdo","fidelity":"/////AE="}]`.
const KTX2_FIRST: &str = "W3siZm9ybWF0Ijoia3R4MiIsIm1ham9yVmVyc2lvbiI6IjgiLCJmaWRlbGl0eSI6Ii8vLy8vQUU9In0seyJmb3JtYXQiOiJrdHgyIiwibWFqb3JWZXJzaW9uIjoiN3JkbyIsImZpZGVsaXR5IjoiLy8vLy9BRT0ifV0%3D";

/// Largest asset that is downloaded.
const LIMIT: u64 = 256 << 20;
/// Stands in for the status code of a refusal that was remembered instead of asked for again.
const REMEMBERED: u16 = 0;
/// Requests made at the same time.
const WORKERS: usize = 8;

pub enum Fetched {
    Body(Vec<u8>),
    /// The asset needs a login (401) or a permission the public does not have (403).
    Denied(u16),
    /// There is no such asset.
    Gone,
}

/// Downloads an asset by ID as the public sees it. `ktx2` asks for the KTX2
/// rendition the client prefers for images.
pub fn fetch(id: u64, ktx2: bool) -> Result<Fetched> {
    let mut url = format!("https://assetdelivery.roblox.com/v1/asset/?id={id}");
    if ktx2 {
        url.push_str("&contentRepresentationPriorityList=");
        url.push_str(KTX2_FIRST);
    }
    match ureq::get(&url).set("User-Agent", "rbxcache").timeout(Duration::from_secs(40)).call() {
        Ok(response) => {
            let mut body = Vec::new();
            response.into_reader().take(LIMIT).read_to_end(&mut body).with_context(|| format!("downloading asset {id}"))?;
            Ok(Fetched::Body(body))
        }
        Err(ureq::Error::Status(code @ (401 | 403), _)) => Ok(Fetched::Denied(code)),
        Err(ureq::Error::Status(400 | 404, _)) => Ok(Fetched::Gone),
        Err(e) => Err(e).with_context(|| format!("looking up asset {id}")),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Already in the ID map and present in the cache.
    Known,
    /// The public download is byte-for-byte a cached entry.
    Exact,
    /// The KTX2 rendition of the image is byte-for-byte a cached entry.
    Rendition,
    /// Fetched, but nothing in the cache matches. It will match once Roblox downloads it.
    NotCached,
    Denied(u16),
    Gone,
    Failed(String),
}

impl Outcome {
    /// True when the asset is tied to at least one cached entry.
    pub fn matched(&self) -> bool {
        matches!(self, Outcome::Known | Outcome::Exact | Outcome::Rendition)
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Known => write!(f, "already known"),
            Outcome::Exact => write!(f, "matched exactly"),
            Outcome::Rendition => write!(f, "matched its KTX2 rendition"),
            Outcome::NotCached => write!(f, "not in the cache yet"),
            Outcome::Denied(401) => write!(f, "needs a Roblox login"),
            Outcome::Denied(REMEMBERED) => write!(f, "not available without a login (remembered)"),
            Outcome::Denied(_) => write!(f, "not available to the public"),
            Outcome::Gone => write!(f, "no such asset"),
            Outcome::Failed(e) => write!(f, "lookup failed: {e}"),
        }
    }
}

/// Runs `job` over `items` on a few threads, reporting how many are done.
fn parallel<T: Sync, R: Send>(items: &[T], progress: &(dyn Fn(usize, usize) + Sync), job: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let done = std::sync::atomic::AtomicUsize::new(0);
    let chunk = items.len().div_ceil(WORKERS).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                let (job, done) = (&job, &done);
                scope.spawn(move || {
                    part.iter()
                        .map(|item| {
                            let result = job(item);
                            progress(done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1, items.len());
                            result
                        })
                        .collect::<Vec<R>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("lookup thread panicked")).collect()
    })
}

/// Cached entries looked up by the fingerprint of their content.
struct Cache<'a> {
    store: &'a Store,
    entries: &'a [Entry],
    by_size: HashMap<u64, Vec<usize>>,
    md5s: HashMap<usize, String>,
}

impl<'a> Cache<'a> {
    fn new(store: &'a Store, entries: &'a [Entry]) -> Self {
        let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if e.wrapped && !matches!(e.kind, "redirect" | "missing" | "empty") {
                by_size.entry(e.body_len).or_default().push(i);
            }
        }
        Self { store, entries, by_size, md5s: HashMap::new() }
    }

    fn holds(&mut self, fp: &Fingerprint) -> bool {
        let Some(candidates) = self.by_size.get(&fp.size) else { return false };
        for &i in candidates {
            if !self.md5s.contains_key(&i) {
                let Ok(body) = self.store.read_body(&self.entries[i].id) else { continue };
                self.md5s.insert(i, fingerprint(&body).md5);
            }
            if self.md5s[&i] == fp.md5 {
                return true;
            }
        }
        false
    }
}

/// Looks up `ids` and records in `idmap` which content each one has, so that
/// config rules naming them resolve to cached entries. `progress(done, total)`
/// follows the downloads. Returns what happened to each ID, in order.
///
/// An ID is only looked up once: the map remembers its fingerprints, and an
/// empty list for one that was refused or does not exist. `refresh` asks again
/// regardless, for when an asset was updated or made public.
pub fn identify(
    store: &Store,
    entries: &[Entry],
    idmap: &mut IdMap,
    ids: &[u64],
    refresh: bool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Vec<(u64, Outcome)> {
    let mut cache = Cache::new(store, entries);
    let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
    let mut unknown = Vec::new();
    for &id in ids {
        match idmap.assets.get(&id.to_string()).cloned() {
            Some(prints) if prints.iter().any(|fp| cache.holds(fp)) => drop(outcomes.insert(id, Outcome::Known)),
            Some(_) if refresh => unknown.push(id),
            // Looked up before: either its content is known and simply not cached, or it was refused.
            Some(prints) if prints.is_empty() => drop(outcomes.insert(id, Outcome::Denied(REMEMBERED))),
            Some(_) => drop(outcomes.insert(id, Outcome::NotCached)),
            None => unknown.push(id),
        }
    }

    let fetched = parallel(&unknown, progress, |&id| (id, fetch(id, false)));
    let mut images = Vec::new();
    for (id, result) in fetched {
        let outcome = match result {
            Ok(Fetched::Body(body)) => {
                let fp = fingerprint(&body);
                let cached = cache.holds(&fp);
                idmap.add(id, fp);
                if !cached && format::is_image(format::sniff(&body)) {
                    images.push(id);
                }
                if cached {
                    Outcome::Exact
                } else {
                    Outcome::NotCached
                }
            }
            Ok(Fetched::Denied(code)) => {
                idmap.assets.entry(id.to_string()).or_default();
                Outcome::Denied(code)
            }
            Ok(Fetched::Gone) => {
                idmap.assets.entry(id.to_string()).or_default();
                Outcome::Gone
            }
            Err(e) => Outcome::Failed(format!("{e:#}")),
        };
        outcomes.insert(id, outcome);
    }

    // Images are cached as the KTX2 rendition the client asked for.
    for (id, result) in parallel(&images, &|_, _| {}, |&id| (id, fetch(id, true))) {
        if let Ok(Fetched::Body(body)) = result {
            let fp = fingerprint(&body);
            if cache.holds(&fp) {
                outcomes.insert(id, Outcome::Rendition);
            }
            idmap.add(id, fp);
        }
    }
    ids.iter().map(|id| (*id, outcomes.get(id).cloned().unwrap_or(Outcome::NotCached))).collect()
}

/// One line saying how a lookup went overall.
pub fn summarize(outcomes: &[(u64, Outcome)]) -> String {
    let count = |test: fn(&Outcome) -> bool| outcomes.iter().filter(|(_, o)| test(o)).count();
    let mut parts =
        vec![format!("{} of {} matched to cached entries", count(Outcome::matched), format::count(outcomes.len(), "asset ID", "asset IDs"))];
    for (n, what) in [
        (count(|o| *o == Outcome::NotCached), "not in the cache yet"),
        (count(|o| matches!(o, Outcome::Denied(_))), "not available without a login"),
        (count(|o| *o == Outcome::Gone), "no longer exist"),
        (count(|o| matches!(o, Outcome::Failed(_))), "could not be looked up"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_each_kind_of_outcome() {
        let outcomes = [(1, Outcome::Exact), (2, Outcome::Rendition), (3, Outcome::NotCached), (4, Outcome::Denied(401)), (5, Outcome::Denied(403))];
        assert_eq!(summarize(&outcomes), "2 of 5 asset IDs matched to cached entries, 1 not in the cache yet, 2 not available without a login");
        assert_eq!(summarize(&[(1, Outcome::Known)]), "1 of 1 asset ID matched to cached entries");
    }

    #[test]
    fn outcomes_read_as_plain_statements() {
        assert_eq!(Outcome::Denied(401).to_string(), "needs a Roblox login");
        assert_eq!(Outcome::Denied(403).to_string(), "not available to the public");
        assert!(Outcome::Exact.matched() && Outcome::Known.matched() && !Outcome::NotCached.matched());
    }
}
