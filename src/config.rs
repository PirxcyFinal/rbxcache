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

use crate::store::{hex, Entry, Id, Store};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
            (Some(m), _) => m,
            (None, Some(true)) => "remove",
            _ => "id",
        };
        let action = match mode {
            "remove" => Action::Remove,
            "local" => match text("local_path") {
                Some(p) => {
                    let p = PathBuf::from(p);
                    Action::Local(if p.is_absolute() { p } else { base.join(p) })
                }
                None => Action::Remove,
            },
            "cdn" => text("cdn_url").map_or(Action::Remove, |u| Action::Cdn(u.to_string())),
            _ => match obj.get("with_id") {
                None | Some(Value::Null) => Action::Remove,
                // 0 and 1 are placeholder IDs in Fleasion configs: the rule does nothing.
                Some(v) if matches!(v.as_u64(), Some(0 | 1)) || matches!(v.as_str().map(str::trim), Some("0" | "1")) => continue,
                Some(v) => match parse_target(v) {
                    Some(t) => Action::With(t),
                    None => Action::Remove,
                },
            },
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

pub fn load(path: &Path) -> Result<Vec<Rule>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text, path.parent().unwrap_or(Path::new(".")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub md5: String,
    pub size: u64,
}

pub fn fingerprint(body: &[u8]) -> Fingerprint {
    Fingerprint { md5: hex(&Md5::digest(body)), size: body.len() as u64 }
}

/// Asset ID -> content fingerprints. One asset can have several (image formats, etc).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct IdMap {
    #[serde(flatten)]
    pub assets: BTreeMap<String, Vec<Fingerprint>>,
}

impl IdMap {
    /// `%APPDATA%\rbxcache\idmap.json`
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
        base.join("rbxcache").join("idmap.json")
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
        std::fs::write(path, serde_json::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }

    /// Records that `asset` has this content. Returns false if it was already known.
    pub fn add(&mut self, asset: u64, fp: Fingerprint) -> bool {
        let list = self.assets.entry(asset.to_string()).or_default();
        if list.contains(&fp) {
            return false;
        }
        list.push(fp);
        true
    }

    pub fn assets_for(&self, fp: &Fingerprint) -> Vec<&str> {
        self.assets.iter().filter(|(_, list)| list.contains(fp)).map(|(id, _)| id.as_str()).collect()
    }
}

/// Content kinds covered by a Fleasion asset-type name.
fn kinds_for_type(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "image" | "decal" | "texture" => &["png", "jpeg", "gif", "webp", "ktx2", "ktx"],
        "audio" | "sound" => &["ogg", "mp3", "wav"],
        "mesh" | "meshpart" => &["mesh"],
        "video" => &["webm", "mp4"],
        "font" | "fontface" => &["ttf", "otf"],
        _ => return None,
    })
}

struct Resolver<'a> {
    store: &'a Store,
    entries: &'a [Entry],
    idmap: &'a IdMap,
    by_id: HashMap<Id, usize>,
    by_size: HashMap<u64, Vec<usize>>,
    md5s: RefCell<HashMap<usize, String>>,
}

impl<'a> Resolver<'a> {
    fn new(store: &'a Store, entries: &'a [Entry], idmap: &'a IdMap) -> Self {
        let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if replaceable(e) {
                by_size.entry(e.body_len).or_default().push(i);
            }
        }
        Self { store, entries, idmap, by_id: entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect(), by_size, md5s: RefCell::default() }
    }

    fn md5(&self, i: usize) -> Result<String> {
        if let Some(m) = self.md5s.borrow().get(&i) {
            return Ok(m.clone());
        }
        let m = fingerprint(&self.store.read_body(&self.entries[i].id)?).md5;
        self.md5s.borrow_mut().insert(i, m.clone());
        Ok(m)
    }

    /// Cache entries a target stands for, or why there are none.
    fn resolve(&self, target: &Target) -> Result<Vec<usize>, String> {
        match target {
            Target::Entry(id) => match self.by_id.get(id) {
                Some(&i) => Ok(vec![i]),
                None => Err(format!("{target} is not in the cache")),
            },
            Target::Type(name) => {
                let kinds = kinds_for_type(name).ok_or_else(|| format!("asset type '{name}' cannot be recognised from cached content"))?;
                Ok((0..self.entries.len()).filter(|&i| replaceable(&self.entries[i]) && kinds.contains(&self.entries[i].kind)).collect())
            }
            Target::Asset(n) => {
                let prints = self.idmap.assets.get(&n.to_string()).ok_or_else(|| format!("{target} is not in the ID map"))?;
                let mut found = Vec::new();
                for fp in prints {
                    for &i in self.by_size.get(&fp.size).map_or(&[][..], Vec::as_slice) {
                        if self.md5(i).map_err(|e| format!("{e:#}"))? == fp.md5 {
                            found.push(i);
                        }
                    }
                }
                if found.is_empty() {
                    return Err(format!("{target} is in the ID map but not in the cache right now (not downloaded yet, or already replaced)"));
                }
                Ok(found)
            }
            Target::Unsupported(s) => Err(format!("'{s}' is Fleasion proxy syntax with no cache equivalent")),
        }
    }
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
    pub notes: Vec<String>,
}

fn fetch(url: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let response = ureq::get(url).timeout(std::time::Duration::from_secs(60)).call().with_context(|| format!("downloading {url}"))?;
    let mut body = Vec::new();
    response.into_reader().take(256 << 20).read_to_end(&mut body)?;
    Ok(body)
}

/// Runs `rules` against the cache. With `apply` false nothing is written (and
/// `cdn` rules are not downloaded); the report shows what would happen.
pub fn run(store: &Store, entries: &[Entry], idmap: &IdMap, rules: &[Rule], apply: bool) -> Vec<RuleReport> {
    let resolver = Resolver::new(store, entries, idmap);
    let mut reports = Vec::new();
    for rule in rules {
        let mut report = RuleReport { name: rule.name.clone(), ..Default::default() };
        let mut matched: Vec<usize> = Vec::new();
        for target in &rule.targets {
            match resolver.resolve(target) {
                Ok(found) => matched.extend(found),
                Err(why) => report.notes.push(why),
            }
        }
        matched.sort_unstable();
        matched.dedup();
        report.matched = matched.len();
        if matched.is_empty() {
            reports.push(report);
            continue;
        }
        let payload: Result<Option<Vec<u8>>> = match &rule.action {
            Action::Remove => Ok(Some(Vec::new())),
            Action::Local(path) => std::fs::read(path).map(Some).with_context(|| format!("reading {}", path.display())),
            Action::Cdn(url) if apply => fetch(url).map(Some),
            Action::Cdn(_) => Ok(None),
            Action::With(target) => match resolver.resolve(target) {
                Ok(found) => store.read_body(&entries[found[0]].id).map(Some),
                Err(why) => Err(anyhow::anyhow!("replacement {why}")),
            },
        };
        match payload {
            Err(e) => report.notes.push(format!("{e:#}")),
            Ok(None) => report.changed = matched.len(),
            Ok(Some(payload)) => {
                for &i in &matched {
                    let e = &entries[i];
                    let outcome = store.read_body(&e.id).and_then(|current| {
                        if current == payload {
                            return Ok(false);
                        }
                        if apply {
                            store.replace_body(&e.id, &payload)?;
                        }
                        Ok(true)
                    });
                    match outcome {
                        Ok(true) => report.changed += 1,
                        Ok(false) => {}
                        Err(err) => report.notes.push(format!("{}: {err:#}", e.id_hex)),
                    }
                }
            }
        }
        reports.push(report);
    }
    reports
}

/// One-line summary of a run, for status bars.
pub fn summarize(reports: &[RuleReport], apply: bool) -> String {
    let changed: usize = reports.iter().map(|r| r.changed).sum();
    let idle = reports.iter().filter(|r| r.matched == 0).count();
    let problems: usize = reports.iter().map(|r| r.notes.len()).sum();
    format!(
        "{} rules: {changed} entries {}, {idle} rules matched nothing, {problems} problems",
        reports.len(),
        if apply { "changed" } else { "would change" }
    )
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
        let back: IdMap = serde_json::from_str(&serde_json::to_string(&map).unwrap()).unwrap();
        assert_eq!(back.assets_for(&fp), ["42"]);
    }
}
