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
/// The place ID of the game last joined. Tried first when a signed-in request
/// is refused, before the creator's other games.
static PLACE: Mutex<Option<u64>> = Mutex::new(None);
/// Games already listed for a creator, and the place that last unlocked one of
/// their assets. Shared across the lookup threads so a creator is listed once.
static PLACES: std::sync::LazyLock<Mutex<PlaceBook>> = std::sync::LazyLock::new(|| Mutex::new(PlaceBook::new()));

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
    /// The asset needs a login (401) or a permission the current account does not have (403).
    Denied(u16),
    /// There is no such asset.
    Gone,
}

/// Downloads an asset by ID. `ktx2` asks for the KTX2 rendition the client
/// prefers for images. The request is sent with the added account's cookie
/// when one is set, so Roblox answers as that account. When it still refuses
/// a signed-in request, the request is repeated naming a game: the one last
/// joined, then each game of the asset's creator, until one hands the file
/// out. That is how a sound can be fetched without having joined its game.
pub fn fetch(id: u64, ktx2: bool) -> Result<Fetched> {
    let mut url = format!("https://assetdelivery.roblox.com/v1/asset/?id={id}");
    if ktx2 {
        url.push_str("&contentRepresentationPriorityList=");
        url.push_str(KTX2_FIRST);
    }
    let first = get(id, &url, None)?;
    if matches!(first, Fetched::Denied(_)) && signed_in() {
        return with_a_place(id, &url, first);
    }
    Ok(first)
}

/// The place ID of the game last joined, if the logs named one.
fn last_place() -> Option<u64> {
    PLACE.lock().ok().and_then(|place| *place)
}

/// Who made an asset. `kind` is 1 for a user and 2 for a group, as Roblox numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Creator {
    id: u64,
    kind: u8,
}

/// Places already asked about, per creator.
struct PlaceBook {
    listed: HashMap<Creator, Vec<u64>>,
    /// The place that last unlocked an asset of this creator. Tried first next time.
    worked: HashMap<Creator, u64>,
}

impl PlaceBook {
    fn new() -> Self {
        Self { listed: HashMap::new(), worked: HashMap::new() }
    }
}

/// Page sizes tried, largest first. A smaller one is only used when the larger
/// one lists nothing. At most this many of a creator's games are tried.
const PAGE_LIMITS: [u32; 3] = [50, 25, 10];
const MAX_PLACES: usize = 300;

/// Asks again naming games until one hands the file out. `first` is what the
/// request without a place was answered with, and is what comes back when none do.
fn with_a_place(id: u64, url: &str, first: Fetched) -> Result<Fetched> {
    let creator = creator_of(id);
    let remembered = creator.and_then(place_that_worked);
    let listed = creator.map(places_of).unwrap_or_default();
    for place in place_order(last_place(), remembered, &listed) {
        if let Fetched::Body(body) = get(id, url, Some(place))? {
            if let Some(creator) = creator {
                remember_place(creator, place);
            }
            return Ok(Fetched::Body(body));
        }
    }
    Ok(first)
}

/// The places to name, in the order worth trying, with no repeats.
fn place_order(last_joined: Option<u64>, remembered: Option<u64>, listed: &[u64]) -> Vec<u64> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for place in last_joined.into_iter().chain(remembered).chain(listed.iter().copied()) {
        if seen.insert(place) {
            order.push(place);
        }
    }
    order
}

fn place_that_worked(creator: Creator) -> Option<u64> {
    PLACES.lock().ok().and_then(|book| book.worked.get(&creator).copied())
}

fn remember_place(creator: Creator, place: u64) {
    if let Ok(mut book) = PLACES.lock() {
        book.worked.insert(creator, place);
    }
}

/// The creator's games. Listed once per creator and remembered for the rest of the run.
fn places_of(creator: Creator) -> Vec<u64> {
    if let Ok(book) = PLACES.lock() {
        if let Some(places) = book.listed.get(&creator) {
            return places.clone();
        }
    }
    let places = list_places(creator);
    if let Ok(mut book) = PLACES.lock() {
        // Another thread may have listed them while this one was asking.
        book.listed.entry(creator).or_insert(places).clone()
    } else {
        places
    }
}

/// Every root place of a creator's games, up to [`MAX_PLACES`].
fn list_places(creator: Creator) -> Vec<u64> {
    let mut places = Vec::new();
    let mut seen = HashSet::new();
    for limit in PAGE_LIMITS {
        let before = places.len();
        let pages = MAX_PLACES.div_ceil(limit as usize);
        for base in game_list_urls(creator, limit) {
            let mut cursor = String::new();
            for _ in 0..pages.max(1) {
                if places.len() >= MAX_PLACES {
                    return places;
                }
                let url = if cursor.is_empty() { base.clone() } else { format!("{base}&cursor={}", query_escape(&cursor)) };
                let Some(page) = get_json(&url, false) else { break };
                for id in place_ids_in(&page) {
                    if seen.insert(id) {
                        places.push(id);
                    }
                }
                cursor = page["nextPageCursor"].as_str().unwrap_or("").to_string();
                if cursor.is_empty() {
                    break;
                }
            }
        }
        // A page size that listed something is enough; the smaller ones are only for when it did not.
        if places.len() > before {
            break;
        }
    }
    places
}

/// The games.roblox.com addresses that list a creator's games at this page size.
fn game_list_urls(creator: Creator, limit: u32) -> Vec<String> {
    match creator.kind {
        1 => vec![
            format!("https://games.roblox.com/v2/users/{}/games?sortOrder=Asc&limit={limit}", creator.id),
            format!("https://games.roblox.com/v2/users/{}/games?accessFilter=Public&sortOrder=Asc&limit={limit}", creator.id),
        ],
        2 => vec![
            format!("https://games.roblox.com/v2/groups/{}/gamesV2?accessFilter=2&limit={limit}&sortOrder=Asc", creator.id),
            format!("https://games.roblox.com/v2/groups/{}/gamesV2?accessFilter=1&limit={limit}&sortOrder=Asc", creator.id),
        ],
        _ => Vec::new(),
    }
}

/// Who published an asset, from Roblox's catalogue. Needs the signed-in cookie.
fn creator_of(asset: u64) -> Option<Creator> {
    let body = get_json(&format!("https://develop.roblox.com/v1/assets?assetIds={asset}"), true)?;
    creator_from(&body)
}

fn creator_from(body: &serde_json::Value) -> Option<Creator> {
    let item = body.get("data")?.get(0)?;
    let creator = &item["creator"];
    let id = creator["targetId"].as_u64().or_else(|| item["creatorTargetId"].as_u64())?;
    let kind = creator["typeId"].as_u64().or_else(|| item["creatorType"].as_u64())?;
    matches!(kind, 1 | 2).then_some(Creator { id, kind: kind as u8 })
}

/// The root place ID of each game on a games.roblox.com page.
fn place_ids_in(page: &serde_json::Value) -> Vec<u64> {
    page["data"].as_array().map(|games| games.iter().filter_map(|game| game["rootPlace"]["id"].as_u64()).collect()).unwrap_or_default()
}

fn query_escape(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A JSON document, or `None` when Roblox did not send one. `cookie` sends the
/// signed-in account's cookie, which the catalogue requires and the games list does not.
fn get_json(url: &str, cookie: bool) -> Option<serde_json::Value> {
    let mut request = ureq::get(url).set("User-Agent", "rbxcache").set("Accept", "application/json").timeout(Duration::from_secs(15));
    if cookie {
        let auth = AUTH.lock().ok().and_then(|auth| auth.clone())?;
        request = request.set("Cookie", &format!(".ROBLOSECURITY={auth}"));
    }
    serde_json::from_reader(request.call().ok()?.into_reader()).ok()
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
            let mut body = Vec::new();
            response.into_reader().take(LIMIT + 1).read_to_end(&mut body).with_context(|| format!("downloading asset {id}"))?;
            anyhow::ensure!(body.len() as u64 <= LIMIT, "asset {id} is larger than {} MB", LIMIT >> 20);
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
    /// A KTX2 pack of the image is cached.
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
            Outcome::Rendition => write!(f, "matched its texture packs"),
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

    #[test]
    fn the_creator_and_their_places_are_read_out_of_roblox_answers() {
        let body = serde_json::json!({"data": [{"creator": {"targetId": 9, "typeId": 2}, "creatorTargetId": 1, "creatorType": 1}]});
        // The nested creator object wins over the flat fields.
        assert_eq!(creator_from(&body), Some(Creator { id: 9, kind: 2 }));
        assert_eq!(creator_from(&serde_json::json!({"data": [{"creatorTargetId": 4, "creatorType": 1}]})), Some(Creator { id: 4, kind: 1 }));
        assert_eq!(creator_from(&serde_json::json!({"data": [{"creator": {"targetId": 4, "typeId": 3}}]})), None);

        let page = serde_json::json!({"data": [{"rootPlace": {"id": 10}}, {"name": "no place"}, {"rootPlace": {"id": 11}}]});
        assert_eq!(place_ids_in(&page), [10, 11]);
    }

    #[test]
    fn a_creators_games_are_listed_at_the_addresses_roblox_serves() {
        let user = Creator { id: 7, kind: 1 };
        let urls = game_list_urls(user, 50);
        assert_eq!(urls[0], "https://games.roblox.com/v2/users/7/games?sortOrder=Asc&limit=50");
        assert!(urls[1].contains("accessFilter=Public"));
        let group = game_list_urls(Creator { id: 8, kind: 2 }, 25);
        assert!(group[0].contains("/v2/groups/8/gamesV2?accessFilter=2"));
        assert!(group[1].contains("accessFilter=1"));
        assert!(game_list_urls(Creator { id: 1, kind: 0 }, 10).is_empty());
    }

    #[test]
    fn places_are_tried_last_joined_first_then_the_one_that_worked_then_the_list() {
        assert_eq!(place_order(Some(1), Some(2), &[2, 3, 1]), [1, 2, 3]);
        assert_eq!(place_order(None, None, &[4, 4, 5]), [4, 5]);
        assert_eq!(query_escape("a+b/c="), "a%2Bb%2Fc%3D");
    }
}
