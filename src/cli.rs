use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sway-layout", about = "Declarative layout manager for Sway")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Load a named layout profile
    Run {
        /// Profile name (matches filename in layouts dir without .json)
        profile: String,

        /// Apply even if target workspaces already have windows
        #[arg(long, short)]
        force: bool,
    },

    /// List available layout profiles
    List,

    /// Write sway bindsym includes from profile shortcuts, then reload sway
    SyncShortcuts,

    /// Internal: spawn wrapper that encodes ws+path in process cmdline
    #[command(hide = true)]
    Spawn {
        workspace: String,
        path: String,
        /// Remaining args are the command to exec
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
}
