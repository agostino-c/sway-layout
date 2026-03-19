use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

// ── Loading ───────────────────────────────────────────────────────────────────

pub fn load_workspace_def(path: &std::path::Path) -> Result<Value> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))
}

pub fn load_startup(layouts_dir: &std::path::Path) -> Result<Vec<String>> {
    let path = layouts_dir.join("startup.json");
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))
}

// ── App collection ────────────────────────────────────────────────────────────

pub struct AppInfo {
    pub cmd: String,
    pub workspace: String,
    pub path: String,
}

pub fn collect_apps(node: &LayoutNode, workspace: &str, apps: &mut Vec<AppInfo>) {
    match node {
        LayoutNode::App { path, cmd } if !cmd.is_empty() => {
            apps.push(AppInfo {
                cmd: cmd.clone(),
                workspace: workspace.to_string(),
                path: path.clone(),
            });
        }
        LayoutNode::App { .. } => {}
        LayoutNode::Container { children, .. } => {
            for child in children {
                collect_apps(child, workspace, apps);
            }
        }
    }
}

// ── Layout tree ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    SplitH,
    SplitV,
    Tabbed,
    Stacking,
}

impl Layout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SplitH => "splith",
            Self::SplitV => "splitv",
            Self::Tabbed => "tabbed",
            Self::Stacking => "stacking",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "splith" => Some(Self::SplitH),
            "splitv" => Some(Self::SplitV),
            "tabbed" => Some(Self::Tabbed),
            "stacking" => Some(Self::Stacking),
            _ => None,
        }
    }
}

// Vec<LayoutNode> is heap-allocated already — no extra Box needed
#[derive(Debug)]
pub enum LayoutNode {
    App {
        path: String,
        cmd: String,
    },
    Container {
        path: String,
        layout: Layout,
        children: Vec<LayoutNode>,
    },
}

// ── Parsing ───────────────────────────────────────────────────────────────────

pub fn build_layout_tree(def: &Value) -> Result<LayoutNode> {
    parse_node(def, "", Layout::SplitH)
}

fn parse_node(node: &Value, path: &str, parent_layout: Layout) -> Result<LayoutNode> {
    match node {
        Value::String(cmd) => Ok(LayoutNode::App {
            path: path.to_string(),
            cmd: cmd.clone(),
        }),

        Value::Object(map) => {
            let mut layout: Option<Layout> = None;
            let mut children_val: Option<&Value> = None;

            for (key, val) in map {
                match Layout::from_str(key) {
                    Some(l) => {
                        if layout.is_some() {
                            bail!("node at {path:?}: multiple layout keys");
                        }
                        layout = Some(l);
                        children_val = Some(val);
                    }
                    None => bail!(
                        "node at {path:?}: unknown key {key:?} \
                         (valid: splith, splitv, tabbed, stacking)"
                    ),
                }
            }

            let layout = layout.unwrap_or(parent_layout);
            let raw = children_val
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow!("node at {path:?}: layout value must be an array"))?;

            let children = raw
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let cp = if path.is_empty() {
                        i.to_string()
                    } else {
                        format!("{path}_{i}")
                    };
                    parse_node(child, &cp, layout)
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(LayoutNode::Container {
                path: path.to_string(),
                layout,
                children,
            })
        }

        other => bail!("node at {path:?}: expected string or object, got {other}"),
    }
}
