use std::collections::HashSet;

/// Walk up the process tree from `pid` looking for a sway-layout spawn wrapper.
/// Returns (workspace, path) if found.
pub fn find_spawn_metadata(pid: i64) -> Option<(String, String)> {
    let mut visited = HashSet::new();
    let mut cur = pid as u32;

    while cur > 1 {
        if !visited.insert(cur) { break; }
        if let Some(args) = read_cmdline(cur) {
            if has_spawn_marker(&args) {
                if let Some(result) = parse_spawn_args(&args) {
                    return Some(result);
                }
            }
        }
        cur = get_ppid(cur)?;
    }
    None
}

fn read_cmdline(pid: u32) -> Option<Vec<String>> {
    let data = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        data.split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

fn has_spawn_marker(args: &[String]) -> bool {
    args.iter().any(|a| a.contains("sway-layout"))
        && args.iter().any(|a| a == "spawn")
}

fn parse_spawn_args(args: &[String]) -> Option<(String, String)> {
    let i = args.iter().position(|a| a == "spawn")?;
    Some((args.get(i + 1)?.clone(), args.get(i + 2)?.clone()))
}

fn get_ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // format: pid (comm) state ppid ...  find last ')' to handle parens in comm
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}
