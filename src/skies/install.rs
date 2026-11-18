//! Find every Roblox client and install or remove a rebuilt sky pack.

use crate::protection::data_dir;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const PACK_NAME: &str = "shaders_d3d11.pack";
const ORIGINAL_NAME: &str = "shaders_d3d11.pack.original";
const BACKUP_SUFFIX: &str = ".rbxcache.bak";

#[derive(Clone, Debug)]
pub struct Client {
    pub name: String,
    pub install: PathBuf,
    pub mods: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct State {
    effect: String,
}

pub fn store_dir() -> Result<PathBuf> {
    let path = data_dir()?.join("skies");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn find_clients() -> Result<Vec<Client>> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        bail!("LOCALAPPDATA is not set");
    };
    let launchers = [
        ("Voidstrap", PathBuf::from("Voidstrap/RblxVersions"), Some(PathBuf::from("Voidstrap/VoidstrapMods"))),
        ("Bloxstrap", PathBuf::from("Bloxstrap/Versions"), Some(PathBuf::from("Bloxstrap/Modifications"))),
        ("Fishstrap", PathBuf::from("Fishstrap/Versions"), Some(PathBuf::from("Fishstrap/Modifications"))),
        ("Roblox", PathBuf::from("Roblox/Versions"), None),
    ];
    let mut found = Vec::new();
    for (name, versions, mods) in launchers {
        let root = local.join(versions);
        if !root.is_dir() {
            continue;
        }
        let mut installs: Vec<PathBuf> = std::fs::read_dir(&root)
            .with_context(|| format!("reading {}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("shaders").join(PACK_NAME).is_file())
            .collect();
        installs.sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
        let mods = mods.map(|m| local.join(m));
        for install in installs {
            found.push(Client { name: name.into(), install, mods: mods.clone() });
        }
    }
    Ok(found)
}

pub fn list_clients() -> Result<String> {
    let clients = find_clients()?;
    if clients.is_empty() {
        return Ok("No Roblox installs with shaders were found.".into());
    }
    Ok(clients
        .iter()
        .map(|c| {
            let mods = c.mods.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(stock, no mods folder)".into());
            format!("{}: {}  mods={mods}", c.name, c.install.display())
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Whether the pack at `path` is Roblox's own, not one rbxcache rebuilt.
fn untouched(path: &Path) -> bool {
    std::fs::read(path).ok().is_some_and(|data| matches!(super::pack::is_rebuilt(&data), Ok(false)))
}

/// Whether the pack at `path` is one rbxcache rebuilt. A pack that cannot be read as a shader
/// pack at all is not counted as one.
fn rebuilt_here(path: &Path) -> bool {
    std::fs::read(path).ok().is_some_and(|data| matches!(super::pack::is_rebuilt(&data), Ok(true)))
}

/// The pack skies are built from: Roblox's own pack from the newest install that still has it
/// (the install's own pack, or the copy taken before rbxcache first wrote over it), kept in the
/// app's data folder. After a Roblox update the saved copy is replaced by the new version's, so
/// a sky is never built from an older version's shaders. A launcher can carry a rebuilt pack
/// into a new version from its mods folder, so packs are checked rather than trusted. When no
/// install has its own pack any more, the saved copy is used as it is.
fn ensure_original(clients: &[Client], store: &Path) -> Result<PathBuf> {
    let saved = store.join(ORIGINAL_NAME);
    let mut installs: Vec<&Client> = clients.iter().collect();
    installs.sort_by_key(|c| std::cmp::Reverse(c.install.metadata().and_then(|m| m.modified()).ok()));
    let source = installs.iter().find_map(|c| {
        let pack = c.install.join("shaders").join(PACK_NAME);
        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", pack.display()));
        [pack, backup].into_iter().find(|p| p.is_file() && untouched(p))
    });
    match source {
        Some(source) => {
            let current = std::fs::read(&source).with_context(|| format!("reading {}", source.display()))?;
            if std::fs::read(&saved).ok().as_deref() != Some(&current[..]) {
                crate::log::write(&format!("saving {} as the original shader pack", source.display()));
                std::fs::write(&saved, &current).with_context(|| format!("saving original from {}", source.display()))?;
            }
        }
        None if saved.is_file() => crate::log::write("no install has Roblox's own shader pack; using the saved original"),
        None => {
            let first = clients.first().map(|c| c.install.join("shaders").join(PACK_NAME)).context("no shader pack to copy as original")?;
            crate::log::write(&format!("no untouched shader pack found; saving {} as the original", first.display()));
            std::fs::copy(&first, &saved).with_context(|| format!("saving original from {}", first.display()))?;
        }
    }
    Ok(saved)
}

pub fn install_pack(edited: &[u8], effect_name: &str) -> Result<String> {
    let clients = find_clients()?;
    if clients.is_empty() {
        bail!("Could not find any Roblox install with a shaders folder.");
    }
    let store = store_dir()?;
    let mut steps = Vec::new();
    let mut seen_mods = HashSet::new();

    for client in &clients {
        if let Some(mods) = &client.mods {
            let target_dir = mods.join("shaders");
            if seen_mods.insert(target_dir.clone()) {
                std::fs::create_dir_all(&target_dir)?;
                let target = target_dir.join(PACK_NAME);
                std::fs::write(&target, edited)?;
                steps.push(format!("{} mods -> {}", client.name, target_dir.display()));
            }
        }
        let target = client.install.join("shaders").join(PACK_NAME);
        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", target.display()));
        if !backup.is_file() {
            std::fs::copy(&target, &backup).ok();
        }
        std::fs::write(&target, edited)?;
        steps.push(format!("{} {} -> {}", client.name, client.install.file_name().and_then(|s| s.to_str()).unwrap_or("?"), target.display()));
    }

    std::fs::write(store.join("state.json"), serde_json::to_string_pretty(&State { effect: effect_name.into() })?)?;
    steps.push(String::new());
    steps.push(format!("Applied '{effect_name}' to every Roblox client found."));
    steps.push("Close Roblox fully, start it again and join a game with a visible sky.".into());
    steps.push("Experimental: works best on NVIDIA GPUs.".into());
    Ok(steps.join("\n"))
}

pub fn remove_pack() -> Result<String> {
    let clients = find_clients()?;
    let store = store_dir()?;
    let original = store.join(ORIGINAL_NAME);
    let original_bytes = std::fs::read(&original).ok();
    let mut steps = Vec::new();
    let mut seen_mods = HashSet::new();

    for client in &clients {
        if let Some(mods) = &client.mods {
            let folder = mods.join("shaders");
            if seen_mods.insert(folder.clone()) {
                // Only a pack rbxcache rebuilt is taken out; a shader mod of the user's own stays.
                let pack = folder.join(PACK_NAME);
                if pack.is_file() && rebuilt_here(&pack) {
                    std::fs::remove_file(&pack)?;
                    steps.push(format!("removed mods pack {}", pack.display()));
                }
                if folder.is_dir() && std::fs::read_dir(&folder)?.next().is_none() {
                    let _ = std::fs::remove_dir(&folder);
                }
            }
        }
        let pack = client.install.join("shaders").join(PACK_NAME);
        let backup = PathBuf::from(format!("{}{BACKUP_SUFFIX}", pack.display()));
        if backup.is_file() {
            std::fs::copy(&backup, &pack)?;
            std::fs::remove_file(&backup)?;
            steps.push(format!("restored {} {}", client.name, client.install.file_name().and_then(|s| s.to_str()).unwrap_or("?")));
        } else if let Some(original) = &original_bytes {
            if pack.is_file() {
                let size = pack.metadata()?.len();
                if size > original.len() as u64 + 10_000 {
                    std::fs::write(&pack, original)?;
                    steps.push(format!(
                        "restored from saved original: {} {}",
                        client.name,
                        client.install.file_name().and_then(|s| s.to_str()).unwrap_or("?")
                    ));
                }
            }
        }
    }
    let _ = std::fs::remove_file(store.join("state.json"));
    if steps.is_empty() {
        steps.push("Nothing to remove.".into());
    }
    steps.push("Sky shaders off. Close Roblox fully and start it again.".into());
    Ok(steps.join("\n"))
}

pub fn prepare_original() -> Result<(PathBuf, Vec<u8>)> {
    let clients = find_clients()?;
    if clients.is_empty() {
        bail!("Could not find any Roblox install with a shaders folder.");
    }
    let store = store_dir()?;
    let path = ensure_original(&clients, &store)?;
    let bytes = std::fs::read(&path)?;
    Ok((path, bytes))
}
