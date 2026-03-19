# sway-layout

A declarative layout manager for the [Sway](https://swaywm.org/) window manager. Define your workspace layouts as JSON profiles and restore them with a single command (or keybinding).

## Features

- **Declarative JSON profiles** – describe which apps go where using nested containers with `splith`, `splitv`, `tabbed`, or `stacking` layouts
- **Multi-workspace support** – a single profile can span several workspaces
- **Safe by default** – skips workspaces that already have windows (override with `--force`)
- **Keybinding sync** – generate and reload Sway `bindsym` includes directly from your profiles
- **Lightweight** – pure Rust binary, no runtime dependencies beyond Sway itself

## Requirements

- [Sway](https://swaywm.org/) (provides the IPC socket used to orchestrate windows)
- [Rust toolchain](https://rustup.rs/) 1.85+ (edition 2024) – only needed to build from source

## Installation

```bash
git clone https://github.com/agostino-c/rustic-swayland.git
cd rustic-swayland/sway-layout
cargo build --release
# copy the binary somewhere on your PATH
cp target/release/sway-layout ~/.local/bin/
```

## Configuration

Profiles are JSON files stored in `~/.config/sway/layouts/`. The filename (without `.json`) is the profile name used on the command line.

### Profile structure

```json
{
  "shortcut": "$mod+Shift+w",
  "workspaces": {
    "1": { ... },
    "2": { ... }
  }
}
```

| Field | Type | Description |
|---|---|---|
| `shortcut` | string (optional) | Sway keybinding that triggers `sway-layout run <profile>` |
| `workspaces` | object | Map of workspace name → layout definition |

### Layout definitions

A layout definition is either:

- **A string** – a shell command to launch a single app
- **An object** with a single layout key (`splith`, `splitv`, `tabbed`, or `stacking`) whose value is an array of child layout definitions (strings or nested objects)

#### Example: side-by-side editor and terminal

```json
{
  "shortcut": "$mod+Shift+d",
  "workspaces": {
    "1": {
      "splith": [
        "code --new-window",
        "foot"
      ]
    }
  }
}
```

#### Example: complex multi-workspace layout

```json
{
  "shortcut": "$mod+Shift+w",
  "workspaces": {
    "1": {
      "splith": [
        "firefox",
        {
          "splitv": [
            "foot",
            "foot -e htop"
          ]
        }
      ]
    },
    "2": {
      "tabbed": [
        "thunderbird",
        "telegram-desktop"
      ]
    }
  }
}
```

This produces:

```
Workspace 1                  Workspace 2
┌──────────┬──────────┐      ┌────────────────────┐
│          │   foot   │      │ thunderbird        │
│ firefox  ├──────────┤      ├────────────────────┤
│          │   htop   │      │ telegram-desktop   │
└──────────┴──────────┘      └────────────────────┘
```

## Usage

### Run a profile

```
sway-layout run <profile> [--force]
```

Launches every app defined in the profile, waits for their windows to appear, then arranges them into the configured layout.

| Flag | Description |
|---|---|
| `--force` / `-f` | Apply the layout even if target workspaces already have windows |

```bash
sway-layout run dev
sway-layout run dev --force
```

### List available profiles

```
sway-layout list
```

Prints all profile names found in `~/.config/sway/layouts/`.

### Sync keybindings

```
sway-layout sync-shortcuts
```

Reads every profile that has a `shortcut` field, writes the corresponding `bindsym` lines to `~/.config/sway/layout-bindings`, and calls `swaymsg reload`. Add the following line to your Sway config to include the generated file:

```
include ~/.config/sway/layout-bindings
```

## Sway integration

1. Add `include ~/.config/sway/layout-bindings` to your `~/.config/sway/config`.
2. Run `sway-layout sync-shortcuts` once (or on login) to generate the file.
3. Sway will now bind each profile's shortcut automatically.

## How it works

1. **Parse** the JSON profile and build an in-memory layout tree.
2. **Check** (unless `--force`) that target workspaces are empty.
3. **Launch** each app via an internal `spawn` wrapper that encodes the target workspace and position into its process command-line.
4. **Listen** for `window::new` events on the Sway IPC socket.
5. **Identify** each new window by walking `/proc` to read the spawn wrapper's arguments.
6. **Arrange** windows using `swaymsg` commands (`move scratchpad`, `layout`, `move to mark`) to reconstruct the declared container tree.
