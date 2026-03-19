use anyhow::{Context, Result};

pub fn sync() -> Result<()> {
    let layouts_dir = crate::layout::layouts_dir();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let output = std::path::PathBuf::from(home).join(".config/sway/layout-bindings");

    let entries = crate::config::load_startup(&layouts_dir)?;

    let lines: Vec<String> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            let shortcut = entry.shortcut.as_ref()?;
            let ws_num = i + 1;
            Some(format!("bindsym {shortcut} workspace {ws_num}"))
        })
        .collect();

    std::fs::write(&output, lines.join("\n") + "\n")?;
    println!("Wrote {} binding(s) to {}", lines.len(), output.display());

    let ok = std::process::Command::new("swaymsg")
        .arg("reload")
        .status()
        .context("swaymsg reload")?
        .success();

    if !ok {
        eprintln!("swaymsg reload failed");
    }
    Ok(())
}
