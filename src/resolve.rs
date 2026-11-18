//! Working out which cached entries belong to which Roblox asset ID.
//!
//! The cache does not record asset IDs, but Roblox's asset delivery endpoint
//! serves many assets to anyone, with no login. An asset fetched that way is
//! byte-for-byte what the client cached, so its fingerprint identifies the
//! entry. Images are the exception: the client asks for KTX2 renditions, one
//! per group of mip levels. The first of those is fetched as well; it carries
//! the content hash that every other pack of the texture has too (see
//! [`crate::texture`]).
//!
//! Both fingerprints go into the ID map whether or not the asset is cached
//! yet, so an entry Roblox downloads later is recognised when it arrives.
//!
//! Assets that need a login or special permission answer 401 or 403 to a
//! guest. When an account has been added (see [`crate::account`]), its
//! session cookie is sent with every request, so Roblox answers as that
//! account would see it. A refusal is asked again naming a game: the one last
//! joined, then each of the asset's creator's games, until one hands it out.
//! The account is checked to be signed in before a lookup starts (see
//! [`crate::account::check_active`]).

use crate::config::{fingerprint, Fingerprint, IdMap};
use crate::format;
use crate::store::{Entry, Store};
use crate::texture;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Mutex;
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

/// The added account's session cookie. With it, an asset Roblox will not give
/// a guest is asked for again, as that account would see it.
static AUTH: Mutex<Option<String>> = Mutex::new(None);
/// The place ID of the game last joined, named when a signed-in request is
/// refused, the way that game's own client would.
static PLACE: Mutex<Option<u64>> = Mutex::new(None);

/// IDs a guest was refused that have been asked for again since an account was added.
static RETRIED: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

/// Uses `cookie` for the requests that follow, or stops doing so when `None`.
pub fn set_auth(cookie: Option<String>) {
    if let Ok(mut auth) = AUTH.lock() {
        // An account being added is a reason to ask again about what a guest was refused.
        if auth.is_none() && cookie.is_some() {
            if let Ok(mut retried) = RETRIED.lock() {
                *retried = None;
            }
        }
        *auth = cookie;
    }
}

/// True once per account added for an ID that a guest was refused: with an
/// account now set, it is worth asking about again, but not on every lookup after.
pub fn retry_refused(id: u64) -> bool {
    signed_in() && RETRIED.lock().is_ok_and(|mut retried| retried.get_or_insert_with(HashSet::new).insert(id))
}

/// True while a session cookie is set.
pub fn signed_in() -> bool {
    AUTH.lock().map(|auth| auth.is_some()).unwrap_or(false)
}

/// Sets the game named when a signed-in request is refused.
pub fn set_place(place: Option<u64>) {
    if let Ok(mut slot) = PLACE.lock() {
        *slot = place;
    }
}

pub enum Fetched {
    Body(Vec<u8>),
    /// A 3D model or mesh: stopped and thrown away (see [`crate::integrity`]).
    Withheld(crate::integrity::Verdict),
    /// The asset needs a login (401) or a permission the current account does not have (403).
    Denied(u16),
    /// There is no such asset.
    Gone,
}

/// Downloads an asset by ID. `ktx2` asks for the KTX2 rendition the client
/// prefers for images. The request carries the added account's cookie when one
/// is set, so Roblox answers as that account; and when that is still refused,
/// it is repeated once naming the game last joined, the context the real client
/// sends for the game it is in. Assets a game keeps to itself, most sounds
/// among them, stay refused: those are matched by ear instead (see the IDs tab).
pub fn fetch(id: u64, ktx2: bool) -> Result<Fetched> {
    let mut url = format!("https://assetdelivery.roblox.com/v1/asset/?id={id}");
    if ktx2 {
        url.push_str("&contentRepresentationPriorityList=");
        url.push_str(KTX2_FIRST);
    }
    let first = get(id, &url, None)?;
    // Signed in but refused: ask once more as the game last joined would, if the logs named one.
    match (first, signed_in().then(last_place).flatten()) {
        (Fetched::Denied(_), Some(place)) => get(id, &url, Some(place)),
        (first, _) => Ok(first),
    }
}

/// The place ID of the game last joined, if the logs named one.
fn last_place() -> Option<u64> {
    PLACE.lock().ok().and_then(|place| *place)
}

/// One request for an asset, with the added account's cookie when one is set
/// and, when given, the place the request is made on behalf of.
fn get(id: u64, url: &str, place: Option<u64>) -> Result<Fetched> {
    let mut request = ureq::get(url).set("User-Agent", "rbxcache").timeout(Duration::from_secs(40));
    if let Some(cookie) = AUTH.lock().ok().and_then(|auth| auth.clone()) {
        request = request.set("Cookie", &format!(".ROBLOSECURITY={cookie}"));
    }
    if let Some(place) = place {
        request = request.set("Roblox-Place-Id", &place.to_string());
    }
    match request.call() {
        Ok(response) => {
            let asset_type = response.header("roblox-assettypeid").and_then(|v| v.trim().parse().ok());
            let mut reader = response.into_reader().take(LIMIT + 1);
            // The first bytes say whether it is a mesh, so most 3D assets stop before the rest comes.
            let mut body = Vec::new();
            reader.by_ref().take(64).read_to_end(&mut body).with_context(|| format!("downloading asset {id}"))?;
            if let Err(refused) = crate::integrity::screen_start(asset_type, &body) {
                return Ok(Fetched::Withheld(refused.0));
            }
            reader.read_to_end(&mut body).with_context(|| format!("downloading asset {id}"))?;
            anyhow::ensure!(body.len() as u64 <= LIMIT, "asset {id} is larger than {} MB", LIMIT >> 20);
            Ok(match crate::integrity::clear_download(asset_type, body) {
                Ok(body) => Fetched::Body(body),
                Err(refused) => Fetched::Withheld(refused.0),
            })
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
    /// A KTX2 pack of the image is cached.
    Rendition,
    /// Fetched, but nothing in the cache matches. It will match once Roblox downloads it.
    NotCached,
    Denied(u16),
    Gone,
    /// A 3D model or mesh, which rbxcache does not download.
    Withheld,
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
            Outcome::Rendition => write!(f, "matched its texture packs"),
            Outcome::NotCached => write!(f, "not in the cache yet"),
            Outcome::Denied(401) => write!(f, "needs a Roblox login"),
            Outcome::Denied(REMEMBERED) => write!(f, "not available without a login (remembered)"),
            Outcome::Denied(_) => write!(f, "not available to the public"),
            Outcome::Gone => write!(f, "no such asset"),
            Outcome::Withheld => write!(f, "a 3D model, which rbxcache does not download"),
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
    /// Content hashes of the cached texture packs; read when first needed.
    textures: Option<HashSet<String>>,
}

impl<'a> Cache<'a> {
    fn new(store: &'a Store, entries: &'a [Entry]) -> Self {
        let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if e.wrapped && !matches!(e.kind, "redirect" | "missing" | "empty") {
                by_size.entry(e.body_len).or_default().push(i);
            }
        }
        Self { store, entries, by_size, md5s: HashMap::new(), textures: None }
    }

    /// The entry holding exactly this content.
    fn find(&mut self, fp: &Fingerprint) -> Option<usize> {
        for &i in self.by_size.get(&fp.size)? {
            if !self.md5s.contains_key(&i) {
                let Ok(body) = self.store.read_body(&self.entries[i].id) else { continue };
                self.md5s.insert(i, fingerprint(&body).md5);
            }
            if self.md5s[&i] == fp.md5 {
                return Some(i);
            }
        }
        None
    }

    /// True when the cache has this content, or any pack of the texture it belongs to.
    fn holds(&mut self, fp: &Fingerprint) -> bool {
        if self.find(fp).is_some() {
            return true;
        }
        let Some(hash) = &fp.texture else { return false };
        let (store, entries) = (self.store, self.entries);
        self.textures.get_or_insert_with(|| entries.iter().filter_map(|e| texture::hash_of(store, e)).collect()).contains(hash)
    }

    /// The same fingerprint with its texture's content hash filled in, when it
    /// lacks one and the pack it describes is cached. Maps written before
    /// packs were understood are completed this way.
    fn completed(&mut self, fp: &Fingerprint) -> Option<Fingerprint> {
        if fp.texture.is_some() {
            return None;
        }
        let i = self.find(fp)?;
        let texture = texture::hash_of(self.store, &self.entries[i])?;
        Some(Fingerprint { texture: Some(texture), ..fp.clone() })
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
        for fp in idmap.assets.get(&id.to_string()).cloned().unwrap_or_default() {
            if let Some(fp) = cache.completed(&fp) {
                idmap.add(id, fp);
            }
        }
        match idmap.assets.get(&id.to_string()).cloned() {
            Some(prints) if prints.iter().any(|fp| cache.holds(fp)) => drop(outcomes.insert(id, Outcome::Known)),
            Some(_) if refresh => unknown.push(id),
            // Refused to a guest, but an account is signed in now.
            Some(prints) if prints.is_empty() && retry_refused(id) => unknown.push(id),
            // Looked up before: either its content is known and simply not cached, or it was refused.
            Some(prints) if prints.is_empty() => drop(outcomes.insert(id, Outcome::Denied(REMEMBERED))),
            Some(_) => drop(outcomes.insert(id, Outcome::NotCached)),
            None => unknown.push(id),
        }
    }

    let fetched = parallel(&unknown, progress, |&id| (id, fetch(id, false)));
    // Images are cached as the KTX2 packs the client asks for, not as the file
    // itself. Their fingerprint waits until the first pack has been asked for
    // too: recorded on its own it would mark the ID as looked up, and a pack
    // request that failed would never be made again.
    let mut images: Vec<(u64, Fingerprint)> = Vec::new();
    for (id, result) in fetched {
        let outcome = match result {
            Ok(Fetched::Body(body)) => {
                let fp = fingerprint(&body);
                if cache.holds(&fp) {
                    idmap.add(id, fp);
                    Outcome::Exact
                } else if format::is_image(format::sniff(&body)) {
                    images.push((id, fp));
                    continue;
                } else {
                    idmap.add(id, fp);
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
            Ok(Fetched::Withheld(_)) => Outcome::Withheld,
            Err(e) => Outcome::Failed(format!("{e:#}")),
        };
        outcomes.insert(id, outcome);
    }

    for ((id, original), result) in parallel(&images, &|_, _| {}, |(id, original)| ((*id, original.clone()), fetch(*id, true))) {
        let outcome = match result {
            Ok(Fetched::Body(body)) => {
                let pack = fingerprint(&body);
                let cached = cache.holds(&pack);
                idmap.add(id, original);
                idmap.add(id, pack);
                if cached {
                    Outcome::Rendition
                } else {
                    Outcome::NotCached
                }
            }
            // There is no pack to be had, so the file itself is all that can be known.
            Ok(Fetched::Denied(_) | Fetched::Gone) => {
                idmap.add(id, original);
                Outcome::NotCached
            }
            Ok(Fetched::Withheld(_)) => Outcome::Withheld,
            Err(e) => Outcome::Failed(format!("{e:#}")),
        };
        outcomes.insert(id, outcome);
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
    fn a_refusal_is_asked_about_again_once_per_sign_in() {
        set_auth(None);
        assert!(!retry_refused(7), "a guest is never retried");
        set_auth(Some("token".into()));
        assert!(retry_refused(7));
        assert!(!retry_refused(7), "once is enough until the next sign-in");
        assert!(retry_refused(8));
        set_auth(None);
        set_auth(Some("token".into()));
        assert!(retry_refused(7), "a new sign-in asks again");
        set_auth(None);
    }

    #[test]
    fn outcomes_read_as_plain_statements() {
        assert_eq!(Outcome::Denied(401).to_string(), "needs a Roblox login");
        assert_eq!(Outcome::Denied(403).to_string(), "not available to the public");
        assert!(Outcome::Exact.matched() && Outcome::Known.matched() && !Outcome::NotCached.matched());
    }
}
