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

/// How much of an on-disk blob is read to classify it when listing.
const HEAD_BYTES: usize = 4096;

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
            let kind = if body_len == 0 && (300..400).contains(&h.status) {
                "redirect"
            } else {
                format::sniff(body)
            };
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

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(path)?.take(HEAD_BYTES as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub struct Store {
    pub dir: PathBuf,
    conn: Connection,
}

impl Store {
    /// `%LOCALAPPDATA%\Roblox`
    pub fn default_dir() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Roblox"))
    }

    /// Opens the cache in `dir` (the folder holding `rbx-storage.db` and `rbx-storage`).
    pub fn open(dir: &Path, write: bool) -> Result<Self> {
        let db = dir.join("rbx-storage.db");
        if !db.is_file() {
            bail!("no rbx-storage.db in {}", dir.display());
        }
        let flags = if write { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY };
        let conn = Connection::open_with_flags(&db, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .with_context(|| format!("opening {}", db.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { dir: dir.to_path_buf(), conn })
    }

    pub fn blob_path(&self, id: &Id) -> PathBuf {
        let h = hex(id);
        self.dir.join("rbx-storage").join(&h[..2]).join(h)
    }

    /// Lists every entry. `progress(done, total)` reports the on-disk blobs being classified.
    pub fn list(&self, mut progress: impl FnMut(usize, usize)) -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        let mut on_disk = Vec::new();
        let mut stmt = self.conn.prepare("SELECT id, size, hits, atime, category, ttl, content FROM files")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let Ok(id) = Id::try_from(row.get_ref(0)?.as_blob()?) else { continue };
            let size: i64 = row.get(1)?;
            let category: i64 = row.get(4)?;
            let content = row.get_ref(6)?.as_blob_or_null()?;
            let d = match content {
                Some(c) => describe(c, c.len() as u64, category),
                None => {
                    on_disk.push(entries.len());
                    describe(&[], 0, category)
                }
            };
            entries.push(Entry {
                id,
                id_hex: hex(&id),
                size: size.max(0) as u64,
                hits: row.get(2)?,
                atime: row.get(3)?,
                category,
                ttl: row.get(5)?,
                inline: content.is_some(),
                kind: d.kind,
                wrapped: d.wrapped,
                url: d.url,
                status: d.status,
                asset_type: d.asset_type,
                body_len: d.body_len,
            });
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
                    let (entries, done, dir) = (&entries, &done, &self.dir);
                    s.spawn(move || {
                        let mut out = Vec::with_capacity(part.len());
                        for &i in part {
                            let e = &entries[i];
                            let path = self_blob_path(dir, &e.id_hex);
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
            let e = &mut entries[i];
            e.kind = d.kind;
            e.wrapped = d.wrapped;
            e.url = d.url;
            e.status = d.status;
            e.asset_type = d.asset_type;
            e.body_len = d.body_len;
        }
        progress(total, total);
        Ok(entries)
    }

    /// Loads the metadata of a single entry.
    pub fn entry(&self, id: &Id) -> Result<Entry> {
        let (size, hits, atime, category, ttl, inline): (i64, i64, i64, i64, i64, bool) = self
            .conn
            .query_row(
                "SELECT size, hits, atime, category, ttl, content IS NOT NULL FROM files WHERE id = ?1",
                params![&id[..]],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .map_err(|_| anyhow!("no entry {}", hex(id)))?;
        let raw = self.read(id)?;
        let d = describe(&raw, raw.len() as u64, category);
        Ok(Entry {
            id: *id,
            id_hex: hex(id),
            size: size.max(0) as u64,
            hits,
            atime,
            category,
            ttl,
            inline,
            kind: d.kind,
            wrapped: d.wrapped,
            url: d.url,
            status: d.status,
            asset_type: d.asset_type,
            body_len: d.body_len,
        })
    }

    /// Reads the stored bytes of an entry, wrapper included.
    pub fn read(&self, id: &Id) -> Result<Vec<u8>> {
        let content: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT content FROM files WHERE id = ?1", params![&id[..]], |r| r.get(0))
            .map_err(|_| anyhow!("no entry {}", hex(id)))?;
        match content {
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

    /// Reads just the payload of an entry.
    pub fn read_body(&self, id: &Id) -> Result<Vec<u8>> {
        let (mut raw, header) = self.read_parsed(id)?;
        if let Some(h) = header {
            raw.drain(..h.body_offset);
        }
        Ok(raw)
    }

    /// Removes an entry from the index and deletes its blob file, if any.
    pub fn delete(&self, id: &Id) -> Result<()> {
        let n = self.conn.execute("DELETE FROM files WHERE id = ?1", params![&id[..]])?;
        if n == 0 {
            bail!("no entry {}", hex(id));
        }
        let path = self.blob_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow!("entry removed from the index, but {} could not be deleted: {e}", path.display())),
        }
    }

    /// Swaps the payload of an entry for `body`, keeping the wrapper (URL, status, headers).
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
        if new.len() > INLINE_MAX {
            let parent = path.parent().expect("blob path has a parent");
            std::fs::create_dir_all(parent)?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, &new).with_context(|| format!("writing {}", tmp.display()))?;
            std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
            self.conn.execute(
                "UPDATE files SET content = NULL, size = ?2 WHERE id = ?1",
                params![&id[..], new.len() as i64],
            )?;
        } else {
            self.conn.execute(
                "UPDATE files SET content = ?2, size = ?3 WHERE id = ?1",
                params![&id[..], &new, new.len() as i64],
            )?;
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(anyhow!("index updated, but old file {} could not be deleted: {e}", path.display()));
                }
            }
        }
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

fn self_blob_path(dir: &Path, id_hex: &str) -> PathBuf {
    dir.join("rbx-storage").join(&id_hex[..2]).join(id_hex)
}

/// True when a Roblox client or Studio process is running (it may be using the cache).
pub fn roblox_running() -> bool {
    let mut cmd = std::process::Command::new("tasklist");
    cmd.args(["/FI", "IMAGENAME eq Roblox*", "/FO", "CSV", "/NH"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("roblox"))
}

pub fn format_time_ms(ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) if ms > 0 => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => "-".into(),
    }
}
