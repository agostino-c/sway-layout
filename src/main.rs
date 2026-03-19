mod cli;
mod config;
mod ipc;
mod layout;
mod proc;
mod shortcuts;
mod spawn;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { profile, force } => layout::run(&profile, force)?,
        Command::List                   => layout::list()?,
        Command::SyncShortcuts          => shortcuts::sync()?,
        Command::Spawn { cmd, .. }      => spawn::run(&cmd)?,
    }
    Ok(())
}
