//! Activity reports from ED Corai's MCP servers to the desktop window.
//!
//! The window only watches: each MCP server runs its tools itself and reports what it starts and
//! finishes. A closed or crashed window therefore never affects the AI app; reports are dropped.
//!
//! Transport: the window listens on `127.0.0.1` (a port the system picks) and writes the port and
//! a random token to `window.json` in the user's state folder. A server connects, sends `Hello`
//! with the token, then one JSON message per line. The window forgets a server's jobs when its
//! connection closes, so a server that dies mid-job does not leave the window busy.

use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One line on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Message {
    /// First line of a connection: the token from `window.json`, and who is calling.
    Hello {
        token: String,
        client: String,
        pid: u32,
    },
    /// A tool call began.
    Started {
        id: u64,
        tool: String,
        detail: String,
    },
    /// A tool call ended; `link` opens the resulting build, when there is one.
    Finished {
        id: u64,
        ok: bool,
        millis: u64,
        detail: String,
        link: Option<String>,
    },
    /// The AI app the server talks to, once it introduced itself.
    Client { name: String },
    /// From a second copy of the window: bring the running one to the front.
    Focus,
}

/// What `window.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

/// ED Corai's per-user state folder: `%LOCALAPPDATA%\ED Corai` on Windows,
/// `~/Library/Application Support/ED Corai` on macOS, `$XDG_STATE_HOME/ed-corai` elsewhere.
pub fn state_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        env("LOCALAPPDATA").map(|d| d.join("ED Corai"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support/ED Corai"))
    } else {
        env("XDG_STATE_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/state")))
            .map(|d| d.join("ed-corai"))
    }
}

/// Where the running window announces itself.
pub fn discovery_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("window.json"))
}

/// 128 random bits as hex, from the standard library's OS-seeded hasher keys.
fn random_token() -> String {
    let part = || {
        let mut h = std::hash::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// Writes `window.json` readable by this user only, via a temporary file and a rename.
fn write_discovery(path: &Path, d: &Discovery) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(
        serde_json::to_string(d)
            .map_err(std::io::Error::other)?
            .as_bytes(),
    )?;
    drop(f);
    std::fs::rename(tmp, path)
}

fn read_discovery(path: &Path) -> Option<Discovery> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// What the window learns from its listener.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A server connected and proved it knows the token.
    Connected { conn: u64, client: String, pid: u32 },
    /// A message from that server.
    Message { conn: u64, message: Message },
    /// The server's connection closed: its unfinished jobs are gone.
    Disconnected { conn: u64 },
    /// A second copy of the window was started and wants this one in front instead.
    FocusRequested,
}

/// The window's end. Removes `window.json` when dropped, unless another window replaced it.
#[derive(Debug)]
pub struct Listener {
    path: PathBuf,
    discovery: Discovery,
}

impl Listener {
    /// Listens on a free local port, announces it at `path` and calls `on_event` from background
    /// threads for every connection and message.
    ///
    /// # Errors
    /// The port cannot be opened or the announcement cannot be written.
    pub fn start(
        path: &Path,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let discovery = Discovery {
            port: socket.local_addr()?.port(),
            token: random_token(),
            pid: std::process::id(),
        };
        write_discovery(path, &discovery)?;
        let token = discovery.token.clone();
        let on_event = std::sync::Arc::new(on_event);
        std::thread::Builder::new()
            .name("activity-listener".into())
            .spawn(move || {
                let next = AtomicU64::new(1);
                for stream in socket.incoming().flatten() {
                    let conn = next.fetch_add(1, Ordering::Relaxed);
                    let (token, on_event) = (token.clone(), std::sync::Arc::clone(&on_event));
                    let _ = std::thread::Builder::new()
                        .name(format!("activity-{conn}"))
                        .spawn(move || serve(conn, stream, &token, &*on_event));
                }
            })?;
        Ok(Self {
            path: path.to_path_buf(),
            discovery,
        })
    }

    /// The port and token this window announced.
    pub fn discovery(&self) -> &Discovery {
        &self.discovery
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if read_discovery(&self.path).is_some_and(|d| d.token == self.discovery.token) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// One server's connection: the hello must carry the token, then messages until it closes.
fn serve(conn: u64, stream: TcpStream, token: &str, on_event: &(dyn Fn(Event) + Send + Sync)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let Ok(Message::Hello {
        token: t,
        client,
        pid,
    }) = serde_json::from_str(&first)
    else {
        return;
    };
    if t != token {
        return;
    }
    if client.is_empty() {
        // A control connection (another copy of the window), not an AI app.
        if reader.read_line(&mut first).is_ok()
            && let Some(line) = first.lines().last()
            && serde_json::from_str::<Message>(line).is_ok_and(|m| m == Message::Focus)
        {
            on_event(Event::FocusRequested);
        }
        return;
    }
    // After the hello, a server may stay quiet for as long as the AI app runs.
    let _ = reader.get_ref().set_read_timeout(None);
    on_event(Event::Connected { conn, client, pid });
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Ok(message) = serde_json::from_str::<Message>(&line) {
            on_event(Event::Message { conn, message });
        }
    }
    on_event(Event::Disconnected { conn });
}

/// Asks a window already announced at `path` to come to the front. True when one answered, in
/// which case a second window should not start.
pub fn focus_running_window(path: &Path) -> bool {
    let Some(d) = read_discovery(path) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, d.port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    let hello = Message::Hello {
        token: d.token,
        client: String::new(),
        pid: std::process::id(),
    };
    let lines = [hello, Message::Focus]
        .iter()
        .filter_map(|m| serde_json::to_string(m).ok())
        .collect::<Vec<_>>()
        .join("\n");
    writeln!(s, "{lines}").is_ok()
}

/// The MCP server's end. Every report is best effort: without a window nothing is sent, and a
/// failed write drops the connection so the next report tries again.
#[derive(Debug)]
pub struct Reporter {
    path: Option<PathBuf>,
    stream: Mutex<Option<TcpStream>>,
    client: Mutex<String>,
    next_id: AtomicU64,
}

impl Default for Reporter {
    fn default() -> Self {
        Self::at(discovery_path())
    }
}

impl Reporter {
    /// A reporter that looks for the window announcement at `path` (`None`: never reports).
    pub fn at(path: Option<PathBuf>) -> Self {
        Self {
            path,
            stream: Mutex::new(None),
            client: Mutex::new("an AI app".into()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Names the AI app on this connection (and on later ones).
    pub fn set_client(&self, name: &str) {
        let changed = self.client.lock().is_ok_and(|mut c| {
            let changed = *c != name;
            *c = name.to_string();
            changed
        });
        // A new connection's hello carries the name; an open one is told separately.
        let connected = self.stream.lock().is_ok_and(|s| s.is_some());
        if changed && connected {
            self.send(&Message::Client { name: name.into() });
        }
    }

    /// A fresh id for a tool call.
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn connect(&self) -> Option<TcpStream> {
        let d = read_discovery(self.path.as_deref()?)?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, d.port));
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(200)).ok()?;
        s.set_write_timeout(Some(Duration::from_millis(500))).ok()?;
        let client = self.client.lock().map(|c| c.clone()).unwrap_or_default();
        let hello = Message::Hello {
            token: d.token,
            client,
            pid: std::process::id(),
        };
        writeln!(s, "{}", serde_json::to_string(&hello).ok()?).ok()?;
        Some(s)
    }

    /// Connects to the window now if it runs and this reporter is not connected yet, so the window
    /// lists the AI app before its first tool call.
    pub fn announce(&self) {
        if let Ok(mut slot) = self.stream.lock()
            && slot.is_none()
        {
            *slot = self.connect();
        }
    }

    /// Keeps announcing every `every` while the reporter lives, so a window opened later still
    /// sees the AI apps that are already running.
    pub fn keep_announcing(self: &std::sync::Arc<Self>, every: Duration) {
        let weak = std::sync::Arc::downgrade(self);
        let _ = std::thread::Builder::new()
            .name("activity-announce".into())
            .spawn(move || {
                while let Some(reporter) = weak.upgrade() {
                    reporter.announce();
                    drop(reporter);
                    std::thread::sleep(every);
                }
            });
    }

    /// Sends one message to the window, if one is running.
    pub fn send(&self, message: &Message) {
        let Ok(line) = serde_json::to_string(message) else {
            return;
        };
        let Ok(mut slot) = self.stream.lock() else {
            return;
        };
        for _ in 0..2 {
            if slot.is_none() {
                *slot = self.connect();
            }
            let Some(s) = slot.as_mut() else { return };
            if writeln!(s, "{line}").is_ok() {
                return;
            }
            *slot = None; // the window went away or restarted: try a fresh connection once
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("corai-activity-{}-{name}", std::process::id()))
            .join("window.json")
    }

    fn recv(rx: &mpsc::Receiver<Event>) -> Event {
        rx.recv_timeout(Duration::from_secs(5)).expect("an event")
    }

    #[test]
    fn reports_reach_the_window_and_a_closed_connection_is_noticed() {
        let path = temp_path("roundtrip");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let listener = Listener::start(&path, move |e| {
            let _ = tx.lock().map(|t| t.send(e));
        })
        .expect("listens");
        let announced = read_discovery(&path).expect("announced");
        assert_eq!(&announced, listener.discovery());
        assert_eq!(announced.token.len(), 32);

        let reporter = Reporter::at(Some(path.clone()));
        reporter.set_client("Claude Code");
        let id = reporter.next_id();
        reporter.send(&Message::Started {
            id,
            tool: "optimize_build".into(),
            detail: "Python".into(),
        });
        let Event::Connected { conn, client, .. } = recv(&rx) else {
            panic!("connected first")
        };
        assert_eq!(client, "Claude Code");
        assert_eq!(
            recv(&rx),
            Event::Message {
                conn,
                message: Message::Started {
                    id,
                    tool: "optimize_build".into(),
                    detail: "Python".into()
                }
            }
        );
        drop(reporter);
        assert_eq!(recv(&rx), Event::Disconnected { conn });

        drop(listener);
        assert!(
            !path.exists(),
            "the announcement is removed with the window"
        );
    }

    #[test]
    fn announcing_connects_before_any_report() {
        let path = temp_path("announce");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let _listener = Listener::start(&path, move |e| {
            let _ = tx.lock().map(|t| t.send(e));
        })
        .expect("listens");
        let reporter = Reporter::at(Some(path.clone()));
        reporter.announce();
        assert!(matches!(recv(&rx), Event::Connected { .. }));
        reporter.set_client("Claude Desktop");
        assert!(matches!(
            recv(&rx),
            Event::Message { message: Message::Client { name }, .. } if name == "Claude Desktop"
        ));
    }

    #[test]
    fn a_second_window_brings_the_first_forward() {
        let path = temp_path("focus");
        assert!(!focus_running_window(&path), "no window yet");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let _listener = Listener::start(&path, move |e| {
            let _ = tx.lock().map(|t| t.send(e));
        })
        .expect("listens");
        assert!(focus_running_window(&path));
        assert_eq!(recv(&rx), Event::FocusRequested);
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "not listed as an AI app"
        );
    }

    #[test]
    fn a_wrong_token_is_turned_away() {
        let path = temp_path("token");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let listener = Listener::start(&path, move |e| {
            let _ = tx.lock().map(|t| t.send(e));
        })
        .expect("listens");
        let mut forged = listener.discovery().clone();
        forged.token = "0".repeat(32);
        let forged_path = temp_path("token-forged");
        write_discovery(&forged_path, &forged).expect("written");
        let reporter = Reporter::at(Some(forged_path.clone()));
        reporter.send(&Message::Client { name: "x".into() });
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "nothing accepted"
        );
        let _ = std::fs::remove_dir_all(forged_path.parent().expect("dir"));
    }

    #[test]
    fn without_a_window_reports_are_dropped() {
        let reporter = Reporter::at(Some(temp_path("absent")));
        reporter.send(&Message::Client { name: "x".into() });
        Reporter::at(None).send(&Message::Client { name: "x".into() });
    }
}
