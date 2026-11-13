mod cli;
mod config;
mod format;
mod gui;
mod store;

use anyhow::{Context, Result};
use clap::Parser;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn FreeConsole() -> i32;
}

fn main() -> Result<()> {
    let args = cli::Args::parse();
    let dir = args
        .dir
        .or_else(store::Store::default_dir)
        .context("could not locate the Roblox folder; pass --dir")?;
    let idmap = args.idmap.unwrap_or_else(config::IdMap::default_path);
    match args.cmd {
        None | Some(cli::Cmd::Gui) => {
            // Started by double-click: drop the console window that Windows opened for us.
            #[cfg(windows)]
            if std::env::args().len() == 1 {
                unsafe { FreeConsole() };
            }
            gui::run(dir, idmap)
        }
        Some(cmd) => cli::run(&dir, &idmap, cmd),
    }
}


