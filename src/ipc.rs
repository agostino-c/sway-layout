use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use anyhow::{Context, Result};

const MAGIC: &[u8; 6] = b"i3-ipc";

fn sock() -> Result<String> {
    std::env::var("SWAYSOCK").context("SWAYSOCK not set")
}

fn write_msg(conn: &mut UnixStream, typ: u32, payload: &[u8]) -> Result<()> {
    let mut buf = Vec::with_capacity(14 + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&typ.to_le_bytes());
    buf.extend_from_slice(payload);
    conn.write_all(&buf).context("ipc write")
}

fn read_msg(conn: &mut UnixStream) -> Result<Vec<u8>> {
    let mut hdr = [0u8; 14];
    conn.read_exact(&mut hdr).context("ipc read header")?;
    let len = u32::from_le_bytes(hdr[6..10].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    if len > 0 {
        conn.read_exact(&mut body).context("ipc read body")?;
    }
    Ok(body)
}

// ── Command connection ────────────────────────────────────────────────────────

pub struct SwayIPC(UnixStream);

impl SwayIPC {
    pub fn connect() -> Result<Self> {
        let path = sock()?;
        Ok(Self(UnixStream::connect(&path)
            .with_context(|| format!("connect {path}"))?))
    }

    fn roundtrip(&mut self, typ: u32, payload: &[u8]) -> Result<Vec<u8>> {
        write_msg(&mut self.0, typ, payload)?;
        read_msg(&mut self.0)
    }

    pub fn cmd(&mut self, s: &str) -> bool {
        let Ok(resp) = self.roundtrip(0, s.as_bytes()) else { return false };
        let Ok(results) = serde_json::from_slice::<Vec<serde_json::Value>>(&resp) else {
            return false;
        };
        results.iter().all(|r| {
            if r["success"] != true {
                eprintln!("  ipc err [{}]: {}", s, r["error"]);
                false
            } else {
                true
            }
        })
    }

    pub fn get_tree(&mut self) -> Result<serde_json::Value> {
        let resp = self.roundtrip(4, &[])?;
        Ok(serde_json::from_slice(&resp)?)
    }

    pub fn get_workspaces(&mut self) -> Result<Vec<String>> {
        let resp = self.roundtrip(1, &[])?;
        let ws: Vec<serde_json::Value> = serde_json::from_slice(&resp)?;
        Ok(ws.iter().filter_map(|w| w["name"].as_str().map(|s| s.to_string())).collect())
    }

    pub fn get_version(&mut self) -> Result<()> {
        self.roundtrip(7, &[])?;
        Ok(())
    }

    pub fn get_focused_workspace(&mut self) -> Result<Option<String>> {
        let resp = self.roundtrip(1, &[])?;
        let ws: Vec<serde_json::Value> = serde_json::from_slice(&resp)?;
        Ok(ws.iter()
            .find(|w| w["focused"].as_bool() == Some(true))
            .and_then(|w| w["name"].as_str())
            .map(|s| s.to_string()))
    }
}

// ── Window event ──────────────────────────────────────────────────────────────

pub struct WindowEvent {
    pub change: String,
    pub con_id: i64,
    pub pid:    i64,
}

// ── Combined event type ───────────────────────────────────────────────────────

#[allow(dead_code)]
pub enum SwayEvent {
    Window(WindowEvent),
    Workspace { change: String, name: String },
    Other,
}

// ── Event subscription (runs in its own thread) ───────────────────────────────

pub struct SwayEvents(UnixStream);

impl SwayEvents {
    pub fn connect() -> Result<Self> {
        let path = sock()?;
        let mut conn = UnixStream::connect(&path)?;
        write_msg(&mut conn, 2, b"[\"window\"]")?;
        read_msg(&mut conn)?; // discard subscribe ack
        Ok(Self(conn))
    }

    pub fn connect_multi(types: &[&str]) -> Result<Self> {
        let path = sock()?;
        let mut conn = UnixStream::connect(&path).with_context(|| format!("connect {path}"))?;
        let sub = serde_json::to_string(types)?;
        write_msg(&mut conn, 2, sub.as_bytes())?;
        read_msg(&mut conn)?;
        Ok(Self(conn))
    }

    /// Blocking — call from a background thread.
    pub fn next(&mut self) -> Result<WindowEvent> {
        loop {
            let body = read_msg(&mut self.0)?;
            let v: serde_json::Value = serde_json::from_slice(&body)?;
            let change = v["change"].as_str().unwrap_or("").to_string();
            let con_id = v["container"]["id"].as_i64().unwrap_or(0);
            let pid    = v["container"]["pid"].as_i64().unwrap_or(0);
            if con_id != 0 {
                return Ok(WindowEvent { change, con_id, pid });
            }
        }
    }

    pub fn next_event(&mut self) -> Result<SwayEvent> {
        loop {
            let body = read_msg(&mut self.0)?;
            let v: serde_json::Value = serde_json::from_slice(&body)?;
            let change = v["change"].as_str().unwrap_or("").to_string();
            if v.get("container").is_some() {
                let con_id = v["container"]["id"].as_i64().unwrap_or(0);
                let pid = v["container"]["pid"].as_i64().unwrap_or(0);
                if con_id != 0 {
                    return Ok(SwayEvent::Window(WindowEvent { change, con_id, pid }));
                }
            } else if let Some(name) = v["current"]["name"].as_str() {
                return Ok(SwayEvent::Workspace { change, name: name.to_string() });
            }
        }
    }
}
