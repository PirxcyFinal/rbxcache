//! Command-line interface.

use crate::config::{self, IdMap};
use crate::format::{self, human_size};
use crate::store::{self, category_name, format_time_ms, Entry, Store};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "rbxcache", version, about = "Explore, export, replace and delete entries of the Roblox asset cache.\nRun without a command to open the window.")]
pub struct Args {
    /// Folder holding rbx-storage.db and rbx-storage [default: %LOCALAPPDATA%\Roblox]
    #[arg(long, global = true)]
    pub dir: Option<PathBuf>,
    /// Asset ID map file [default: %APPDATA%\rbxcache\idmap.json]
    #[arg(long, global = true)]
    pub idmap: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Open the graphical explorer (the default)
    Gui,
    /// Summarise the cache by category and content type
    Stats,
    /// List entries
    List {
        /// Only this content type (png, ktx2, mesh, ogg, rbxm, redirect, ...)
        #[arg(long)]
        kind: Option<String>,
        /// Only this category number (10 = assets, 11 = redirects)
        #[arg(long)]
        category: Option<i64>,
        /// Only entries whose id, type or URL contains this text
        #[arg(long)]
        search: Option<String>,
        /// Only entries at least this many bytes
        #[arg(long, default_value_t = 0)]
        min_size: u64,
        /// Include the cached redirect records
        #[arg(long)]
        redirects: bool,
        #[arg(long, value_enum, default_value_t = Sort::Size)]
        sort: Sort,
        /// Maximum rows to print (0 = all)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show everything known about one entry
    Info { id: String },
    /// Write the payload of an entry to a file
    Export {
        id: String,
        /// Output file [default: <id>.<ext> in the current folder]
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Keep the RBXH wrapper instead of extracting the payload
        #[arg(long)]
        raw: bool,
    },
    /// Export the payload of every entry of one content type into a folder
    ExportAll {
        out_dir: PathBuf,
        #[arg(long)]
        kind: String,
    },
    /// Replace the payload of an entry with the contents of a file
    Replace { id: String, file: PathBuf },
    /// Record that a cache entry is the content of a Roblox asset ID
    Tag { id: String, asset_id: u64 },
    /// Apply a Fleasion-style replacement config to the cache
    ApplyConfig {
        file: PathBuf,
        /// Only report what would change
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete entries (Roblox downloads them again when needed)
    Delete {
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Sort {
    Size,
    Hits,
    Recent,
    Id,
}

pub fn run(dir: &Path, idmap_path: &Path, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Gui => unreachable!("handled by main"),
        Cmd::Stats => stats(&Store::open(dir, false)?),
        Cmd::List { kind, category, search, min_size, redirects, sort, limit } => {
            let store = Store::open(dir, false)?;
            let mut entries = store.list(|_, _| {})?;
            let search = search.map(|s| s.to_lowercase());
            let want_redirects = redirects || kind.as_deref() == Some("redirect") || category == Some(11);
            entries.retain(|e| {
                (want_redirects || e.kind != "redirect")
                    && kind.as_deref().is_none_or(|k| e.kind == k)
                    && category.is_none_or(|c| e.category == c)
                    && e.size >= min_size
                    && search.as_deref().is_none_or(|s| matches(e, s))
            });
            match sort {
                Sort::Size => entries.sort_by(|a, b| b.size.cmp(&a.size)),
                Sort::Hits => entries.sort_by(|a, b| b.hits.cmp(&a.hits)),
                Sort::Recent => entries.sort_by(|a, b| b.atime.cmp(&a.atime)),
                Sort::Id => entries.sort_by(|a, b| a.id.cmp(&b.id)),
            }
            let shown = if limit == 0 { entries.len() } else { limit.min(entries.len()) };
            println!("{:<32}  {:<12} {:>10} {:>7}  {}", "ID", "TYPE", "SIZE", "HITS", "LAST USED");
            for e in &entries[..shown] {
                println!("{:<32}  {:<12} {:>10} {:>7}  {}", e.id_hex, e.kind, human_size(e.size), e.hits, format_time_ms(e.atime));
            }
            println!("{shown} of {} matching entries shown", entries.len());
            Ok(())
        }
        Cmd::Info { id } => {
            let store = Store::open(dir, false)?;
            let id = store.resolve(&id)?;
            let e = store.entry(&id)?;
            let (raw, header) = store.read_parsed(&id)?;
            for (k, v) in describe(&store, &e, &raw, header.as_ref()) {
                println!("{k:<16} {v}");
            }
            Ok(())
        }
        Cmd::Export { id, output, raw } => {
            let store = Store::open(dir, false)?;
            let id = store.resolve(&id)?;
            let e = store.entry(&id)?;
            let data = if raw { store.read(&id)? } else { store.read_body(&id)? };
            let out = output.unwrap_or_else(|| export_name(&e, raw).into());
            std::fs::write(&out, &data).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {} ({})", out.display(), human_size(data.len() as u64));
            Ok(())
        }
        Cmd::ExportAll { out_dir, kind } => {
            let store = Store::open(dir, false)?;
            let entries = store.list(|_, _| {})?;
            std::fs::create_dir_all(&out_dir)?;
            let (mut n, mut bytes) = (0usize, 0u64);
            for e in entries.iter().filter(|e| e.kind == kind) {
                let data = store.read_body(&e.id)?;
                std::fs::write(out_dir.join(export_name(e, false)), &data)?;
                n += 1;
                bytes += data.len() as u64;
            }
            println!("exported {n} {kind} entries ({}) to {}", human_size(bytes), out_dir.display());
            Ok(())
        }
        Cmd::Replace { id, file } => {
            let store = Store::open(dir, true)?;
            warn_if_running();
            let id = store.resolve(&id)?;
            let body = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let e = store.replace_body(&id, &body)?;
            println!("replaced {} - now {} ({})", e.id_hex, e.kind, human_size(e.size));
            Ok(())
        }
        Cmd::Tag { id, asset_id } => {
            let store = Store::open(dir, false)?;
            let id = store.resolve(&id)?;
            let mut idmap = IdMap::load(idmap_path)?;
            let fp = config::fingerprint(&store.read_body(&id)?);
            if idmap.add(asset_id, fp) {
                idmap.save(idmap_path)?;
                println!("asset {asset_id} -> {} recorded in {}", store::hex(&id), idmap_path.display());
            } else {
                println!("asset {asset_id} already has this content recorded");
            }
            Ok(())
        }
        Cmd::ApplyConfig { file, dry_run } => {
            let rules = config::load(&file)?;
            let store = Store::open(dir, !dry_run)?;
            if !dry_run {
                warn_if_running();
            }
            let entries = store.list(|_, _| {})?;
            let idmap = IdMap::load(idmap_path)?;
            let reports = config::run(&store, &entries, &idmap, &rules, !dry_run);
            for r in &reports {
                println!("{:<28} matched {:>5}  {} {:>5}", r.name, r.matched, if dry_run { "would change" } else { "changed" }, r.changed);
                for note in &r.notes {
                    println!("    ! {note}");
                }
            }
            println!("{}", config::summarize(&reports, !dry_run));
            Ok(())
        }
        Cmd::Delete { ids } => {
            let store = Store::open(dir, true)?;
            warn_if_running();
            let mut failed = 0;
            for id in &ids {
                match store.resolve(id).and_then(|id| store.delete(&id).map(|()| id)) {
                    Ok(id) => println!("deleted {}", store::hex(&id)),
                    Err(e) => {
                        eprintln!("{id}: {e}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                bail!("{failed} of {} entries could not be deleted", ids.len());
            }
            Ok(())
        }
    }
}

fn warn_if_running() {
    if store::roblox_running() {
        eprintln!("warning: Roblox is running and may overwrite or ignore changes to its cache");
    }
}

pub fn matches(e: &Entry, needle_lower: &str) -> bool {
    e.id_hex.contains(needle_lower) || e.kind.contains(needle_lower) || e.url.to_lowercase().contains(needle_lower)
}

pub fn export_name(e: &Entry, raw: bool) -> String {
    format!("{}.{}", e.id_hex, if raw { "rbxh" } else { format::extension(e.kind) })
}

/// Key/value description of an entry, shared by `info` and the window's detail panel.
pub fn describe(store: &Store, e: &Entry, raw: &[u8], header: Option<&format::Header>) -> Vec<(&'static str, String)> {
    let body = &raw[header.map_or(0, |h| h.body_offset)..];
    let mut out = vec![
        ("ID", e.id_hex.clone()),
        ("Type", e.kind.to_string()),
        ("Category", format!("{} ({})", e.category, category_name(e.category))),
        ("Stored size", format!("{} ({} bytes)", human_size(raw.len() as u64), raw.len())),
        ("Payload size", format!("{} ({} bytes)", human_size(body.len() as u64), body.len())),
        ("Hits", e.hits.to_string()),
        ("Last used", format_time_ms(e.atime)),
    ];
    if e.ttl > 0 {
        out.push(("Expires", format_time_ms(e.ttl * 1000)));
    }
    out.push(("Stored in", if e.inline { "database (inline)".into() } else { store.blob_path(&e.id).display().to_string() }));
    if let Some(d) = payload_details(e.kind, body) {
        out.push(("Details", d));
    }
    match header {
        Some(h) => {
            out.push(("HTTP status", h.status.to_string()));
            if let Some(t) = e.asset_type {
                out.push(("Asset type", format!("{t} ({})", format::asset_type_name(t))));
            }
            if !h.url.is_empty() {
                out.push(("URL", h.url.clone()));
            }
            if !h.headers.trim().is_empty() {
                out.push(("Headers", h.headers.trim().replace("\r\n", "; ")));
            }
            out.push(("Checksum", format!("{:08x} (stored length {})", h.body_sum, h.body_len)));
        }
        None => out.push(("Wrapper", "none (raw blob)".into())),
    }
    out
}

/// Format-specific facts read from the payload itself.
fn payload_details(kind: &str, body: &[u8]) -> Option<String> {
    let u32at = |o: usize| body.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    match kind {
        "png" | "jpeg" | "gif" | "webp" => {
            let reader = image::ImageReader::new(std::io::Cursor::new(body)).with_guessed_format().ok()?;
            let (w, h) = reader.into_dimensions().ok()?;
            Some(format!("{w} x {h} px"))
        }
        "ktx2" => {
            let compression = match u32at(44)? {
                0 => "none",
                1 => "BasisLZ",
                2 => "Zstandard",
                3 => "ZLIB",
                _ => "unknown",
            };
            Some(format!("{} x {} px, {} mip level(s), vkFormat {}, supercompression {compression}", u32at(20)?, u32at(24)?, u32at(40)?.max(1), u32at(12)?))
        }
        "mesh" => {
            let line = body.split(|&c| c == b'\n').next()?;
            Some(String::from_utf8_lossy(line).trim().to_string())
        }
        _ => None,
    }
}

fn stats(store: &Store) -> Result<()> {
    let mut last = 0;
    let entries = store.list(|done, total| {
        if total > 0 && done * 10 / total != last {
            last = done * 10 / total;
            eprint!("\rscanning {}%", last * 10);
        }
    })?;
    eprintln!();
    let mut by_cat: BTreeMap<i64, (usize, u64)> = BTreeMap::new();
    let mut by_kind: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for e in &entries {
        for slot in [by_cat.entry(e.category).or_default(), by_kind.entry(e.kind).or_default()] {
            slot.0 += 1;
            slot.1 += e.size;
        }
    }
    let total: u64 = entries.iter().map(|e| e.size).sum();
    println!("{} entries, {} in {}", entries.len(), human_size(total), store.dir.display());
    println!("\n{:<22} {:>8} {:>12}", "CATEGORY", "ENTRIES", "SIZE");
    for (c, (n, s)) in &by_cat {
        println!("{:<22} {n:>8} {:>12}", format!("{c} ({})", category_name(*c)), human_size(*s));
    }
    let mut kinds: Vec<_> = by_kind.into_iter().collect();
    kinds.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    println!("\n{:<22} {:>8} {:>12}", "TYPE", "ENTRIES", "SIZE");
    for (k, (n, s)) in kinds {
        println!("{k:<22} {n:>8} {:>12}", human_size(s));
    }
    Ok(())
}

