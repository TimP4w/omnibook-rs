use std::io::{self, BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::presence_config::PresenceConfig;

// ── Daemon side ───────────────────────────────────────────────────────────────

struct Conn {
    stream: UnixStream,
    buf: String,
}

pub struct DaemonIpc {
    listener: UnixListener,
    conns: Vec<Conn>,
}

impl DaemonIpc {
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() { let _ = std::fs::remove_file(path); }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, conns: Vec::new() })
    }

    fn accept_pending(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    self.conns.push(Conn { stream, buf: String::new() });
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn drain_commands(&mut self) -> Vec<PresenceConfig> {
        let mut updates = Vec::new();
        let mut dead = Vec::new();
        let mut tmp = [0u8; 8192];

        for (i, conn) in self.conns.iter_mut().enumerate() {
            loop {
                match conn.stream.read(&mut tmp) {
                    Ok(0) => { dead.push(i); break; }
                    Ok(n) => conn.buf.push_str(&String::from_utf8_lossy(&tmp[..n])),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => { dead.push(i); break; }
                }
            }
            while let Some(cfg) = extract_set_config(&mut conn.buf) {
                updates.push(cfg);
            }
        }
        for i in dead.into_iter().rev() { self.conns.swap_remove(i); }
        updates
    }

    pub fn push_state(&mut self, prox: &str, attn: &str) {
        let msg = format!("STATE presence={} attention={}\n", prox, attn);
        let bytes = msg.as_bytes();
        let mut dead = Vec::new();
        for (i, conn) in self.conns.iter_mut().enumerate() {
            if conn.stream.write_all(bytes).is_err() {
                dead.push(i);
            }
        }
        for i in dead.into_iter().rev() { self.conns.swap_remove(i); }
    }

    /// Accept new connections and drain any incoming SET_CONFIG commands.
    pub fn accept_and_drain(&mut self) -> Vec<PresenceConfig> {
        self.accept_pending();
        self.drain_commands()
    }
}

/// Extract the first complete SET_CONFIG block from `buf`, consuming it.
fn extract_set_config(buf: &mut String) -> Option<PresenceConfig> {
    const CMD: &str = "SET_CONFIG\n";
    const END: &str = "END\n";
    let start = buf.find(CMD)?;
    let body_start = start + CMD.len();
    let end_off = buf[body_start..].find(END)?;
    let body = buf[body_start..body_start + end_off].to_string();
    *buf = buf[body_start + end_off + END.len()..].to_string();
    Some(PresenceConfig::parse(&body))
}

// ── Client side ───────────────────────────────────────────────────────────────

/// Connect to the daemon socket.
/// Returns `(writer, read_half)`: the writer is `Arc<Mutex<>>` for use across GTK
/// callbacks on the main thread; the read half is passed to `spawn_state_reader`.
pub fn connect(path: &Path) -> io::Result<(Arc<Mutex<UnixStream>>, UnixStream)> {
    let write_half = UnixStream::connect(path)?;
    let read_half = write_half.try_clone()?;
    Ok((Arc::new(Mutex::new(write_half)), read_half))
}

/// Send a SET_CONFIG command to the daemon.
pub fn send_set_config(writer: &Mutex<UnixStream>, cfg: &PresenceConfig) -> io::Result<()> {
    let msg = format!("SET_CONFIG\n{}END\n", cfg.serialize());
    writer.lock().unwrap().write_all(msg.as_bytes())
}

/// Spawn a background thread that reads STATE pushes and updates the shared
/// `(presence, attention)` pair so the GTK timer can read it without blocking.
pub fn spawn_state_reader(read_half: UnixStream, state: Arc<Mutex<(String, String)>>) {
    std::thread::spawn(move || {
        let reader = io::BufReader::new(read_half);
        for line in reader.lines().flatten() {
            if let Some(rest) = line.strip_prefix("STATE presence=") {
                if let Some((prox, attn)) = rest.split_once(" attention=") {
                    if let Ok(mut s) = state.lock() {
                        s.0 = prox.to_string();
                        s.1 = attn.to_string();
                    }
                }
            }
        }
    });
}
