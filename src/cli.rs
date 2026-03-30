use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sway-layout", about = "Declarative layout manager for Sway")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the startup sequence (reads startup.json, assigns workspaces 1, 2, ...)
    Startup {
        /// Apply even if target workspaces already have windows
        #[arg(long, short)]
        force: bool,
    },

    /// Launch a workspace definition into the next free workspace number
    Run {
        /// Workspace definition name (matches filename in layouts dir without .json)
        name: String,
    },

    /// List available workspace definitions
    List,

    /// Write sway bindsym includes from startup.json shortcuts, then reload sway
    SyncShortcuts,

    /// Run persistently, re-applying workspace layouts whenever they change
    Daemon,

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
