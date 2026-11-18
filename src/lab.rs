//! An experiment: does Roblox look at a cache entry again while a game is running?
//!
//! The chosen entries are given a new picture at a fixed interval, over and
//! over, and the cache index is watched for Roblox reading them. If Roblox only
//! reads an asset when it first loads it, the read count stays where it was and
//! nothing changes on screen, however fast the swapping. If the count climbs,
//! swapping that asset live works, and an animated sky is possible for it.

use crate::config;
use crate::store::{Entry, Id, Store};
use anyhow::{bail, Context, Result};
use md5::{Digest, Md5};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What to swap, with what, and how often.
pub struct Setup {
    pub dir: PathBuf,
    /// The entries as the window last saw them, for telling a texture's packs apart from other entries.
    pub entries: Vec<Entry>,
    pub targets: Vec<Id>,
    /// The pictures to cycle through, as image files.
    pub frames: Vec<Vec<u8>>,
    pub interval: Duration,
    /// Whether what the entries held is put back when the test stops.
    pub restore: bool,
}

/// How the test is going.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Times every target was given a new picture.
    pub swaps: u64,
    /// Times Roblox read a target since the test began.
    pub reads: i64,
    /// Times Roblox was found to have written a target itself between two swaps.
    pub overwritten: u64,
    /// How long the last swap took, in milliseconds.
    pub write_ms: f32,
    /// Which picture is in place now, counting from one.
    pub frame: usize,
}

pub enum Msg {
    Progress(Progress),
    /// The test ended. The entries as they are now, and what went wrong if it did not end by being stopped.
    Done(Vec<Entry>, Option<String>),
}

/// Solid colours round the colour wheel: pictures that cannot be mistaken for
/// each other or for anything a game would show.
pub fn colour_frames() -> Vec<Vec<u8>> {
    const COLOURS: [[u8; 3]; 8] = [[255, 0, 0], [255, 140, 0], [255, 235, 0], [0, 220, 0], [0, 220, 220], [0, 80, 255], [160, 0, 255], [255, 0, 180]];
    COLOURS
        .iter()
        .filter_map(|&[r, g, b]| {
            let mut png = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(64, 64, image::Rgba([r, g, b, 255])).write_to(&mut png, image::ImageFormat::Png).ok()?;
            Some(png.into_inner())
        })
        .collect()
}

/// The pictures in a folder, in name order.
pub fn load_frames(folder: &Path) -> Result<Vec<Vec<u8>>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("{} holds no pictures (png, jpg, webp or gif)", folder.display());
    }
    paths.iter().map(|path| std::fs::read(path).with_context(|| format!("reading {}", path.display()))).collect()
}

fn digest(bytes: &[u8]) -> [u8; 16] {
    Md5::digest(bytes).into()
}

/// How often Roblox has read these entries, as the index counts it.
fn hits(store: &Store, ids: &[Id]) -> HashMap<Id, i64> {
    ids.iter().filter_map(|id| Some((*id, store.entry(id).ok()?.hits))).collect()
}

/// Runs the test until `stop` is set, reporting after every swap, and ends with [`Msg::Done`].
pub fn run(setup: Setup, stop: &AtomicBool, report: impl Fn(Msg)) {
    let (entries, problem) = match swap_until_stopped(&setup, stop, &report) {
        Ok(entries) => (entries, None),
        Err(e) => (Vec::new(), Some(format!("{e:#}"))),
    };
    report(Msg::Done(entries, problem));
}

fn swap_until_stopped(setup: &Setup, stop: &AtomicBool, report: &impl Fn(Msg)) -> Result<Vec<Entry>> {
    if setup.targets.is_empty() || setup.frames.is_empty() {
        bail!("the test needs at least one entry and one picture");
    }
    let store = Store::open(&setup.dir, true)?;
    // A texture is several entries; all of them are swapped and watched.
    let mut all: Vec<Id> = Vec::new();
    for target in &setup.targets {
        for id in config::siblings(&store, &setup.entries, target) {
            if !all.contains(&id) {
                all.push(id);
            }
        }
    }
    let originals: Vec<(Id, Vec<u8>)> = all.iter().map(|id| Ok((*id, store.read_body(id)?))).collect::<Result<_>>()?;
    let before = hits(&store, &all);

    let mut progress = Progress::default();
    let mut written: HashMap<Id, [u8; 16]> = HashMap::new();
    let outcome = (|| -> Result<()> {
        while !stop.load(Ordering::Relaxed) {
            // Anything that is no longer what was written last was written by Roblox in between.
            for (id, last) in &written {
                if store.read_body(id).is_ok_and(|now| digest(&now) != *last) {
                    progress.overwritten += 1;
                }
            }
            let frame = &setup.frames[progress.swaps as usize % setup.frames.len()];
            let started = Instant::now();
            for target in &setup.targets {
                for (entry, body) in config::replace_by_hand(&store, &setup.entries, target, frame.clone())? {
                    written.insert(entry.id, digest(&body));
                }
            }
            progress.write_ms = started.elapsed().as_secs_f32() * 1000.0;
            progress.swaps += 1;
            progress.frame = (progress.swaps as usize - 1) % setup.frames.len() + 1;
            let now = hits(&store, &all);
            progress.reads = all.iter().map(|id| (now.get(id).copied().unwrap_or(0) - before.get(id).copied().unwrap_or(0)).max(0)).sum();
            report(Msg::Progress(progress.clone()));

            // Sleep in short steps so that stopping does not wait out a long interval.
            let until = Instant::now() + setup.interval;
            while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20).min(setup.interval));
            }
        }
        Ok(())
    })();

    // Every original is put back even if one of them fails, so no other entry keeps a test frame.
    let mut restore_error = None;
    if setup.restore {
        for (id, body) in &originals {
            if let Err(e) = store.replace_body(id, body) {
                restore_error.get_or_insert(e);
            }
        }
    }
    outcome?;
    if let Some(e) = restore_error {
        return Err(e.context("putting an original back"));
    }
    Ok(all.iter().filter_map(|id| store.entry(id).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_frames_are_distinct_pictures() {
        let frames = colour_frames();
        assert_eq!(frames.len(), 8);
        assert!(frames.iter().all(|frame| crate::format::sniff(frame) == "png"));
        let unique: std::collections::HashSet<[u8; 16]> = frames.iter().map(|frame| digest(frame)).collect();
        assert_eq!(unique.len(), frames.len());
    }

    #[test]
    fn the_test_swaps_counts_and_puts_the_original_back() {
        let (dir, ids) = crate::config::tests::cache_of("lab", &[b"OggS original"]);
        let store = Store::open(&dir, false).unwrap();
        let entries = store.list(|_, _| {}).unwrap();
        drop(store);
        let setup = Setup {
            dir: dir.clone(),
            entries,
            targets: ids.clone(),
            frames: vec![b"OggS one".to_vec(), b"OggS two".to_vec()],
            interval: Duration::from_millis(10),
            restore: true,
        };
        let stop = AtomicBool::new(false);
        let seen = std::sync::Mutex::new(Vec::new());
        let ended = std::sync::Mutex::new(None);
        std::thread::scope(|scope| {
            let (stop, seen, ended) = (&stop, &seen, &ended);
            scope.spawn(move || {
                run(setup, stop, |msg| match msg {
                    Msg::Progress(progress) => {
                        // Stopped from inside, after the third swap has been reported.
                        if progress.swaps >= 3 {
                            stop.store(true, Ordering::Relaxed);
                        }
                        seen.lock().unwrap().push(progress);
                    }
                    Msg::Done(entries, problem) => *ended.lock().unwrap() = Some((entries.len(), problem)),
                });
            });
        });
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.iter().map(|p| (p.swaps, p.frame)).collect::<Vec<_>>(), [(1, 1), (2, 2), (3, 1)]);
        // Nothing but the test touched the cache: no reads, and nobody else wrote.
        assert!(seen.iter().all(|p| p.reads == 0 && p.overwritten == 0));
        assert_eq!(ended.into_inner().unwrap(), Some((1, None)));
        assert_eq!(Store::open(&dir, false).unwrap().read_body(&ids[0]).unwrap(), b"OggS original");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frames_load_in_name_order_and_only_pictures() {
        let dir = std::env::temp_dir().join(format!("rbxcache-lab-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in [("b.png", "second"), ("a.png", "first"), ("notes.txt", "ignored")] {
            std::fs::write(dir.join(name), body).unwrap();
        }
        assert_eq!(load_frames(&dir).unwrap(), [b"first".to_vec(), b"second".to_vec()]);
        std::fs::remove_file(dir.join("a.png")).unwrap();
        std::fs::remove_file(dir.join("b.png")).unwrap();
        assert!(load_frames(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
