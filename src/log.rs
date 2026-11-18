//! A plain-text log, so a slow or failed sky build can be looked at afterwards.
//!
//! One file, `rbxcache.log`, in rbxcache's data folder. It keeps the last run's worth
//! of lines: past about half a megabyte it is moved to `rbxcache.log.old` and started again.

use std::io::Write;
use std::path::PathBuf;

const LIMIT: u64 = 512 * 1024;

pub fn path() -> Option<PathBuf> {
    crate::protection::data_dir().ok().map(|d| d.join("rbxcache.log"))
}

/// Appends one timestamped line. Never fails: a log that cannot be written is not worth an error.
pub fn write(message: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if path.metadata().is_ok_and(|m| m.len() > LIMIT) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        for line in message.lines() {
            let _ = writeln!(file, "{stamp}  {line}");
        }
    }
}

/// The last `count` lines of the log, newest last.
pub fn tail(count: usize) -> String {
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return String::new() };
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(count)..].join("\n")
}

/// Opens the log in the default text editor, creating it first if there is none yet.
pub fn open() {
    let Some(path) = path() else { return };
    if !path.is_file() {
        write("log created");
    }
    let _ = open::that(path);
}

/// Records a panic (with where it happened) before the program carries on or dies, so a
/// crash leaves a trace even when the window just vanishes.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        write(&format!("PANIC on thread '{name}': {info}"));
        previous(info);
    }));
}
