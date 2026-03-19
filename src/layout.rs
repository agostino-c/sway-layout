use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::os::unix::process::CommandExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::{AppInfo, LayoutNode, build_layout_tree, collect_apps};
use crate::ipc::{SwayEvents, SwayIPC, WindowEvent};
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
        .collect();
    names.sort();
    names.iter().for_each(|n| println!("{n}"));
    Ok(())
}

pub fn run(profile_name: &str, force: bool) -> Result<()> {
    let path = layouts_dir().join(format!("{profile_name}.json"));
    let profile = crate::config::load_profile(&path)?;

    let mut workspace_trees: HashMap<String, LayoutNode> = HashMap::new();
    let mut blank_workspaces: Vec<String> = Vec::new();
    let mut all_apps: Vec<AppInfo> = Vec::new();

    for (ws, def) in &profile.workspaces {
        if def.is_null() {
            blank_workspaces.push(ws.clone());
            continue;
        }
        let tree = build_layout_tree(def)?;
        collect_apps(&tree, ws, &mut all_apps);
        workspace_trees.insert(ws.clone(), tree);
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

    if all_apps.is_empty() && blank_workspaces.is_empty() {
        println!("No apps to launch");
        return Ok(());
    }

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
        let placed = collect_windows(rx, expected, &mut ipc);

        println!("\nArranging layouts...");
        for (ws, tree) in &workspace_trees {
            if let Some(windows) = placed.get(ws) {
                arrange_workspace(&mut ipc, ws, tree, windows);
            }
        }
    }

    for ws in &blank_workspaces {
        println!("Creating blank workspace {ws}");
        ipc.cmd(&format!("workspace {ws}"));
    }

    if let Some(first_ws) = profile.workspaces.keys().next() {
        println!("\nReturning to workspace {first_ws}");
        ipc.cmd(&format!("workspace {first_ws}"));
    }

    println!("\nLayout complete.");
    Ok(())
}

// ── Internal ──────────────────────────────────────────────────────────────────

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
    if windows.is_empty() {
        return;
    }

    let total = windows.values().map(|v| v.len()).sum::<usize>();
    let root_layout = match tree {
        LayoutNode::Container { layout, .. } => layout.as_str(),
        _ => "splith",
    };

    println!("  Arranging {ws}: {total} windows, root={root_layout}");
    ipc.cmd(&format!("workspace {ws}"));
    ipc.cmd(&format!("layout {root_layout}"));
    arrange_tree(ipc, tree, windows, 0);
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

    let mark = format!("_layout_{path}");
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

    // process subtrees after root level is fully populated —
    // mirrors Go defer behaviour without the footgun
    for child in deferred {
        arrange_tree(ipc, child, windows, depth + 1);
    }
}
