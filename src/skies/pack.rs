//! Patch Roblox's shaders_d3d11.pack sky entries with rebuilt HLSL.

use super::d3d;
use super::effects::{self, EffectSrc};
use anyhow::{anyhow, bail, Result};
use md5::{Digest, Md5};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const NAME_TABLE: usize = 84;
const NAME_SIZE: usize = 68;
const RECORD_SIZE: usize = 64;
const FEATURE_SIZE: usize = 65;

struct SkyRecord {
    pos: usize,
    name: String,
    kind: u8,
    offset: usize,
    /// One of the classic sky's shaders (`Sky...`), drawn when a game has no Atmosphere,
    /// rather than the advanced sky's (`AdvSky...`).
    classic: bool,
}

/// Whether `name` is a sky shader, and if so whether it is the classic sky's.
pub(super) fn sky_family(name: &str) -> Option<bool> {
    if name.starts_with("AdvSky") {
        Some(false)
    } else if name == "SkyVS" || (name.starts_with("Sky") && name.ends_with("FS")) {
        Some(true)
    } else {
        None
    }
}

fn layout(data: &[u8]) -> Result<(Vec<String>, usize, usize)> {
    if data.len() < 20 {
        bail!("shader pack is too small");
    }
    let magic = &data[0..4];
    let variants = u16::from_le_bytes(data[6..8].try_into().unwrap());
    let flags = u16::from_le_bytes(data[8..10].try_into().unwrap()) as usize;
    let features = u16::from_le_bytes(data[10..12].try_into().unwrap()) as usize;
    let names = u16::from_le_bytes(data[12..14].try_into().unwrap()) as usize;
    let records = u16::from_le_bytes(data[14..16].try_into().unwrap()) as usize;
    if magic != b"RBXS" || variants != 1 {
        bail!("not a shader pack this build knows (magic {magic:?}, variants {variants})");
    }
    let mut name_list = Vec::with_capacity(names);
    for i in 0..names {
        let start = NAME_TABLE + i * NAME_SIZE;
        let raw = &data[start..start + 64];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(64);
        name_list.push(String::from_utf8_lossy(&raw[..end]).into_owned());
    }
    let table = NAME_TABLE + names * NAME_SIZE + flags * 64 + features * FEATURE_SIZE;
    if table + records * RECORD_SIZE > data.len() {
        bail!("the shader pack is shorter than its tables say");
    }
    let first_blob = u32::from_le_bytes(data[table + 16..table + 20].try_into().unwrap()) as usize;
    if first_blob != table + records * RECORD_SIZE {
        bail!("the record table is not where this build expects it");
    }
    Ok((name_list, table, records))
}

fn sky_records(data: &[u8]) -> Result<Vec<SkyRecord>> {
    Ok(records_of(data)?.into_iter().filter_map(|(record, sky)| sky.then_some(record)).collect())
}

/// Every record in the pack, and whether it is one of the sky shaders.
fn records_of(data: &[u8]) -> Result<Vec<(SkyRecord, bool)>> {
    let (names, table, records) = layout(data)?;
    let mut out = Vec::with_capacity(records);
    for i in 0..records {
        let pos = table + i * RECORD_SIZE;
        let offset = u32::from_le_bytes(data[pos + 16..pos + 20].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(data[pos + 20..pos + 24].try_into().unwrap()) as usize;
        if offset.checked_add(size.max(12)).is_none_or(|end| end > data.len()) {
            bail!("record {i} points past the end of the pack");
        }
        let name_index = u16::from_le_bytes(data[pos + 30..pos + 32].try_into().unwrap()) as usize;
        let name = names.get(name_index).cloned().unwrap_or_default();
        let classic = sky_family(&name);
        out.push((SkyRecord { pos, name, kind: data[pos + 28], offset, classic: classic.unwrap_or(false) }, classic.is_some()));
    }
    Ok(out)
}

/// Whether the pack is one rbxcache rebuilt rather than Roblox's own. In Roblox's pack every
/// shader's code lies inside the file in its first stretch, before the last of the others ends;
/// a rebuild adds the new sky shaders after all of it.
pub fn is_rebuilt(data: &[u8]) -> Result<bool> {
    let records = records_of(data)?;
    let others_end = records.iter().filter(|(_, sky)| !sky).map(|(r, _)| r.offset).max().unwrap_or(0);
    Ok(records.iter().any(|(r, sky)| *sky && r.offset > others_end))
}

fn compile_shader(name: &str, kind: u8, classic: bool, effect: &EffectSrc) -> Result<Option<Vec<u8>>> {
    let started = std::time::Instant::now();
    let (source, target) = match (kind == b'v', classic) {
        (true, true) => (effects::classic_vertex_source(effect), "vs_5_0"),
        (true, false) => {
            let Some(source) = effects::vertex_source(effect) else { return Ok(None) };
            (source, "vs_5_0")
        }
        (false, true) => (effects::classic_pixel_source(name, effect), "ps_5_0"),
        (false, false) => (effects::pixel_source(name, effect), "ps_5_0"),
    };
    let cached = cache_path(&source, target);
    if let Some(code) = cached.as_ref().and_then(|path| std::fs::read(path).ok()).filter(|code| code.starts_with(b"DXBC")) {
        crate::log::write(&format!("reused {name} ({target}) from the shader cache: {} KB", code.len() / 1024));
        return Ok(Some(code));
    }
    let compiled = compile_source(&source, target);
    match &compiled {
        Ok(code) => crate::log::write(&format!(
            "compiled {name} ({target}): {} KB of HLSL -> {} KB in {} ms",
            source.len() / 1024,
            code.len() / 1024,
            started.elapsed().as_millis()
        )),
        Err(e) => crate::log::write(&format!("compile of {name} ({target}) FAILED after {} ms: {e:#}", started.elapsed().as_millis())),
    }
    let code = compiled?;
    if let Some(path) = cached {
        // Written beside it and renamed into place, so a half-written file is never read.
        let partial = path.with_extension("part");
        if std::fs::write(&partial, &code).is_ok() && std::fs::rename(&partial, &path).is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
    Ok(Some(code))
}

/// Compiled shaders are kept in rbxcache's data folder, named by a hash of their HLSL and
/// target, so building a sky again (after a Roblox update, or going back to one) skips the
/// compiler, which takes seconds a shader for the bigger skies. Any change to a sky's HLSL
/// changes the name, so a stale shader is never reused.
fn cache_dir() -> Option<PathBuf> {
    let dir = crate::protection::data_dir().ok()?.join("shader-cache");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn cache_path(source: &str, target: &str) -> Option<PathBuf> {
    let mut hasher = Md5::new();
    hasher.update(target.as_bytes());
    hasher.update([0u8]);
    hasher.update(source.as_bytes());
    let name: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Some(cache_dir()?.join(format!("{name}.dxbc")))
}

/// Keeps the shader cache under about 96 MB, dropping the shaders used longest ago first.
fn trim_cache() {
    let Some(dir) = cache_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= 96 << 20 {
        return;
    }
    files.sort();
    for (_, size, path) in files {
        if total <= 64 << 20 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

/// The first argument a shader-compiling worker is started with: `rbxcache WORKER_FLAG <target>
/// <in.hlsl> <out.dxbc>`.
pub const WORKER_FLAG: &str = "--sky-compile-worker";

/// Runs as a shader-compiling worker when started as one: compiles the HLSL file to the DXBC
/// file and exits. `None` when this process is not a worker.
pub fn worker_main() -> Option<i32> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if (args.len() != 5) || (args[1] != WORKER_FLAG) {
        return None;
    }
    let target = args[2].to_string_lossy().into_owned();
    let result = std::fs::read_to_string(&args[3])
        .map_err(anyhow::Error::from)
        .and_then(|source| d3d::compile(&source, "main", &target))
        .and_then(|code| std::fs::write(&args[4], code).map_err(anyhow::Error::from));
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e:#}");
            1
        }
    })
}

/// Compiles HLSL, in a worker process where one can be started and in this process otherwise.
fn compile_source(source: &str, target: &str) -> Result<Vec<u8>> {
    #[cfg(not(test))]
    if let Some(result) = compile_in_worker(source, target) {
        return result;
    }
    d3d::compile(source, "main", target)
}

/// Compiles HLSL in a separate rbxcache process. d3dcompiler works partly one compile at a time
/// within a process, so shaders compiled side by side each in its own process finish far sooner
/// (measured on 18 Nov 2026: six at once about 3.7 times as fast as one after another, against
/// about 2 times for six threads). `None` when no worker could be run, so the caller compiles
/// in this process instead.
#[cfg(not(test))]
fn compile_in_worker(source: &str, target: &str) -> Option<Result<Vec<u8>>> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let exe = std::env::current_exe().ok()?;
    let dir = std::env::temp_dir().join("rbxcache-shaders");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let input = dir.join(format!("{stamp}.hlsl"));
    let output = dir.join(format!("{stamp}.dxbc"));
    std::fs::write(&input, source).ok()?;
    let mut command = std::process::Command::new(exe);
    command.arg(WORKER_FLAG).arg(target).arg(&input).arg(&output);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up for each worker.
        command.creation_flags(0x0800_0000);
    }
    let run = command.output();
    let code = std::fs::read(&output).ok();
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
    match run {
        Ok(done) if done.status.success() => code.filter(|code| code.starts_with(b"DXBC")).map(Ok),
        Ok(done) if done.status.code() == Some(1) => Some(Err(anyhow!("D3DCompile failed: {}", String::from_utf8_lossy(&done.stderr).trim()))),
        _ => None,
    }
}

/// A compiled shader, or `None` for one the effect leaves as Roblox has it.
type Compiled = Result<Option<Vec<u8>>>;

/// Where a compiled shader was put in the pack: its offset, its size and its digest.
type Placed = (usize, usize, [u8; 16]);

/// Compiles every shader in `jobs`, several at a time, and hands the results back in order.
fn compile_all(jobs: &[&SkyRecord], effect: &EffectSrc) -> Vec<Compiled> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<Compiled>>> = jobs.iter().map(|_| Mutex::new(None)).collect();
    // Each compile runs in its own worker process (see `compile_in_worker`) and peaks at about
    // 125 MB for the biggest sky, so up to a dozen at once, leaving two threads to the rest.
    let workers = std::thread::available_parallelism().map(|n| n.get().saturating_sub(2)).unwrap_or(4).clamp(1, 12).min(jobs.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(record) = jobs.get(i) else { break };
                let result = compile_shader(&record.name, record.kind, record.classic, effect);
                *results[i].lock().unwrap() = Some(result);
            });
        }
    });
    results.into_iter().map(|slot| slot.into_inner().unwrap().unwrap_or_else(|| Err(anyhow!("a sky shader was not compiled")))).collect()
}

/// Returns a new pack with the sky shaders rebuilt for `effect`.
pub fn rebuild(original: &[u8], effect: &EffectSrc) -> Result<Vec<u8>> {
    let mut data = original.to_vec();
    let records = sky_records(&data)?;
    // Each shader is built from its own name, which picks the variant; records that share a
    // name and a blob share the result. The distinct ones are compiled side by side, then
    // added to the pack in the order they first appear, so the pack comes out the same.
    let mut seen = HashSet::new();
    let jobs: Vec<&SkyRecord> = records.iter().filter(|record| seen.insert((record.offset, record.name.clone()))).collect();
    let compiled = compile_all(&jobs, effect);
    trim_cache();
    let mut built: HashMap<(usize, String), Option<Placed>> = HashMap::new();
    for (record, code) in jobs.iter().zip(compiled) {
        let entry = match code? {
            None => None,
            Some(code) => {
                let mut prefix = data[record.offset..record.offset + 12].to_vec();
                prefix[0] |= 1; // may read globals
                let mut blob = prefix;
                blob.extend_from_slice(&code);
                let digest: [u8; 16] = Md5::digest(&blob).into();
                let offset = data.len();
                let size = blob.len();
                data.extend_from_slice(&blob);
                Some((offset, size, digest))
            }
        };
        built.insert((record.offset, record.name.clone()), entry);
    }

    let mut changed = 0usize;
    for record in &records {
        let Some(Some((offset, size, digest))) = built.get(&(record.offset, record.name.clone())).cloned() else { continue };
        data[record.pos..record.pos + 16].copy_from_slice(&digest);
        data[record.pos + 16..record.pos + 20].copy_from_slice(&(offset as u32).to_le_bytes());
        data[record.pos + 20..record.pos + 24].copy_from_slice(&(size as u32).to_le_bytes());
        changed += 1;
    }
    if changed == 0 {
        bail!("found no sky shader to change");
    }
    Ok(data)
}
