mod anim;
mod audio;
mod changelog;
mod cli;
mod config;
mod format;
mod gui;
mod instance;
mod originals;
mod packs;
mod protection;
mod resolve;
mod settings;
mod store;
mod texture;
mod theme;
mod tray;
mod update;
mod viewers;
mod watch;

use anyhow::{Context, Result};
use clap::Parser;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn FreeConsole() -> i32;
}

fn main() -> Result<()> {
    let args = cli::Args::parse();
    let dir = args.dir.or_else(store::Store::default_dir).context("could not locate the Roblox folder; pass --dir")?;
    let idmap = args.idmap.unwrap_or_else(config::IdMap::default_path);
    match args.cmd {
        cmd @ (None | Some(cli::Cmd::Gui { .. })) => {
            let tab = match cmd {
                Some(cli::Cmd::Gui { tab }) => tab,
                _ => None,
            };
            // Started with no arguments, usually by double-click: drop the console
            // window Windows opened for us. With it gone, a failure to start has
            // nowhere to print, so it is shown in a dialog instead.
            let detached = std::env::args().len() == 1;
            #[cfg(windows)]
            if detached {
                unsafe { FreeConsole() };
            }
            let result = gui::run(dir, idmap, tab.as_deref());
            if let (true, Err(error)) = (detached, &result) {
                rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("rbxcache").set_description(format!("{error:#}")).show();
            }
            result
        }
        Some(cmd) => cli::run(&dir, &idmap, cmd),
    }
}
