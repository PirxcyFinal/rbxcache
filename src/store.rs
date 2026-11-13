//! Access to the cache: the `rbx-storage.db` index and the `rbx-storage` blob folder.

use crate::format::{self, Header};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub type Id = [u8; 16];

/// Roblox keeps entries up to this size inside the database and larger ones as files.
pub const INLINE_MAX: usize = 7000;

/// How much of an on-disk blob is read at first to classify it when listing.
const HEAD_BYTES: usize = 4096;
/// Upper bound on a wrapper (URL plus response headers) worth reading in full.
const MAX_WRAPPER: usize = 1 << 20;

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: Id,
    pub id_hex: String,
    /// Stored size including the `RBXH` wrapper.
    pub size: u64,
    pub hits: i64,
    /// Last access, milliseconds since the Unix epoch.
    pub atime: i64,
    pub category: i64,
    /// Expiry in seconds since the Unix epoch (redirect entries only).
    pub ttl: i64,
    pub inline: bool,
    pub kind: &'static str,
    pub wrapped: bool,
    pub url: String,
    pub status: u32,
    pub asset_type: Option<u32>,
    pub body_len: u64,
}

/// The columns of a row that change as Roblox uses the cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub id: Id,
    pub size: i64,
    pub hits: i64,
    pub atime: i64,
}

/// The columns of a `files` row that do not depend on the content.
struct Row {
    id: Id,
    size: i64,
    hits: i64,
    atime: i64,
    category: i64,
    ttl: i64,
    inline: bool,
}

impl Entry {
    fn new(row: Row, d: Described) -> Self {
        Self {
            id: row.id,
            id_hex: hex(&row.id),
            size: row.size.max(0) as u64,
            hits: row.hits,
            atime: row.atime,
            category: row.category,
            ttl: row.ttl,
            inline: row.inline,
            kind: d.kind,
            wrapped: d.wrapped,
            url: d.url,
            status: d.status,
            asset_type: d.asset_type,
            body_len: d.body_len,
        }
    }

    pub fn stamp(&self) -> Stamp {
        Stamp { id: self.id, size: self.size as i64, hits: self.hits, atime: self.atime }
    }

    fn describe(&mut self, d: Described) {
        self.kind = d.kind;
        self.wrapped = d.wrapped;
        self.url = d.url;
        self.status = d.status;
        self.asset_type = d.asset_type;
        self.body_len = d.body_len;
    }
}

pub fn category_name(category: i64) -> &'static str {
    match category {
        1 => "meta",
        10 => "asset",
        11 => "redirect",
        21 => "zstd-blob",
        22 => "csg-physics",
        29 => "asset-map",
        _ => "other",
    }
}

pub fn hex(id: &[u8]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

fn blob_path(blobs: &Path, id_hex: &str) -> PathBuf {
    blobs.join(&id_hex[..2]).join(id_hex)
}

/// The folder holding the blob files of the cache whose index is in `dir`.
/// It normally sits beside the index. Sober on Linux may keep it under the
/// Flatpak cache folder instead, so that is used when the sibling is absent.
pub fn blob_dir(dir: &Path) -> PathBuf {
    let beside = dir.join("rbx-storage");
    if beside.is_dir() {
        return beside;
    }
    // .../org.vinegarhq.Sober/data/sober/appData -> .../org.vinegarhq.Sober/cache/sober/rbx-storage
    let sober = dir.ancestors().nth(3).map(|root| root.join("cache").join("sober").join("rbx-storage"));
    match sober {
        Some(path) if dir.ends_with("data/sober/appData") && path.is_dir() => path,
        _ => beside,
    }
}

struct Described {
    kind: &'static str,
    wrapped: bool,
    url: String,
    status: u32,
    asset_type: Option<u32>,
    body_len: u64,
}

/// Classifies a blob from its first bytes. `total` is the full stored size.
fn describe(head: &[u8], total: u64, category: i64) -> Described {
    match format::parse_header(head) {
        Some(h) => {
            let body = &head[h.body_offset..];
            let body_len = total.saturating_sub(h.body_offset as u64);
            let kind = if body_len == 0 && (300..400).contains(&h.status) { "redirect" } else { format::sniff(body) };
            Described { kind, wrapped: true, asset_type: format::asset_type_id(&h.headers), url: h.url, status: h.status, body_len }
        }
        None => {
            let kind = match (format::sniff(head), category) {
                ("binary", 22) => "csg-physics",
                ("binary", 21) => "zstd-blob",
                ("json", 29) => "asset-map",
                (k, _) => k,
            };
            Described { kind, wrapped: false, url: String::new(), status: 0, asset_type: None, body_len: total }
        }
    }
}

/// Reads the start of a blob: `HEAD_BYTES`, or more when its wrapper is longer
/// than that, so the wrapper and the first bytes of the body are both covered.
fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(HEAD_BYTES);
    file.by_ref().take(HEAD_BYTES as u64).read_to_end(&mut buf)?;
    // Each pass can reveal one more length field: first the URL's, then the headers'.
    for _ in 0..2 {
        let wanted = match format::wrapper_len(&buf) {
            Some(len) => len.saturating_add(64).min(MAX_WRAPPER),
            None => break,
        };
        if wanted <= buf.len() {
            break;
        }
        let more = (wanted - buf.len()) as u64;
        if file.by_ref().take(more).read_to_end(&mut buf)? < more as usize {
            break;
        }
    }
    Ok(buf)
}

pub struct Store {
    pub dir: PathBuf,
    blobs: PathBuf,
    conn: Connection,
}

impl Store {
    /// Where the Roblox client keeps its cache index: `%LOCALAPPDATA%\Roblox` on
    /// Windows, `~/Library/Roblox` on macOS, and Sober's data folder on Linux.
    pub fn default_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        let dir = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Roblox"));
        #[cfg(target_os = "macos")]
        let dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Roblox"));
        #[cfg(all(unix, not(target_os = "macos")))]
        let dir = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".var/app/org.vinegarhq.Sober/data/sober/appData"));
        dir
    }

    /// Opens the cache in `dir` (the folder holding `rbx-storage.db` and `rbx-storage`).
    pub fn open(dir: &Path, write: bool) -> Result<Self> {
        let db = dir.join("rbx-storage.db");
        if !db.is_file() {
            bail!("no rbx-storage.db in {}", dir.display());
        }
        let flags = if write { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY };
        let conn = Connection::open_with_flags(&db, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).with_context(|| format!("opening {}", db.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { dir: dir.to_path_buf(), blobs: blob_dir(dir), conn })
    }

    pub fn blob_path(&self, id: &Id) -> PathBuf {
        blob_path(&self.blobs, &hex(id))
    }

    /// Lists every entry. `progress(done, total)` reports the on-disk blobs being classified.
    pub fn list(&self, mut progress: impl FnMut(usize, usize)) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut on_disk = Vec::new();
        let mut stmt = self.conn.prepare("SELECT id, size, hits, atime, category, ttl, content FROM files")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            // A row that does not have the expected shape is skipped rather than failing the listing.
            let Some(id) = row.get_ref(0)?.as_blob().ok().and_then(|b| Id::try_from(b).ok()) else { continue };
            let number = |column: usize| row.get::<_, Option<i64>>(column).ok().flatten().unwrap_or(0);
            let category = number(4);
            let content = row.get_ref(6)?.as_blob_or_null().ok().flatten();
            let d = match content {
                Some(c) => describe(c, c.len() as u64, category),
                None => {
                    on_disk.push(entries.len());
                    describe(&[], 0, category)
                }
            };
            let row = Row { id, size: number(1), hits: number(2), atime: number(3), category, ttl: number(5), inline: content.is_some() };
            entries.push(Entry::new(row, d));
        }

        // Classifying the large blobs means opening each file, so spread it over threads.
        let total = on_disk.len();
        let done = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
        let chunk = total.div_ceil(threads).max(1);
        let described: Vec<(usize, Described)> = std::thread::scope(|s| {
            let handles: Vec<_> = on_disk
                .chunks(chunk)
                .map(|part| {
                    let (entries, done, blobs) = (&entries, &done, &self.blobs);
                    s.spawn(move || {
                        let mut out = Vec::with_capacity(part.len());
                        for &i in part {
                            let e = &entries[i];
                            let path = blob_path(blobs, &e.id_hex);
                            let d = match read_head(&path) {
                                Ok(head) => {
                                    let len = std::fs::metadata(&path).map_or(e.size, |m| m.len());
                                    describe(&head, len, e.category)
                                }
                                Err(_) => Described { kind: "missing", ..describe(&[], 0, e.category) },
                            };
                            out.push((i, d));
                            done.fetch_add(1, Ordering::Relaxed);
                        }
                        out
                    })
                })
                .collect();
            while !handles.iter().all(|h| h.is_finished()) {
                progress(done.load(Ordering::Relaxed), total);
                std::thread::sleep(Duration::from_millis(60));
            }
            handles.into_iter().flat_map(|h| h.join().expect("classifier thread panicked")).collect()
        });
        for (i, d) in described {
            entries[i].describe(d);
        }
        progress(total, total);
        Ok(entries)
    }

    /// Turns "no such row" into a clear message and keeps the cause of anything else.
    fn lookup<T>(&self, id: &Id, result: rusqlite::Result<T>) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(anyhow!("no entry {}", hex(id))),
            Err(e) => Err(e).with_context(|| format!("reading entry {} from the index", hex(id))),
        }
    }

    /// Loads one entry, classifying it from the start of its content.
    pub fn entry(&self, id: &Id) -> Result<Entry> {
        let found = self.conn.query_row("SELECT size, hits, atime, category, ttl, content FROM files WHERE id = ?1", params![&id[..]], |r| {
            let content: Option<Vec<u8>> = r.get(5)?;
            let row =
                Row { id: *id, size: r.get(0)?, hits: r.get(1)?, atime: r.get(2)?, category: r.get(3)?, ttl: r.get(4)?, inline: content.is_some() };
            Ok((row, content))
        });
        let (row, content) = self.lookup(id, found)?;
        let d = match content {
            Some(c) => describe(&c, c.len() as u64, row.category),
            None => {
                let path = self.blob_path(id);
                match read_head(&path) {
                    Ok(head) => describe(&head, std::fs::metadata(&path).map_or(row.size.max(0) as u64, |m| m.len()), row.category),
                    Err(_) => Described { kind: "missing", ..describe(&[], 0, row.category) },
                }
            }
        };
        Ok(Entry::new(row, d))
    }

    /// A number that changes whenever another process commits to the index.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// The cheap, frequently changing columns of every row.
    pub fn stamps(&self) -> Result<Vec<Stamp>> {
        let mut stmt = self.conn.prepare("SELECT id, size, hits, atime FROM files")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let Some(id) = row.get_ref(0)?.as_blob().ok().and_then(|b| Id::try_from(b).ok()) else { continue };
            let number = |column: usize| row.get::<_, Option<i64>>(column).ok().flatten().unwrap_or(0);
            out.push(Stamp { id, size: number(1), hits: number(2), atime: number(3) });
        }
        Ok(out)
    }

    /// Reads the stored bytes of an entry, wrapper included.
    pub fn read(&self, id: &Id) -> Result<Vec<u8>> {
        let content = self.conn.query_row("SELECT content FROM files WHERE id = ?1", params![&id[..]], |r| r.get::<_, Option<Vec<u8>>>(0));
        match self.lookup(id, content)? {
            Some(c) => Ok(c),
            None => {
                let path = self.blob_path(id);
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
            }
        }
    }

    /// Reads an entry and splits off the wrapper: `(raw, header)`. The payload is
    /// `raw[header.body_offset..]`, or all of `raw` when there is no wrapper.
    pub fn read_parsed(&self, id: &Id) -> Result<(Vec<u8>, Option<Header>)> {
        let raw = self.read(id)?;
        let header = format::parse_header(&raw);
        Ok((raw, header))
    }

    /// The content of an entry with any wrapper removed.
    pub fn read_body(&self, id: &Id) -> Result<Vec<u8>> {
        let (mut raw, header) = self.read_parsed(id)?;
        if let Some(h) = header {
            raw.drain(..h.body_offset);
        }
        Ok(raw)
    }

    /// Deletes an entry's blob file, if it has one, and then its index row. The
    /// file goes first so a blob Roblox still holds open leaves the entry intact.
    pub fn delete(&self, id: &Id) -> Result<()> {
        let exists = self.conn.query_row("SELECT 1 FROM files WHERE id = ?1", params![&id[..]], |_| Ok(()));
        self.lookup(id, exists)?;
        let path = self.blob_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("deleting {}", path.display())),
        }
        self.conn.execute("DELETE FROM files WHERE id = ?1", params![&id[..]])?;
        Ok(())
    }

    /// Swaps the payload of an entry for `body`, keeping the wrapper (URL, status,
    /// headers). The index is only committed once the blob file is in place.
    pub fn replace_body(&self, id: &Id, body: &[u8]) -> Result<Entry> {
        if body.len() > u32::MAX as usize {
            bail!("replacement is too large");
        }
        let (old, header) = self.read_parsed(id)?;
        let new = match &header {
            Some(h) => format::with_new_body(&old, h, body),
            None => body.to_vec(),
        };
        let path = self.blob_path(id);
        let tx = self.conn.unchecked_transaction()?;
        if new.len() > INLINE_MAX {
            std::fs::create_dir_all(path.parent().expect("blob path has a parent"))?;
            let tmp = path.with_extension("tmp");
            let written = std::fs::write(&tmp, &new)
                .with_context(|| format!("writing {}", tmp.display()))
                .and_then(|()| Ok(tx.execute("UPDATE files SET content = NULL, size = ?2 WHERE id = ?1", params![&id[..], new.len() as i64])?))
                .and_then(|_| std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display())));
            if let Err(e) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        } else {
            tx.execute("UPDATE files SET content = ?2, size = ?3 WHERE id = ?1", params![&id[..], &new, new.len() as i64])?;
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("deleting the old file {}", path.display())),
            }
        }
        tx.commit()?;
        self.entry(id)
    }

    /// Resolves a full id or a unique hex prefix of one.
    pub fn resolve(&self, prefix: &str) -> Result<Id> {
        let p = prefix.trim().to_ascii_lowercase();
        if p.is_empty() || p.len() > 32 || !p.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("'{prefix}' is not a hex id");
        }
        let mut stmt = self.conn.prepare("SELECT id FROM files WHERE lower(hex(id)) LIKE ?1 || '%' LIMIT 2")?;
        let found: Vec<Vec<u8>> = stmt.query_map(params![p], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        match found.as_slice() {
            [one] => Id::try_from(one.as_slice()).map_err(|_| anyhow!("entry has a malformed id")),
            [] => bail!("no entry matches '{prefix}'"),
            _ => bail!("'{prefix}' matches more than one entry; give more digits"),
        }
    }
}

/// True when a Roblox client or Studio process is running (it may be using the cache).
#[cfg(windows)]
pub fn roblox_running() -> bool {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/FI", "IMAGENAME eq Roblox*", "/FO", "CSV", "/NH"]);
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("roblox"))
}

/// True when a Roblox client is running (it may be using the cache): the
/// Roblox app on macOS, Sober on Linux.
#[cfg(not(windows))]
pub fn roblox_running() -> bool {
    const NAMES: &[&str] = if cfg!(target_os = "macos") { &["RobloxPlayer", "RobloxStudio"] } else { &["sober", "Sober"] };
    NAMES.iter().any(|name| std::process::Command::new("pgrep").args(["-x", name]).output().is_ok_and(|o| o.status.success()))
}

pub fn format_time_ms(ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) if ms > 0 => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => "-".into(),
    }
}
