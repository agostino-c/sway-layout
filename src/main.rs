mod cli;
mod config;
mod ipc;
mod layout;
mod proc;
mod spawn;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Startup { force } => layout::startup(force)?,
        Command::Run { name }      => layout::run(&name)?,
        Command::List              => layout::list()?,
        Command::Spawn { cmd, .. } => spawn::run(&cmd)?,
    }
    Ok(())
}
