//! Fleasion-style replacement configs, applied directly to the cache.
//!
//! A config is JSON: `{"replacement_rules": [rule, ...]}` (or just the list).
//! Each rule has `replace_ids`, a `mode` (`id`, `remove`, `local`, `cdn`) and the
//! matching `with_id` / `local_path` / `cdn_url`. Rules can be nested in
//! `{"type": "group", "children": [...]}` and switched off with `"enabled": false`.
//!
//! The cache does not record which Roblox asset ID an entry belongs to, so numeric
//! asset IDs are resolved through an [`IdMap`]: asset ID -> fingerprint (MD5 and
//! size) of the asset's content. Fingerprints do not depend on the machine, so an
//! ID map can be shared.

use crate::format;
use crate::originals::Originals;
use crate::store::{hex, Entry, Id, Store};
use crate::texture::{self, Pack};
use anyhow::{bail, ensure, Context, Result};
use image::RgbaImage;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A cache entry, by its 32-digit hex id.
    Entry(Id),
    /// A Roblox asset ID, resolved through the ID map.
    Asset(u64),
    /// Every cached asset of a type, e.g. `Audio`.
    Type(String),
    /// Fleasion syntax that has no cache equivalent (texture-pack slots and so on).
    Unsupported(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Entry(id) => write!(f, "entry {}", hex(id)),
            Target::Asset(n) => write!(f, "asset {n}"),
            Target::Type(t) => write!(f, "type {t}"),
            Target::Unsupported(s) => write!(f, "'{s}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Remove,
    Local(PathBuf),
    Cdn(String),
    With(Target),
    /// The rule is malformed; it is reported with this reason and changes nothing.
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub targets: Vec<Target>,
    pub action: Action,
}

fn parse_target(v: &Value) -> Option<Target> {
    match v {
        Value::Number(n) => n.as_u64().filter(|&n| n > 0).map(Target::Asset),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
                let mut id = [0u8; 16];
                for (i, b) in id.iter_mut().enumerate() {
                    *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
                }
                Some(Target::Entry(id))
            } else if s.bytes().all(|c| c.is_ascii_digit()) {
                s.parse().ok().filter(|&n: &u64| n > 0).map(Target::Asset)
            } else if s.contains(':') {
                Some(Target::Unsupported(s.to_string()))
            } else {
                Some(Target::Type(s.to_ascii_lowercase()))
            }
        }
        _ => None,
    }
}

fn collect_rules(entries: &[Value], base: &Path, out: &mut Vec<Rule>) {
    for entry in entries {
        let Some(obj) = entry.as_object() else { continue };
        if obj.get("type").and_then(Value::as_str) == Some("group") {
            if obj.get("enabled").and_then(Value::as_bool) != Some(false) {
                if let Some(children) = obj.get("children").and_then(Value::as_array) {
                    collect_rules(children, base, out);
                }
            }
            continue;
        }
        if obj.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let targets: Vec<Target> =
            obj.get("replace_ids").and_then(Value::as_array).map(|a| a.iter().filter_map(parse_target).collect()).unwrap_or_default();
        if targets.is_empty() {
            continue;
        }
        let text = |key: &str| obj.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        // Older configs use a `remove` flag instead of a mode.
        let mode = match (text("mode"), obj.get("remove").and_then(Value::as_bool)) {
            (Some(m), _) => m.to_ascii_lowercase(),
            (None, Some(true)) => "remove".to_string(),
            _ => "id".to_string(),
        };
        // As in Fleasion, a rule with no replacement given removes its targets.
        let action = match mode.as_str() {
            "remove" => Action::Remove,
            "local" => match text("local_path") {
                Some(p) => {
                    let p = PathBuf::from(p);
                    Action::Local(if p.is_absolute() { p } else { base.join(p) })
                }
                None => Action::Remove,
            },
            "cdn" => text("cdn_url").map_or(Action::Remove, |u| Action::Cdn(u.to_string())),
            "id" => match obj.get("with_id") {
                None | Some(Value::Null) => Action::Remove,
                Some(Value::String(s)) if s.trim().is_empty() => Action::Remove,
                // 0 and 1 are placeholder IDs in Fleasion configs: the rule does nothing.
                Some(v) if matches!(v.as_u64(), Some(0 | 1)) || matches!(v.as_str().map(str::trim), Some("0" | "1")) => continue,
                Some(v) => match parse_target(v) {
                    Some(t @ (Target::Asset(_) | Target::Entry(_))) => Action::With(t),
                    _ => Action::Invalid(format!("with_id {v} is not an asset ID or a cache entry")),
                },
            },
            other => Action::Invalid(format!("unknown mode '{other}'")),
        };
        let name = text("name").map_or_else(|| format!("rule {}", out.len() + 1), str::to_string);
        out.push(Rule { name, targets, action });
    }
}

/// Parses a config. Relative `local_path` values are taken relative to `base`.
pub fn parse(text: &str, base: &Path) -> Result<Vec<Rule>> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).context("the config is not valid JSON")?;
    let list = match &value {
        Value::Array(list) => list.as_slice(),
        Value::Object(obj) => obj.get("replacement_rules").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
        _ => bail!("the config must be a JSON object or list"),
    };
    let mut rules = Vec::new();
    collect_rules(list, base, &mut rules);
    Ok(rules)
}

/// Every asset ID the rules mention, as targets or as replacements, once each.
pub fn asset_ids(rules: &[Rule]) -> Vec<u64> {
    let mut ids = Vec::new();
    for rule in rules {
        let replacement = match &rule.action {
            Action::With(target) => Some(target),
            _ => None,
        };
        for target in rule.targets.iter().chain(replacement) {
            if let Target::Asset(id) = target {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
        }
    }
    ids
}

/// The name a config goes by: its file name without the extension.
pub fn name_of(path: &Path) -> String {
    path.file_stem().map_or_else(|| "config".to_string(), |n| n.to_string_lossy().into_owned())
}

/// Joins the rules of several configs into one list, in order, so they can be
/// run together. Where two rules set the same entry, the later one has it.
/// With more than one config, each rule's name is prefixed with its config's.
pub fn combine<'a>(configs: impl IntoIterator<Item = (&'a str, &'a [Rule])>) -> Vec<Rule> {
    let configs: Vec<_> = configs.into_iter().collect();
    let several = configs.len() > 1;
    let mut rules = Vec::new();
    for (name, config) in configs {
        rules.extend(config.iter().cloned().map(|mut rule| {
            if several {
                rule.name = format!("{name} / {}", rule.name);
            }
            rule
        }));
    }
    rules
}

/// True when the two paths name the same file, however each is spelled.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Loads several config files as one list of rules; see [`combine`]. A file
/// given more than once counts once, at its first place.
pub fn load_all(paths: &[PathBuf]) -> Result<Vec<Rule>> {
    let mut unique: Vec<&PathBuf> = Vec::new();
    for path in paths {
        if !unique.iter().any(|seen| same_file(seen, path)) {
            unique.push(path);
        }
    }
    let loaded: Vec<(String, Vec<Rule>)> = unique.into_iter().map(|p| Ok((name_of(p), load(p)?))).collect::<Result<_>>()?;
    Ok(combine(loaded.iter().map(|(name, rules)| (name.as_str(), rules.as_slice()))))
}

pub fn load(path: &Path) -> Result<Vec<Rule>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text, path.parent().unwrap_or(Path::new("."))).with_context(|| format!("in {}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub md5: String,
    pub size: u64,
    /// For a texture pack: the content hash shared by every pack of the texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
}

impl Fingerprint {
    fn same_content(&self, other: &Fingerprint) -> bool {
        self.md5 == other.md5 && self.size == other.size
    }
}

pub fn fingerprint(body: &[u8]) -> Fingerprint {
    Fingerprint { md5: hex(&Md5::digest(body)), size: body.len() as u64, texture: texture::content_hash(body) }
}

/// Asset ID -> content fingerprints. One asset can have several (image formats, etc).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IdMap {
    #[serde(flatten)]
    pub assets: BTreeMap<String, Vec<Fingerprint>>,
}

impl IdMap {
    /// `%APPDATA%\rbxcache\idmap.json` on Windows; elsewhere `idmap.json` in the app's data folder.
    pub fn default_path() -> PathBuf {
        #[cfg(windows)]
        let folder = std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("rbxcache"));
        #[cfg(not(windows))]
        let folder = crate::protection::data_dir().ok();
        folder.unwrap_or_else(|| PathBuf::from(".")).join("idmap.json")
    }

    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("{} is not a valid ID map", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Written beside the map and renamed over it, so a failed write cannot truncate it.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    }

    /// Records that `asset` has this content. Returns false if it was already known.
    pub fn add(&mut self, asset: u64, fp: Fingerprint) -> bool {
        let list = self.assets.entry(asset.to_string()).or_default();
        match list.iter_mut().find(|known| known.same_content(&fp)) {
            Some(known) if known.texture.is_some() || fp.texture.is_none() => false,
            Some(known) => {
                known.texture = fp.texture;
                true
            }
            None => {
                list.push(fp);
                true
            }
        }
    }

    /// The assets this content belongs to: the same bytes, or a pack of the same texture.
    pub fn assets_for(&self, fp: &Fingerprint) -> Vec<&str> {
        let matches = |known: &Fingerprint| known.same_content(fp) || (fp.texture.is_some() && known.texture == fp.texture);
        self.assets.iter().filter(|(_, list)| list.iter().any(matches)).map(|(id, _)| id.as_str()).collect()
    }

    /// Takes every fingerprint of `other` that this map does not have. Returns how many assets gained one.
    pub fn merge(&mut self, other: &IdMap) -> usize {
        let mut gained = 0;
        for (asset, prints) in &other.assets {
            let Ok(asset) = asset.parse::<u64>() else { continue };
            let mut any = false;
            for fp in prints {
                any |= self.add(asset, fp.clone());
            }
            gained += usize::from(any);
        }
        gained
    }
}

/// Test for the content kinds covered by a Fleasion asset-type name.
fn kinds_for_type(name: &str) -> Option<fn(&str) -> bool> {
    Some(match name {
        "image" | "decal" | "texture" => format::is_image,
        "audio" | "sound" => format::is_audio,
        "mesh" | "meshpart" => |kind| kind == "mesh",
        "video" => |kind| matches!(kind, "webm" | "mp4"),
        "font" | "fontface" => |kind| matches!(kind, "ttf" | "otf"),
        _ => return None,
    })
}

struct Resolver<'a> {
    store: &'a Store,
    entries: &'a [Entry],
    idmap: &'a IdMap,
    /// With these, an entry that holds a replacement still counts as the asset it was.
    originals: Option<&'a Originals>,
    by_id: HashMap<Id, usize>,
    by_size: HashMap<u64, Vec<usize>>,
    md5s: RefCell<HashMap<usize, String>>,
    /// Texture content hash -> the cached packs of that texture; built when first needed.
    textures: OnceCell<HashMap<String, Vec<usize>>>,
}

impl<'a> Resolver<'a> {
    fn new(store: &'a Store, entries: &'a [Entry], idmap: &'a IdMap, originals: Option<&'a Originals>) -> Self {
        let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if !e.wrapped || e.kind == "redirect" {
                continue;
            }
            // A replaced entry is found by the size its original had.
            let original = originals.and_then(|originals| originals.probably_replaced(e)).map(|record| record.original.size);
            if let Some(size) = original.filter(|size| *size != e.body_len) {
                by_size.entry(size).or_default().push(i);
            }
            if replaceable(e) || original.is_some() {
                by_size.entry(e.body_len).or_default().push(i);
            }
        }
        Self {
            store,
            entries,
            idmap,
            originals,
            by_id: entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect(),
            by_size,
            md5s: RefCell::default(),
            textures: OnceCell::new(),
        }
    }

    fn md5(&self, i: usize) -> Result<String> {
        if let Some(m) = self.md5s.borrow().get(&i) {
            return Ok(m.clone());
        }
        let current = fingerprint(&self.store.read_body(&self.entries[i].id)?);
        // An entry that still holds our replacement answers with the fingerprint of what it replaced.
        let m = match self.originals.and_then(|originals| originals.replaced(&self.entries[i].id, &current)) {
            Some(record) => record.original.md5.clone(),
            None => current.md5,
        };
        self.md5s.borrow_mut().insert(i, m.clone());
        Ok(m)
    }

    /// The content of an entry as Roblox wrote it: what was saved from it when
    /// it holds a replacement, otherwise what it holds.
    fn content(&self, i: usize) -> Result<Vec<u8>> {
        let id = &self.entries[i].id;
        let current = self.store.read_body(id)?;
        match self.originals.filter(|originals| originals.replaced(id, &fingerprint(&current)).is_some()) {
            Some(originals) => originals.original_body(id),
            None => Ok(current),
        }
    }

    /// The kind of content an entry holds, or held before it was replaced.
    fn kind(&self, i: usize) -> &str {
        match self.originals.and_then(|originals| originals.probably_replaced(&self.entries[i])) {
            Some(record) => &record.kind,
            None => self.entries[i].kind,
        }
    }

    fn textures(&self) -> &HashMap<String, Vec<usize>> {
        self.textures.get_or_init(|| {
            let mut packs: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, e) in self.entries.iter().enumerate() {
                if let Some(hash) = texture::hash_of(self.store, e) {
                    packs.entry(hash).or_default().push(i);
                }
            }
            packs
        })
    }

    /// Cache entries a target stands for, or why there are none.
    fn resolve(&self, target: &Target) -> Result<Vec<usize>, Miss> {
        match target {
            Target::Entry(id) => match self.by_id.get(id) {
                // One pack of a texture stands for all of them.
                Some(&i) => Ok(match texture::hash_of(self.store, &self.entries[i]) {
                    Some(hash) => self.textures().get(&hash).cloned().unwrap_or_else(|| vec![i]),
                    None => vec![i],
                }),
                None => Err(Miss::Problem(format!("{target} is not in the cache"))),
            },
            Target::Type(name) => {
                let covers =
                    kinds_for_type(name).ok_or_else(|| Miss::Problem(format!("asset type '{name}' cannot be recognised from cached content")))?;
                Ok((0..self.entries.len()).filter(|&i| self.entries[i].wrapped && covers(self.kind(i))).collect())
            }
            Target::Asset(n) => {
                let prints = self.idmap.assets.get(&n.to_string()).ok_or_else(|| Miss::Problem(format!("{target} was never looked up")))?;
                let mut found = Vec::new();
                for fp in prints {
                    for &i in self.by_size.get(&fp.size).map_or(&[][..], Vec::as_slice) {
                        if self.md5(i).map_err(|e| Miss::Problem(format!("{e:#}")))? == fp.md5 {
                            found.push(i);
                        }
                    }
                }
                // A texture is cached as several packs. Matching one of them, or knowing
                // the texture's content hash, brings in the rest.
                let mut hashes: Vec<String> = prints.iter().filter_map(|fp| fp.texture.clone()).collect();
                hashes.extend(found.iter().filter_map(|&i| texture::hash_of(self.store, &self.entries[i])));
                for hash in &hashes {
                    found.extend(self.textures().get(hash).map_or(&[][..], Vec::as_slice));
                }
                found.sort_unstable();
                found.dedup();
                match (found.is_empty(), prints.is_empty()) {
                    (false, _) => Ok(found),
                    (true, true) => Err(Miss::Refused),
                    (true, false) => Err(Miss::Waiting),
                }
            }
            Target::Unsupported(s) => Err(Miss::Problem(format!("'{s}' is Fleasion proxy syntax with no cache equivalent"))),
        }
    }
}

/// Why a target stands for no cache entry.
enum Miss {
    /// The asset is known, but Roblox has not downloaded it, or it was already replaced.
    Waiting,
    /// Roblox would not hand the asset out (it needs a login, or no longer exists), so nothing can be matched to it.
    Refused,
    Problem(String),
}

/// Redirect records and missing files hold no content to swap.
fn replaceable(e: &Entry) -> bool {
    e.wrapped && !matches!(e.kind, "redirect" | "missing")
}

#[derive(Debug, Default)]
pub struct RuleReport {
    pub name: String,
    /// Cache entries the rule's targets resolved to.
    pub matched: usize,
    /// Entries whose content was (or, on a dry run, would be) changed.
    pub changed: usize,
    /// The entries as they are after being rewritten; empty on a dry run.
    pub written: Vec<Entry>,
    /// Targets that are known but not in the cache: not downloaded yet, or already replaced.
    pub waiting: usize,
    /// Targets Roblox would not hand out: they need a login, or no longer exist.
    pub refused: usize,
    /// What went wrong, as opposed to what is merely not there.
    pub notes: Vec<String>,
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let response =
        ureq::get(url).set("User-Agent", "rbxcache").timeout(Duration::from_secs(60)).call().with_context(|| format!("downloading {url}"))?;
    const LIMIT: u64 = 256 << 20;
    let mut body = Vec::new();
    response.into_reader().take(LIMIT + 1).read_to_end(&mut body)?;
    if body.len() as u64 > LIMIT {
        bail!("{url} is larger than {} MB", LIMIT >> 20);
    }
    Ok(body)
}

/// What asking for a file came to.
enum Got {
    Body(Arc<Vec<u8>>),
    /// Roblox would not hand the asset out, or has no such asset.
    Unavailable,
    /// The request itself failed, at this moment.
    Failed(String, Instant),
}

/// How long a failed download is taken as read before it is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Downloads made while the app runs, kept so that re-applying a config to
/// what Roblox has just written does not fetch the same files again. A
/// failure is kept too, for a minute, so a dead address is not asked about
/// for every entry that arrives.
static DOWNLOADS: Mutex<Option<HashMap<String, Got>>> = Mutex::new(None);

/// Fetches `key` once. `Ok(None)` means there is nothing to be had.
fn obtain(key: &str, fetch: impl FnOnce() -> Result<Option<Vec<u8>>>) -> Result<Option<Arc<Vec<u8>>>> {
    let known = DOWNLOADS.lock().ok().and_then(|downloads| {
        Some(match downloads.as_ref()?.get(key)? {
            Got::Body(body) => Ok(Some(body.clone())),
            Got::Unavailable => Ok(None),
            Got::Failed(why, at) if at.elapsed() < RETRY_AFTER => Err(anyhow::anyhow!("{why}")),
            Got::Failed(..) => return None,
        })
    });
    if let Some(known) = known {
        return known;
    }
    let (got, result) = match fetch() {
        Ok(Some(body)) => {
            let body = Arc::new(body);
            (Got::Body(body.clone()), Ok(Some(body)))
        }
        Ok(None) => (Got::Unavailable, Ok(None)),
        Err(e) => (Got::Failed(format!("{e:#}"), Instant::now()), Err(e)),
    };
    if let Ok(mut downloads) = DOWNLOADS.lock() {
        downloads.get_or_insert_with(HashMap::new).insert(key.to_string(), got);
    }
    result
}

/// The original file of a public asset, as the replacement for another.
/// `Ok(None)` when Roblox will not give it to someone who is not logged in.
fn download_asset(id: u64) -> Result<Option<Arc<Vec<u8>>>> {
    obtain(&format!("asset:{id}"), || match crate::resolve::fetch(id, false)? {
        crate::resolve::Fetched::Body(body) => Ok(Some(body)),
        crate::resolve::Fetched::Denied(_) | crate::resolve::Fetched::Gone => Ok(None),
    })
}

/// Replacement content, with the picture it holds decoded at most once.
struct Payload {
    bytes: Arc<Vec<u8>>,
    digest: [u8; 16],
    image: OnceCell<Option<RgbaImage>>,
}

impl Payload {
    fn new(bytes: Arc<Vec<u8>>) -> Self {
        Self { digest: Md5::digest(&bytes[..]).into(), bytes, image: OnceCell::new() }
    }

    /// The payload as a picture. Nothing at all counts as a fully transparent one.
    fn image(&self) -> Option<&RgbaImage> {
        let decode = || match format::sniff(&self.bytes) {
            _ if self.bytes.is_empty() => Some(texture::blank()),
            "png" | "jpeg" | "gif" | "webp" => image::load_from_memory(&self.bytes).ok().map(|i| i.to_rgba8()),
            "ktx2" => {
                let t = crate::viewers::decode_ktx2(&self.bytes, u32::MAX).ok()?;
                RgbaImage::from_raw(t.width as u32, t.height as u32, t.pixels)
            }
            _ => None,
        };
        self.image.get_or_init(decode).as_ref()
    }

    /// The bytes to store in an entry of this kind that now holds `current`.
    /// A texture pack gets the picture at its own size and in its own format,
    /// and an image entry never gets a KTX2 file or nothing at all; everything
    /// else takes the payload as it is.
    fn fit(&self, kind: &str, current: &[u8]) -> Cow<'_, [u8]> {
        let raw = Cow::Borrowed(&self.bytes[..]);
        if !format::is_image(kind) {
            return raw;
        }
        let Some(image) = self.image() else { return raw };
        let foreign = self.bytes.is_empty() || format::sniff(&self.bytes) == "ktx2";
        match (kind, Pack::parse(current)) {
            ("ktx2", Some(pack)) => pack.refit(image).into(),
            // A KTX2 file that is not one of Roblox's packs: an uncompressed texture stands in.
            ("ktx2", None) if format::sniff(&self.bytes) != "ktx2" => {
                crate::viewers::encode_ktx2(image.width(), image.height(), image.as_raw()).map_or(raw, Cow::Owned)
            }
            ("png" | "jpeg" | "gif" | "webp", _) if foreign => {
                let mut png = std::io::Cursor::new(Vec::new());
                match image.write_to(&mut png, image::ImageFormat::Png) {
                    Ok(()) => png.into_inner().into(),
                    Err(_) => raw,
                }
            }
            _ => raw,
        }
    }

    /// True when `current` already is what [`Payload::fit`] would store.
    fn held_by(&self, kind: &str, current: &[u8]) -> bool {
        if kind == "ktx2" {
            if let (Some(pack), Some(image)) = (Pack::parse(current), self.image()) {
                return <[u8; 16]>::from(Md5::digest(current)) == texture::refit_digest(&pack, &self.digest, image);
            }
        }
        current == &*self.fit(kind, current)
    }
}

/// The cached entries that hold the same asset as `id`: the entry itself and,
/// when it is a texture pack, the other packs of that texture. The largest comes first.
pub fn siblings(store: &Store, entries: &[Entry], id: &Id) -> Vec<Id> {
    let hash = entries.iter().find(|e| e.id == *id).and_then(|e| texture::hash_of(store, e));
    let mut found: Vec<&Entry> = match hash {
        Some(hash) => entries.iter().filter(|e| texture::hash_of(store, e).as_ref() == Some(&hash)).collect(),
        None => entries.iter().filter(|e| e.id == *id).collect(),
    };
    found.sort_by_key(|e| std::cmp::Reverse(e.body_len));
    found.into_iter().map(|e| e.id).collect()
}

/// Replaces the content of `id` by hand with `payload`, fitted as a config
/// rule's would be, in every cached pack when the entry is a texture.
/// Returns each rewritten entry with the bytes it now holds.
pub fn replace_by_hand(store: &Store, entries: &[Entry], id: &Id, payload: Vec<u8>) -> Result<Vec<(Entry, Vec<u8>)>> {
    let payload = Payload::new(Arc::new(payload));
    let mut written = Vec::new();
    for target in siblings(store, entries, id) {
        let kind = entries.iter().find(|e| e.id == target).map_or("binary", |e| e.kind);
        let body = payload.fit(kind, &store.read_body(&target)?).into_owned();
        written.push((store.replace_body(&target, &body)?, body));
    }
    ensure!(!written.is_empty(), "no entry {}", hex(id));
    Ok(written)
}

/// What one rule will do, worked out before anything is written.
struct Plan {
    report: RuleReport,
    /// Every entry the rule would set, whether or not it already holds the payload.
    claims: Vec<usize>,
    /// The claimed entries whose content differs from `payload`.
    writes: Vec<usize>,
    payload: Option<Payload>,
}

impl Plan {
    fn new(name: &str) -> Self {
        Self { report: RuleReport { name: name.to_string(), ..Default::default() }, claims: Vec::new(), writes: Vec::new(), payload: None }
    }
}

/// Runs `rules` against the cache. With `apply` false nothing is written; the
/// report shows what would happen. Files the rules need are downloaded either way.
///
/// Every rule is resolved against the cache as it was at the start, so rules
/// that swap two assets with each other see the original content of both.
/// `only` limits the entries that may be changed, for re-applying a config to
/// the few entries Roblox has just written. With `originals`, what each entry
/// held is saved before it is replaced, and entries replaced earlier are
/// still recognised as the assets they were.
pub fn run(
    store: &Store,
    entries: &[Entry],
    idmap: &IdMap,
    rules: &[Rule],
    apply: bool,
    only: Option<&HashSet<Id>>,
    originals: Option<&mut Originals>,
) -> Vec<RuleReport> {
    execute(store, entries, idmap, rules, apply, only, originals).0
}

/// Makes the cache match `rules` and nothing else: applies them, then puts
/// back the original of every entry that was replaced before and that no rule
/// sets any more. This is what turning a config or a rule off comes down to.
/// Returns the report of each rule and the entries that were put back.
pub fn sync(store: &Store, entries: &[Entry], idmap: &IdMap, rules: &[Rule], originals: &mut Originals) -> (Vec<RuleReport>, Vec<Entry>) {
    let (mut reports, claimed) = execute(store, entries, idmap, rules, true, None, Some(originals));
    let restored = originals.restore_unclaimed(store, &claimed);
    if let Err(e) = originals.save() {
        reports.push(RuleReport { name: "originals".into(), notes: vec![format!("{e:#}")], ..Default::default() });
    }
    (reports, restored)
}

/// [`run`], also returning every entry some rule sets, changed or not.
fn execute(
    store: &Store,
    entries: &[Entry],
    idmap: &IdMap,
    rules: &[Rule],
    apply: bool,
    only: Option<&HashSet<Id>>,
    mut originals: Option<&mut Originals>,
) -> (Vec<RuleReport>, HashSet<Id>) {
    let resolver = Resolver::new(store, entries, idmap, originals.as_deref());
    let mut plans = Vec::new();
    for rule in rules {
        let mut plan = Plan::new(&rule.name);
        let mut matched: Vec<usize> = Vec::new();
        for target in &rule.targets {
            match resolver.resolve(target) {
                Ok(found) => matched.extend(found),
                Err(Miss::Waiting) => plan.report.waiting += 1,
                Err(Miss::Refused) => plan.report.refused += 1,
                Err(Miss::Problem(why)) => plan.report.notes.push(why),
            }
        }
        matched.sort_unstable();
        matched.dedup();
        if let Some(only) = only {
            matched.retain(|&i| only.contains(&entries[i].id));
        }
        plan.report.matched = matched.len();
        let payload: Result<Option<Arc<Vec<u8>>>> = match &rule.action {
            Action::Invalid(why) => Err(anyhow::anyhow!("{why}")),
            _ if matched.is_empty() => Ok(None),
            Action::Remove => Ok(Some(Arc::default())),
            Action::Local(path) => std::fs::read(path).map(|body| Some(Arc::new(body))).with_context(|| format!("reading {}", path.display())),
            Action::Cdn(url) => obtain(url, || fetch(url).map(Some)),
            Action::With(target) => {
                let sources = resolver.resolve(target).ok().unwrap_or_default();
                // The largest cached copy: of a texture's packs, the one with the most detail.
                let cached = sources.iter().copied().max_by_key(|&i| entries[i].body_len);
                let read = |i: usize| resolver.content(i).map(|body| Some(Arc::new(body)));
                let unavailable = || anyhow::anyhow!("replacement {target} is not in the cache, and Roblox would not hand it out");
                match (target, cached) {
                    // A cached pack is only part of a texture, so the original is used when Roblox gives it
                    // out. A request that fails is an error, not a reason to settle for the pack: the two
                    // give different pictures, and the entry would be rewritten whenever the network came and went.
                    (Target::Asset(id), Some(i)) if entries[i].kind == "ktx2" => download_asset(*id).and_then(|got| match got {
                        Some(body) => Ok(Some(body)),
                        None => read(i),
                    }),
                    (_, Some(i)) => read(i),
                    (Target::Asset(id), None) => download_asset(*id).and_then(|got| got.map(Some).ok_or_else(unavailable)),
                    _ => Err(anyhow::anyhow!("replacement {target} matches nothing in the cache")),
                }
            }
        };
        match payload {
            // The rule still has its entries: one that could not be worked out
            // this time must not look as if it had been switched off.
            Err(e) => {
                plan.report.notes.push(format!("{e:#}"));
                plan.claims = matched;
            }
            Ok(None) => {}
            Ok(Some(payload)) => {
                let payload = Payload::new(payload);
                for &i in &matched {
                    match store.read_body(&entries[i].id) {
                        Ok(current) if payload.held_by(entries[i].kind, &current) => {}
                        Ok(_) => plan.writes.push(i),
                        Err(e) => plan.report.notes.push(format!("{}: {e:#}", entries[i].id_hex)),
                    }
                }
                plan.claims = matched;
                plan.payload = Some(payload);
            }
        }
        plans.push(plan);
    }
    // Where several rules set the same entry, the last one has it. An earlier rule
    // must not write there even when the entry already holds the last rule's
    // content, or the two would undo each other on every run.
    let mut taken: HashSet<usize> = HashSet::new();
    for plan in plans.iter_mut().rev() {
        plan.writes.retain(|i| !taken.contains(i));
        plan.report.changed = plan.writes.len();
        taken.extend(plan.claims.iter().copied());
    }
    // The resolver read the originals; from here on they are written.
    let kinds: HashMap<usize, String> = taken.iter().map(|&i| (i, resolver.kind(i).to_string())).collect();
    drop(resolver);
    if apply {
        for plan in &mut plans {
            let Some(payload) = &plan.payload else { continue };
            for &i in &plan.writes {
                let id = &entries[i].id;
                let written = store.read_body(id).and_then(|current| {
                    let fitted = payload.fit(entries[i].kind, &current);
                    // Nothing is replaced unless what it held could be saved first.
                    if let Some(originals) = originals.as_deref_mut() {
                        originals.keep(id, &kinds[&i], &current, &fitted)?;
                    }
                    store.replace_body(id, &fitted)
                });
                match written {
                    Ok(entry) => plan.report.written.push(entry),
                    Err(e) => {
                        plan.report.changed -= 1;
                        plan.report.notes.push(format!("{}: {e:#}", entries[i].id_hex));
                    }
                }
            }
        }
        if let Some(Err(e)) = originals.as_deref().map(Originals::save) {
            if let Some(plan) = plans.last_mut() {
                plan.report.notes.push(format!("{e:#}"));
            }
        }
    }
    let claimed = taken.iter().map(|&i| entries[i].id).collect();
    (plans.into_iter().map(|plan| plan.report).collect(), claimed)
}

/// One-line summary of a run, for status bars.
pub fn summarize(reports: &[RuleReport], apply: bool) -> String {
    let total = |count: fn(&RuleReport) -> usize| reports.iter().map(count).sum::<usize>();
    let mut text = format!(
        "{}: {} {}",
        format::count(reports.len(), "rule", "rules"),
        format::count(total(|r| r.changed), "entry", "entries"),
        if apply { "changed" } else { "would change" },
    );
    for (n, what) in [(total(|r| r.waiting), "not in the cache yet"), (total(|r| r.refused), "that Roblox would not hand out")] {
        if n > 0 {
            text.push_str(&format!(", {} {what}", format::count(n, "target", "targets")));
        }
    }
    let problems = total(|r| r.notes.len());
    if problems > 0 {
        text.push_str(&format!(", {}", format::count(problems, "problem", "problems")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fleasion_shapes() {
        let text = r#"{"replacement_rules": [
            {"name": "skin", "replace_ids": [123, "456", "Audio"], "with_id": 789},
            {"name": "off", "replace_ids": [1], "enabled": false},
            {"type": "group", "children": [
                {"name": "gone", "replace_ids": ["00112233445566778899aabbccddeeff"], "mode": "remove"},
                {"name": "file", "replace_ids": [5], "mode": "local", "local_path": "a.png"},
                {"name": "url", "replace_ids": [6], "mode": "cdn", "cdn_url": "https://example.com/a"}
            ]},
            {"name": "legacy", "replace_ids": [7], "remove": true},
            {"name": "placeholder", "replace_ids": [8], "with_id": 0},
            {"name": "slot", "replace_ids": ["7547298786:1"], "with_id": 9},
            {"name": "empty with", "replace_ids": [10], "with_id": null}
        ]}"#;
        let rules = parse(text, Path::new("C:/cfg")).unwrap();
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["skin", "gone", "file", "url", "legacy", "slot", "empty with"]);
        assert_eq!(rules[0].targets, [Target::Asset(123), Target::Asset(456), Target::Type("audio".into())]);
        assert_eq!(rules[0].action, Action::With(Target::Asset(789)));
        assert!(matches!(rules[1].targets[0], Target::Entry(id) if id[0] == 0 && id[15] == 0xff));
        assert_eq!(rules[1].action, Action::Remove);
        assert_eq!(rules[2].action, Action::Local(PathBuf::from("C:/cfg").join("a.png")));
        assert_eq!(rules[3].action, Action::Cdn("https://example.com/a".into()));
        assert_eq!(rules[4].action, Action::Remove);
        assert_eq!(rules[5].targets, [Target::Unsupported("7547298786:1".into())]);
        assert_eq!(rules[6].action, Action::Remove);
    }

    #[test]
    fn malformed_rules_never_fall_back_to_removing() {
        let text = r#"[
            {"name": "caps", "replace_ids": [5], "mode": "LOCAL", "local_path": "a.png"},
            {"name": "typo", "replace_ids": [5], "mode": "locl", "local_path": "a.png"},
            {"name": "type source", "replace_ids": [5], "with_id": "Mesh"},
            {"name": "negative", "replace_ids": [5], "with_id": -3}
        ]"#;
        let rules = parse(text, Path::new("C:/cfg")).unwrap();
        assert_eq!(rules[0].action, Action::Local(PathBuf::from("C:/cfg").join("a.png")));
        for rule in &rules[1..] {
            assert!(matches!(rule.action, Action::Invalid(_)), "{} parsed as {:?}", rule.name, rule.action);
        }
    }

    #[test]
    fn combining_keeps_order_and_names_the_source() {
        let rule = |name: &str| Rule { name: name.to_string(), targets: vec![Target::Asset(1)], action: Action::Remove };
        let (a, b) = (vec![rule("one"), rule("two")], vec![rule("three")]);
        let names = |rules: Vec<Rule>| rules.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(combine([("first", a.as_slice()), ("second", b.as_slice())])), ["first / one", "first / two", "second / three"]);
        assert_eq!(names(combine([("only", a.as_slice())])), ["one", "two"]);
        assert_eq!(name_of(Path::new("C:/packs/My Pack.json")), "My Pack");
    }

    /// A cache holding these bodies as wrapped entries, in a fresh folder under
    /// the temp directory. The entries get the IDs 1, 2, ... repeated 16 times.
    fn cache_of(tag: &str, bodies: &[&[u8]]) -> (PathBuf, Vec<Id>) {
        let dir = std::env::temp_dir().join(format!("rbxcache-config-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("rbx-storage")).unwrap();
        let db = rusqlite::Connection::open(dir.join("rbx-storage.db")).unwrap();
        db.execute_batch("CREATE TABLE files (id BLOB PRIMARY KEY NOT NULL, content BLOB, size INTEGER DEFAULT 0 NOT NULL, hits INTEGER DEFAULT 0 NOT NULL, atime INTEGER DEFAULT 0 NOT NULL, category INTEGER DEFAULT 0 NOT NULL, score INTEGER DEFAULT 0 NOT NULL, ttl INTEGER DEFAULT 0 NOT NULL)").unwrap();
        let mut ids = Vec::new();
        for (n, body) in bodies.iter().enumerate() {
            let mut blob = b"RBXH".to_vec();
            for v in [2u32, 0] {
                blob.extend_from_slice(&v.to_le_bytes());
            }
            blob.push(0);
            for v in [200u32, 0, 0, body.len() as u32, format::checksum(body), 0] {
                blob.extend_from_slice(&v.to_le_bytes());
            }
            blob.extend_from_slice(body);
            let id = [n as u8 + 1; 16];
            db.execute(
                "INSERT INTO files (id, content, size, category) VALUES (?1, ?2, ?3, 10)",
                rusqlite::params![&id[..], blob, blob.len() as i64],
            )
            .unwrap();
            ids.push(id);
        }
        (dir, ids)
    }

    fn png(colour: [u8; 4]) -> Vec<u8> {
        let mut png = std::io::Cursor::new(Vec::new());
        RgbaImage::from_pixel(8, 8, image::Rgba(colour)).write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    #[test]
    fn a_payload_is_fitted_to_the_entry_it_goes_into() {
        let picture = Payload::new(Arc::new(png([9, 8, 7, 255])));
        let pack = texture::tests::sample_pack(137, 16, 3, "abc");
        // Into a texture pack: the same pack, repainted.
        let fitted = picture.fit("ktx2", &pack);
        let back = Pack::parse(&fitted).unwrap();
        assert_eq!((back.width, back.levels, back.content_hash.as_str()), (16, 3, "abc"));
        assert!(picture.held_by("ktx2", &fitted) && !picture.held_by("ktx2", &pack));
        // Into anything that is not a texture: untouched.
        assert_eq!(&*picture.fit("png", b""), &picture.bytes[..]);
        assert_eq!(format::sniff(&picture.fit("ktx2", b"")), "ktx2");
        let sound = Payload::new(Arc::new(b"OggS.....".to_vec()));
        assert_eq!(&*sound.fit("ktx2", &pack), b"OggS.....");
        assert_eq!(&*sound.fit("ogg", b"OggS"), b"OggS.....");
        // A pack going into an image entry, and nothing at all, both become pictures.
        assert_eq!(format::sniff(&Payload::new(Arc::new(pack.clone())).fit("png", b"")), "png");
        let nothing = Payload::new(Arc::default());
        assert_eq!(format::sniff(&nothing.fit("png", b"")), "png");
        assert_eq!(Pack::parse(&nothing.fit("ktx2", &pack)).unwrap().format, 137);
        assert!(nothing.fit("ogg", b"OggS").is_empty());
    }

    #[test]
    fn every_pack_of_a_texture_is_replaced() {
        let packs = [texture::tests::sample_pack(137, 8, 4, "feed"), texture::tests::sample_pack(137, 32, 2, "feed")];
        let other = texture::tests::sample_pack(137, 8, 4, "another texture");
        let (dir, ids) = cache_of("packs", &[&packs[0], &packs[1], &other]);
        std::fs::write(dir.join("new.png"), png([0, 250, 0, 255])).unwrap();
        // Only the first pack was ever fetched by ID; the content hash brings in the second.
        let mut idmap = IdMap::default();
        idmap.add(42, fingerprint(&packs[0]));
        let rules = [Rule { name: "paint".into(), targets: vec![Target::Asset(42)], action: Action::Local(dir.join("new.png")) }];
        let store = Store::open(&dir, true).unwrap();
        for round in 0..2 {
            let entries = store.list(|_, _| {}).unwrap();
            let reports = run(&store, &entries, &idmap, &rules, true, None, None);
            assert_eq!((reports[0].matched, reports[0].changed), (2, if round == 0 { 2 } else { 0 }), "round {round}: {:?}", reports[0].notes);
        }
        for (id, side) in [(ids[0], 8), (ids[1], 32)] {
            let body = store.read_body(&id).unwrap();
            let decoded = crate::viewers::decode_ktx2(&body, 64).unwrap();
            assert_eq!(decoded.width, side);
            assert!(decoded.pixels[1] > 240 && decoded.pixels[0] < 12, "pack of side {side} holds {:?}", &decoded.pixels[..4]);
        }
        assert_eq!(store.read_body(&ids[2]).unwrap(), other);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_last_rule_keeps_an_entry_however_often_the_rules_run() {
        let (dir, ids) = cache_of("order", &[b"OggS original"]);
        let id = ids[0];
        std::fs::write(dir.join("first.bin"), b"OggS first, and longer").unwrap();
        std::fs::write(dir.join("second.bin"), b"OggS second").unwrap();
        let rule = |name: &str, file: &str| Rule { name: name.into(), targets: vec![Target::Entry(id)], action: Action::Local(dir.join(file)) };
        let rules = [rule("first", "first.bin"), rule("second", "second.bin")];
        let store = Store::open(&dir, true).unwrap();
        for round in 0..3 {
            let entries = store.list(|_, _| {}).unwrap();
            let reports = run(&store, &entries, &IdMap::default(), &rules, true, None, None);
            assert_eq!(store.read_body(&id).unwrap(), b"OggS second", "round {round}");
            // The entry is counted for the rule that has it, and only while it still needs writing.
            assert_eq!((reports[0].changed, reports[1].changed), (0, usize::from(round == 0)), "round {round}");
        }
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn absent_targets_are_counted_apart_from_problems() {
        let (dir, _) = cache_of("absent", &[b"OggS cached"]);
        let mut idmap = IdMap::default();
        idmap.add(1, fingerprint(b"content Roblox has not downloaded"));
        // An empty list is how a refused lookup is remembered.
        idmap.assets.entry("2".into()).or_default();
        let targets = vec![Target::Asset(1), Target::Asset(2), Target::Asset(3), Target::Unsupported("9:1".into())];
        let rules = [Rule { name: "r".into(), targets, action: Action::Remove }];
        let store = Store::open(&dir, false).unwrap();
        let reports = run(&store, &store.list(|_, _| {}).unwrap(), &idmap, &rules, false, None, None);
        assert_eq!((reports[0].waiting, reports[0].refused, reports[0].notes.len()), (1, 1, 2), "{:?}", reports[0].notes);
        let summary = "1 rule: 0 entries would change, 1 target not in the cache yet, 1 target that Roblox would not hand out, 2 problems";
        assert_eq!(summarize(&reports, false), summary);
        assert_eq!(summarize(&[RuleReport::default()], true), "1 rule: 0 entries changed");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Originals kept in a folder of their own inside a test's cache folder.
    fn originals_in(dir: &Path) -> Originals {
        Originals::load(dir.join("originals")).unwrap()
    }

    #[test]
    fn switching_a_rule_off_puts_the_original_back() {
        let pack = texture::tests::sample_pack(137, 8, 2, "cafe");
        let (dir, ids) = cache_of("revert", &[b"OggS first sound", b"OggS second sound", &pack]);
        std::fs::write(dir.join("new.ogg"), b"OggS replacement, longer than either").unwrap();
        std::fs::write(dir.join("new.png"), png([0, 0, 250, 255])).unwrap();
        let mut idmap = IdMap::default();
        idmap.add(1, fingerprint(b"OggS first sound"));
        idmap.add(2, fingerprint(b"OggS second sound"));
        idmap.add(3, fingerprint(&pack));
        let rule = |name: &str, asset: u64, file: &str| Rule {
            name: name.into(),
            targets: vec![Target::Asset(asset)],
            action: Action::Local(dir.join(file)),
        };
        let all = [rule("one", 1, "new.ogg"), rule("two", 2, "new.ogg"), rule("paint", 3, "new.png")];
        let store = Store::open(&dir, true).unwrap();
        let sync = |rules: &[Rule]| {
            let entries = store.list(|_, _| {}).unwrap();
            let mut originals = originals_in(&dir);
            let (reports, restored) = sync(&store, &entries, &idmap, rules, &mut originals);
            (reports.iter().map(|r| r.changed).sum::<usize>(), reports.iter().map(|r| r.waiting).sum::<usize>(), restored.len())
        };
        assert_eq!(sync(&all), (3, 0, 0));
        // The replaced entries are still known as the assets they were, so nothing is done twice or reported as missing.
        assert_eq!(sync(&all), (0, 0, 0));
        assert_eq!(store.read_body(&ids[0]).unwrap(), b"OggS replacement, longer than either");

        // Without the second and third rule, their entries go back to what Roblox wrote.
        assert_eq!(sync(&all[..1]), (0, 0, 2));
        assert_eq!(store.read_body(&ids[1]).unwrap(), b"OggS second sound");
        assert_eq!(store.read_body(&ids[2]).unwrap(), pack);
        assert_eq!(store.read_body(&ids[0]).unwrap(), b"OggS replacement, longer than either");
        // And back on again.
        assert_eq!(sync(&all), (2, 0, 0));
        assert_eq!(sync(&[]), (0, 0, 3));
        assert_eq!(store.read_body(&ids[0]).unwrap(), b"OggS first sound");
        assert!(originals_in(&dir).replaced(&ids[0], &fingerprint(b"OggS first sound")).is_none());
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_roblox_writes_again_is_not_mistaken_for_a_replacement() {
        let (dir, ids) = cache_of("rewritten", &[b"OggS version one"]);
        let rules = [Rule { name: "mute".into(), targets: vec![Target::Type("audio".into())], action: Action::Remove }];
        let store = Store::open(&dir, true).unwrap();
        let run_once = |rules: &[Rule]| {
            let mut originals = originals_in(&dir);
            sync(&store, &store.list(|_, _| {}).unwrap(), &IdMap::default(), rules, &mut originals)
        };
        // Removing leaves nothing that looks like a sound, yet the rule keeps the entry on every run.
        for round in 0..3 {
            let (reports, restored) = run_once(&rules);
            assert_eq!((reports[0].changed, restored.len()), (usize::from(round == 0), 0), "round {round}");
            assert!(store.read_body(&ids[0]).unwrap().is_empty());
        }
        // Roblox downloads the asset again, in a newer version. That is the original now.
        store.replace_body(&ids[0], b"OggS version two").unwrap();
        assert_eq!(run_once(&rules).0[0].changed, 1);
        let (_, restored) = run_once(&[]);
        assert_eq!(restored.len(), 1);
        assert_eq!(store.read_body(&ids[0]).unwrap(), b"OggS version two");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rule_that_fails_keeps_its_entries_and_swaps_hold_still() {
        let (dir, ids) = cache_of("swap", &[b"OggS left", b"OggS right", b"OggS third"]);
        let entry = |i: usize| Target::Entry(ids[i]);
        let swap = [
            Rule { name: "left".into(), targets: vec![entry(0)], action: Action::With(entry(1)) },
            Rule { name: "right".into(), targets: vec![entry(1)], action: Action::With(entry(0)) },
            Rule { name: "third".into(), targets: vec![entry(2)], action: Action::Local(dir.join("sound.ogg")) },
        ];
        std::fs::write(dir.join("sound.ogg"), b"OggS replacement").unwrap();
        let store = Store::open(&dir, true).unwrap();
        let run_once = || {
            let mut originals = originals_in(&dir);
            let (reports, restored) = sync(&store, &store.list(|_, _| {}).unwrap(), &IdMap::default(), &swap, &mut originals);
            (reports.iter().map(|r| r.changed).sum::<usize>(), restored.len())
        };
        assert_eq!(run_once(), (3, 0));
        // Each side is given what the other held to begin with, however often the rules run.
        for _ in 0..2 {
            assert_eq!(run_once(), (0, 0));
            assert_eq!((store.read_body(&ids[0]).unwrap(), store.read_body(&ids[1]).unwrap()), (b"OggS right".to_vec(), b"OggS left".to_vec()));
        }
        // The file of the third rule goes missing. The rule is reported, and its entry is not put back.
        std::fs::remove_file(dir.join("sound.ogg")).unwrap();
        assert_eq!(run_once(), (0, 0));
        assert_eq!(store.read_body(&ids[2]).unwrap(), b"OggS replacement");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_pack_named_by_its_entry_stands_for_the_texture() {
        let packs = [texture::tests::sample_pack(137, 8, 4, "beef"), texture::tests::sample_pack(137, 32, 2, "beef")];
        let (dir, ids) = cache_of("entry-pack", &[&packs[0], &packs[1]]);
        let rules = [Rule { name: "clear".into(), targets: vec![Target::Entry(ids[1])], action: Action::Remove }];
        let store = Store::open(&dir, true).unwrap();
        let reports = run(&store, &store.list(|_, _| {}).unwrap(), &IdMap::default(), &rules, true, None, None);
        assert_eq!((reports[0].matched, reports[0].changed), (2, 2));
        // A texture that is not one of Roblox's packs gets a transparent picture too, not an empty file.
        let plain = crate::viewers::encode_ktx2(2, 2, &[255; 16]).unwrap();
        let nothing = Payload::new(Arc::default());
        let cleared = nothing.fit("ktx2", &plain);
        assert!(crate::viewers::decode_ktx2(&cleared, 64).unwrap().pixels.iter().all(|&c| c == 0));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_given_twice_is_loaded_once() {
        let dir = std::env::temp_dir().join(format!("rbxcache-config-test-twice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pack.json");
        std::fs::write(&path, r#"[{"name": "r", "replace_ids": [5], "mode": "remove"}]"#).unwrap();
        assert_eq!(load_all(&[path.clone(), path.clone()]).unwrap().len(), 1);
        std::fs::write(&path, "not json").unwrap();
        let error = format!("{:#}", load(&path).unwrap_err());
        assert!(error.contains("pack.json"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn accepts_a_bare_list_and_rejects_garbage() {
        assert_eq!(parse(r#"[{"replace_ids": [5], "mode": "remove"}]"#, Path::new(".")).unwrap().len(), 1);
        assert!(parse("not json", Path::new(".")).is_err());
    }

    #[test]
    fn id_map_round_trips() {
        let mut map = IdMap::default();
        let fp = fingerprint(b"hello");
        assert_eq!(fp.md5, "5d41402abc4b2a76b9719d911017c592");
        assert!(map.add(42, fp.clone()));
        assert!(!map.add(42, fp.clone()));
        let text = serde_json::to_string(&map).unwrap();
        assert_eq!(text, r#"{"42":[{"md5":"5d41402abc4b2a76b9719d911017c592","size":5}]}"#);
        let back: IdMap = serde_json::from_str(&text).unwrap();
        assert_eq!(back.assets_for(&fp), ["42"]);

        // A texture's hash completes a fingerprint that lacked it, and names every pack of the texture.
        let pack = Fingerprint { texture: Some("feed".into()), ..fp.clone() };
        assert!(map.add(42, pack.clone()) && !map.add(42, pack));
        let sibling = Fingerprint { md5: "0".repeat(32), size: 9, texture: Some("feed".into()) };
        assert_eq!(map.assets_for(&sibling), ["42"]);
        let mut other = IdMap::default();
        assert_eq!((other.merge(&map), other.merge(&map)), (1, 0));
        assert_eq!(other.assets_for(&sibling), ["42"]);
    }
}
