# sway-layout

A declarative layout manager for the [Sway](https://swaywm.org/) window manager. Define your workspace layouts as JSON files and restore them on startup or on demand.

## Features

- **Declarative JSON workspaces** – describe which apps go where using nested containers with `splith`, `splitv`, `tabbed`, or `stacking` layouts
- **Startup sequence** – define which workspaces open at sway launch via `startup.json`
- **On-demand workspaces** – launch any workspace definition into the next free slot with a single command
- **Safe by default** – startup skips workspaces that already have windows (override with `--force`)
- **Lightweight** – pure Rust binary, no runtime dependencies beyond Sway itself

## Requirements

- [Sway](https://swaywm.org/) (provides the IPC socket used to orchestrate windows)
- [Rust toolchain](https://rustup.rs/) 1.85+ (edition 2024) – only needed to build from source

## Installation

```bash
git clone https://github.com/agostino-c/rustic-swayland.git
cd rustic-swayland/sway-layout
cargo build --release
cp target/release/sway-layout ~/.local/bin/
```

## Configuration

Workspace definition files live in `~/.config/sway/layouts/`. Each file is named after the workspace definition (without `.json`) and contains a bare layout definition — no wrapper object.

### Workspace definition format

A layout definition is either:

- **A string** – a shell command to launch a single app
- **An object** with a single layout key (`splith`, `splitv`, `tabbed`, or `stacking`) whose value is an array of child definitions

```json
{"tabbed": ["foot", "firefox"]}
```

```json
{
  "splith": [
    "firefox",
    {
      "splitv": [
        "foot",
        "foot -e htop"
      ]
    }
  ]
}
```

An empty container creates the workspace with the given layout but launches no apps:

```json
{"tabbed": []}
```

### Startup sequence

`startup.json` is an ordered list of workspace definition names. They are assigned to workspace numbers 1, 2, 3, ... in order.

```json
["mainframe", "free"]
```

This opens `mainframe.json` on workspace 1 and `free.json` on workspace 2 when Sway starts.

## Usage

### Run the startup sequence

```
sway-layout startup [--force]
```

Reads `startup.json`, launches each workspace in order. Skips workspaces that already have windows unless `--force` is passed.

Add to your Sway config to run on login:

```
exec sway-layout startup
```

### Launch a workspace on demand

```
sway-layout run <name>
```

Loads the named definition and opens it in the next free workspace number. Useful for launching a workspace from an app's action button or a keybinding:

```bash
sway-layout run work
```

### List available definitions

```
sway-layout list
```

Prints all workspace definition names found in `~/.config/sway/layouts/` (excludes `startup.json`).

## How it works

1. **Parse** the JSON definition and build an in-memory layout tree.
2. **Find** the target workspace (fixed slot for `startup`, next free number for `run`).
3. **Launch** each app via an internal `spawn` wrapper that encodes the target workspace and position into its process command-line.
4. **Listen** for `window::new` events on the Sway IPC socket.
5. **Identify** each new window by walking `/proc` to read the spawn wrapper's arguments.
6. **Arrange** windows using `swaymsg` commands (`move scratchpad`, `layout`, `move to mark`) to reconstruct the declared container tree.
