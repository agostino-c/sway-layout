use anyhow::{Context, Result};

pub fn sync() -> Result<()> {
    let layouts_dir = crate::layout::layouts_dir();
    let home        = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let output      = std::path::PathBuf::from(home).join(".config/sway/layout-bindings");

    let mut lines: Vec<String> = std::fs::read_dir(&layouts_dir)
        .with_context(|| format!("reading {}", layouts_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let path     = e.path();
            let name     = path.file_stem()?.to_str()?.to_string();
            let profile  = crate::config::load_profile(&path).ok()?;
            let shortcut = profile.shortcut?;
            Some(format!("bindsym {shortcut} exec sway-layout run {name}"))
        })
        .collect();

    lines.sort(); // deterministic output
    std::fs::write(&output, lines.join("\n") + "\n")?;
    println!("Wrote {} binding(s) to {}", lines.len(), output.display());

    let ok = std::process::Command::new("swaymsg")
        .arg("reload")
        .status()
        .context("swaymsg reload")?
        .success();

    if !ok { eprintln!("swaymsg reload failed"); }
    Ok(())
}
