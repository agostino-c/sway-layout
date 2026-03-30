use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::os::unix::process::CommandExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::{AppInfo, LayoutNode, build_layout_tree, collect_apps};
use crate::ipc::{SwayEvent, SwayEvents, SwayIPC, WindowEvent};
use crate::proc::find_spawn_metadata;

pub fn layouts_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::PathBuf::from(home).join(".config/sway/layouts")
}

pub fn wait_for_ipc() -> Result<SwayIPC> {
    for _ in 0..50 {
        if let Ok(mut ipc) = SwayIPC::connect() {
            if ipc.get_version().is_ok() {
                return Ok(ipc);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("cannot connect to sway IPC after 5 seconds")
}

pub fn list() -> Result<()> {
    let dir = layouts_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .filter(|name| name != "startup")
        .collect();
    names.sort();
    names.iter().for_each(|n| println!("{n}"));
    Ok(())
}

/// Run the startup sequence: assigns workspace 1, 2, 3... to each entry in startup.json.
pub fn startup(force: bool) -> Result<()> {
    let dir = layouts_dir();
    let entries = crate::config::load_startup(&dir)?;

    let mut workspace_trees: HashMap<String, LayoutNode> = HashMap::new();
    let mut all_apps: Vec<AppInfo> = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        let ws = (i + 1).to_string();
        let path = dir.join(format!("{}.json", entry.name));
        let def = crate::config::load_workspace_def(&path)?;
        let tree = build_layout_tree(&def)?;
        collect_apps(&tree, &ws, &mut all_apps);
        workspace_trees.insert(ws, tree);
    }

    let mut ipc = wait_for_ipc()?;

    if !force {
        let occupied = workspaces_with_windows(&mut ipc)?;
        let conflicts: Vec<_> = workspace_trees
            .keys()
            .filter(|ws| occupied.contains(*ws))
            .cloned()
            .collect();
        if !conflicts.is_empty() {
            println!("Workspaces {conflicts:?} already have windows. Use --force to override.");
            return Ok(());
        }
    }

    apply_workspaces(&mut ipc, all_apps, workspace_trees)?;
    ipc.cmd("workspace 1");
    println!("\nLayout complete.");
    Ok(())
}

/// Launch a workspace definition into the next free numbered workspace.
pub fn run(def_name: &str) -> Result<()> {
    let dir = layouts_dir();
    let path = dir.join(format!("{def_name}.json"));
    let def = crate::config::load_workspace_def(&path)?;

    let mut ipc = wait_for_ipc()?;

    let used: HashSet<u32> = ipc
        .get_workspaces()?
        .iter()
        .filter_map(|n| n.parse::<u32>().ok())
        .collect();
    let ws_num = (1u32..).find(|n| !used.contains(n)).unwrap();
    let ws = ws_num.to_string();

    println!("Launching workspace definition '{def_name}' into workspace {ws}");

    let tree = build_layout_tree(&def)?;
    let mut all_apps: Vec<AppInfo> = Vec::new();
    collect_apps(&tree, &ws, &mut all_apps);

    let mut workspace_trees = HashMap::new();
    workspace_trees.insert(ws.clone(), tree);

    apply_workspaces(&mut ipc, all_apps, workspace_trees)?;
    ipc.cmd(&format!("workspace {ws}"));
    println!("\nLayout complete.");
    Ok(())
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn apply_workspaces(
    ipc: &mut SwayIPC,
    all_apps: Vec<AppInfo>,
    workspace_trees: HashMap<String, LayoutNode>,
) -> Result<()> {
    if !all_apps.is_empty() {
        println!("Launching {} apps...", all_apps.len());

        let (tx, rx) = mpsc::channel::<WindowEvent>();
        std::thread::spawn(move || {
            let mut events = match SwayEvents::connect() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("events: {e}");
                    return;
                }
            };
            loop {
                match events.next() {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let expected = launch_apps(&all_apps)?;
        let placed = collect_windows(rx, expected, ipc);

        println!("\nArranging layouts...");
        for (ws, tree) in &workspace_trees {
            let windows = placed.get(ws).cloned().unwrap_or_default();
            arrange_workspace(ipc, ws, &tree, &windows);
        }
    } else {
        // No apps — just create workspaces with correct layout
        for (ws, tree) in &workspace_trees {
            arrange_workspace(ipc, ws, &tree, &HashMap::new());
        }
    }

    Ok(())
}

fn launch_apps(apps: &[AppInfo]) -> Result<usize> {
    let self_exe = std::env::current_exe()?;
    let mut count = 0;

    for app in apps {
        println!(
            "  Launching: {} (ws={}, path={})",
            app.cmd, app.workspace, app.path
        );

        let result = unsafe {
            std::process::Command::new(&self_exe)
                .args(["spawn", &app.workspace, &app.path, &app.cmd])
                .pre_exec(|| {
                    nix::unistd::setsid()
                        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "setsid"))?;
                    Ok(())
                })
                .spawn()
        };

        match result {
            Ok(child) => {
                println!("    pid: {}", child.id());
                count += 1;
            }
            Err(e) => eprintln!("    failed: {e}"),
        }
    }

    Ok(count)
}

fn collect_windows(
    rx: mpsc::Receiver<WindowEvent>,
    expected: usize,
    ipc: &mut SwayIPC,
) -> HashMap<String, HashMap<String, Vec<i64>>> {
    println!("\nWaiting for {expected} windows...");

    let mut placed: HashMap<String, HashMap<String, Vec<i64>>> = HashMap::new();
    let mut count = 0usize;
    let quiet_dur = Duration::from_secs(3);
    let hard_stop = Instant::now() + Duration::from_secs(30);
    let mut quiet_dl = Instant::now() + quiet_dur;

    loop {
        let timeout = quiet_dl
            .min(hard_stop)
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(50));

        match rx.recv_timeout(timeout) {
            Ok(ev) if ev.change == "new" && ev.con_id != 0 && ev.pid != 0 => {
                println!("  New window: con_id={}, pid={}", ev.con_id, ev.pid);
                ipc.cmd(&format!("[con_id={}] move scratchpad", ev.con_id));

                match find_spawn_metadata(ev.pid) {
                    Some((ws, path)) => {
                        println!("    Tracked: workspace={ws}, path={path}");
                        placed
                            .entry(ws)
                            .or_default()
                            .entry(path)
                            .or_default()
                            .push(ev.con_id);
                        count += 1;
                        quiet_dl = Instant::now() + quiet_dur;
                    }
                    None => {
                        println!("    No metadata (external window)");
                        ipc.cmd(&format!("[con_id={}] scratchpad show", ev.con_id));
                    }
                }
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if count >= expected {
            println!("\nAll {expected} windows placed");
            break;
        }
        if Instant::now() >= quiet_dl || Instant::now() >= hard_stop {
            println!("\nTimeout: {count}/{expected} windows placed");
            break;
        }
    }

    placed
}

fn workspaces_with_windows(ipc: &mut SwayIPC) -> Result<HashSet<String>> {
    let tree = ipc.get_tree()?;
    let mut result = HashSet::new();
    walk_tree(&tree, "", &mut result);
    Ok(result)
}

fn walk_tree(node: &serde_json::Value, ws: &str, out: &mut HashSet<String>) {
    let ws = if node["type"].as_str() == Some("workspace") {
        node["name"].as_str().unwrap_or(ws)
    } else {
        ws
    };

    if node.get("app_id").is_some() && !ws.is_empty() {
        out.insert(ws.to_string());
    }

    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node[key].as_array() {
            for child in children {
                walk_tree(child, ws, out);
            }
        }
    }
}

fn arrange_workspace(
    ipc: &mut SwayIPC,
    ws: &str,
    tree: &LayoutNode,
    windows: &HashMap<String, Vec<i64>>,
) {
    let root_layout = match tree {
        LayoutNode::Container { layout, .. } => layout.as_str(),
        _ => "splith",
    };

    ipc.cmd(&format!("workspace {ws}"));
    ipc.cmd(&format!("layout {root_layout}"));

    if windows.is_empty() {
        return;
    }

    let total = windows.values().map(|v| v.len()).sum::<usize>();
    println!("  Arranging {ws}: {total} windows, root={root_layout}");
    arrange_tree(ipc, ws, tree, windows, 0);
}

fn first_window(node: &LayoutNode, windows: &HashMap<String, Vec<i64>>) -> Option<i64> {
    match node {
        LayoutNode::App { path, .. } => windows.get(path)?.first().copied(),
        LayoutNode::Container { children, .. } => {
            children.iter().find_map(|c| first_window(c, windows))
        }
    }
}

fn place_window(ipc: &mut SwayIPC, w: i64, depth: usize, n: usize, mark: &str, layout: &str) {
    if n > 0 || depth == 0 {
        ipc.cmd(&format!("[con_id={w}] scratchpad show"));
        ipc.cmd(&format!("[con_id={w}] floating disable"));
    }
    if n == 0 && depth > 0 {
        ipc.cmd(&format!("[con_id={w}] splith"));
        ipc.cmd(&format!("[con_id={w}] layout {layout}"));
    } else if n > 0 {
        ipc.cmd(&format!("[con_id={w}] move to mark {mark}"));
    }
    ipc.cmd(&format!("[con_id={w}] mark --add {mark}"));
}

fn arrange_tree(
    ipc: &mut SwayIPC,
    ws: &str,
    node: &LayoutNode,
    windows: &HashMap<String, Vec<i64>>,
    depth: usize,
) {
    let LayoutNode::Container {
        path,
        layout,
        children,
    } = node
    else {
        return;
    };

    let mark = if path.is_empty() {
        format!("_layout_{ws}")
    } else {
        format!("_layout_{ws}_{path}")
    };
    let layout_str = layout.as_str();
    let mut n = 0usize;
    let mut deferred: Vec<&LayoutNode> = Vec::new();

    for child in children {
        match child {
            LayoutNode::App {
                path: child_path, ..
            } => {
                if let Some(wins) = windows.get(child_path) {
                    for &w in wins {
                        place_window(ipc, w, depth, n, &mark, layout_str);
                        n += 1;
                    }
                }
            }
            LayoutNode::Container { .. } => {
                if let Some(w) = first_window(child, windows) {
                    place_window(ipc, w, depth, n, &mark, layout_str);
                    n += 1;
                    deferred.push(child);
                }
            }
        }
    }

    for child in deferred {
        arrange_tree(ipc, ws, child, windows, depth + 1);
    }
}

pub fn daemon() -> Result<()> {
    let dir = layouts_dir();
    let entries = crate::config::load_startup(&dir)?;

    let mut ws_layouts: HashMap<String, String> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let ws = (i + 1).to_string();
        let def_path = dir.join(format!("{}.json", entry.name));
        match crate::config::load_workspace_def(&def_path).and_then(|d| build_layout_tree(&d)) {
            Ok(LayoutNode::Container { layout, .. }) => {
                ws_layouts.insert(ws, layout.as_str().to_string());
            }
            Ok(_) => {}
            Err(e) => eprintln!("warning: could not load {}: {e}", entry.name),
        }
    }

    if ws_layouts.is_empty() {
        anyhow::bail!("no workspace layouts found in startup.json");
    }

    println!(
        "sway-layout daemon: enforcing layouts for workspaces {:?}",
        {
            let mut keys: Vec<_> = ws_layouts.keys().collect();
            keys.sort();
            keys
        }
    );

    let mut events = SwayEvents::connect_multi(&["window", "workspace"])?;
    let mut ipc = wait_for_ipc()?;
    let mut current_ws = ipc.get_focused_workspace()?;

    loop {
        match events.next_event()? {
            SwayEvent::Workspace { change, name } if change == "focus" => {
                current_ws = Some(name.clone());
                if let Some(layout) = ws_layouts.get(&name) {
                    ipc.cmd(&format!("layout {layout}"));
                }
            }
            SwayEvent::Window(WindowEvent { change, .. }) if change == "new" => {
                if let Some(ws) = &current_ws {
                    if let Some(layout) = ws_layouts.get(ws) {
                        ipc.cmd(&format!("layout {layout}"));
                    }
                }
            }
            _ => {}
        }
    }
}
