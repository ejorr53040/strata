//! Per-repo daemon: holds the index in memory and answers `strata` clients over a Unix
//! socket. Started by the client (`strata-daemon <root> <socket> <idle-secs>`), never by
//! hand; `idle-secs` is the effective `daemon.idle_timeout_secs` (0: never exit when idle).
//!
//! Protocol: one request per connection, one JSON line each way.
//! Request `{"build", "include", "cmd", "args"}`; reply `{"ok": text}`, `{"err": message}`,
//! or `{"stale_build": true}` when the client was built against a different daemon binary
//! (the daemon then exits so the client can start a fresh one).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use strata_core::graph::Edge;
use strata_core::snapshot::Snapshot;
use strata_core::walk::WalkOptions;

/// Snapshots kept per distinct `index.include`; older ones are dropped past this.
const MAX_SNAPSHOTS: usize = 4;

struct Daemon {
    root: PathBuf,
    socket: PathBuf,
    build: String,
    snapshots: Mutex<HashMap<String, (Arc<Snapshot>, Instant)>>,
    last_request: Mutex<Instant>,
    /// Requests being answered; the daemon never exits idle while one is open.
    in_flight: AtomicUsize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, socket, idle] = &args[..] else {
        eprintln!("usage: strata-daemon <root> <socket> <idle-secs>");
        std::process::exit(2);
    };
    let idle: u64 = idle.parse().unwrap_or_else(|_| {
        eprintln!("idle-secs must be a whole number of seconds, got `{idle}`");
        std::process::exit(2);
    });
    let exe = std::env::current_exe().expect("own executable path");
    let daemon = Arc::new(Daemon {
        root: root.into(),
        socket: socket.into(),
        build: strata_config::build_id(&exe).expect("own executable is readable"),
        snapshots: Mutex::default(),
        last_request: Mutex::new(Instant::now()),
        in_flight: AtomicUsize::new(0),
    });
    // The client holds the spawn lock, so whatever is at the path is a dead daemon's socket.
    let _ = std::fs::remove_file(&daemon.socket);
    let listener = UnixListener::bind(&daemon.socket).unwrap_or_else(|e| {
        eprintln!("bind {}: {e}", daemon.socket.display());
        std::process::exit(1);
    });
    std::fs::set_permissions(&daemon.socket, std::fs::Permissions::from_mode(0o600))
        .expect("restrict socket to its owner");
    if idle > 0 {
        let d = daemon.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            let quiet = d.last_request.lock().unwrap().elapsed() > Duration::from_secs(idle);
            if quiet && d.in_flight.load(Ordering::SeqCst) == 0 {
                d.exit();
            }
        });
    }
    for stream in listener.incoming().flatten() {
        let d = daemon.clone();
        std::thread::spawn(move || d.serve(stream));
    }
}

impl Daemon {
    /// Remove the socket before exiting, so a client never connects to a dying daemon and
    /// a successor's socket is never deleted by its predecessor.
    fn exit(&self) -> ! {
        let _ = std::fs::remove_file(&self.socket);
        std::process::exit(0);
    }

    fn serve(&self, stream: UnixStream) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        self.answer(stream);
        *self.last_request.lock().unwrap() = Instant::now();
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    fn answer(&self, stream: UnixStream) {
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            return;
        }
        let (reply, exit) = match serde_json::from_str::<Value>(&line) {
            Ok(req) => self.handle(&req),
            Err(e) => (json!({"err": format!("bad request: {e}")}), false),
        };
        if exit {
            let _ = std::fs::remove_file(&self.socket);
        }
        let mut out = &stream;
        let _ = writeln!(out, "{reply}");
        if exit {
            self.exit();
        }
    }

    /// The reply, and whether the daemon exits once it is sent.
    fn handle(&self, req: &Value) -> (Value, bool) {
        let cmd = req["cmd"].as_str().unwrap_or_default();
        let include = req["include"].as_str().unwrap_or_default();
        match cmd {
            "status" => return (json!({"ok": self.status(include)}), false),
            "stop" => return (json!({"ok": "stopped\n"}), true),
            _ => {}
        }
        if req["build"].as_str() != Some(&self.build) {
            return (json!({"stale_build": true}), true);
        }
        let snap = self.snapshot(include);
        match render(&snap, cmd, &req["args"]) {
            Ok(text) => (json!({"ok": text}), false),
            Err(e) => (json!({"err": e}), false),
        }
    }

    fn status(&self, include: &str) -> String {
        let snaps = self.snapshots.lock().unwrap();
        let index = match snaps.get(include) {
            Some((s, built)) => format!(
                " files={} builds={} reindexed={} age={}s",
                s.indexed_files(),
                s.builds(),
                s.reindexed(),
                built.elapsed().as_secs()
            ),
            None => " (not indexed yet)".into(),
        };
        format!(
            "running pid={} build={}{index}\n",
            std::process::id(),
            self.build
        )
    }

    /// The snapshot for `include`, rebuilt if any indexed file changed since it was built.
    fn snapshot(&self, include: &str) -> Arc<Snapshot> {
        let cached = self.snapshots.lock().unwrap().get(include).cloned();
        if let Some((s, _)) = cached {
            if !s.is_stale() {
                return s;
            }
        }
        // Updates are serialized: concurrent queries on a stale index wait for one update.
        let mut snaps = self.snapshots.lock().unwrap();
        let prev = snaps.get(include).map(|(s, _)| s.clone());
        if let Some(s) = &prev {
            if !s.is_stale() {
                return s.clone();
            }
        }
        let opts = WalkOptions::from_config_value(include);
        let s = Arc::new(match prev {
            Some(p) => p.update(&self.root, &opts),
            None => Snapshot::build(&self.root, &opts),
        });
        if snaps.len() >= MAX_SNAPSHOTS && !snaps.contains_key(include) {
            let oldest = snaps
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone());
            snaps.remove(&oldest.unwrap());
        }
        snaps.insert(include.to_string(), (s.clone(), Instant::now()));
        s
    }
}

/// The exact stdout of the `strata` subcommand `cmd`.
fn render(snap: &Snapshot, cmd: &str, args: &Value) -> Result<String, String> {
    let s = |k: &str| args[k].as_str().unwrap_or_default();
    let json_out = args["json"].as_bool().unwrap_or(false);
    let g = snap.graph();
    Ok(match cmd {
        "callers" => edges(g.callers(s("symbol")), json_out),
        "refs" => edges(g.references(s("symbol")), json_out),
        "callees" => edges(g.callees(s("symbol")), json_out),
        "symbols" => {
            let syms = snap.symbols(s("query"));
            if json_out {
                format!("{}\n", serde_json::to_string(&syms).unwrap())
            } else {
                syms.iter()
                    .map(|s| format!("{} {} {}:{}\n", s.kind, s.name, s.path, s.line))
                    .collect()
            }
        }
        "files_unindexed" => snap
            .unindexed()
            .iter()
            .map(|s| format!("unindexed {} {}\n", s.rel, s.reason.label()))
            .collect(),
        "map" => {
            let level = args["level"].as_u64().map(|l| l as u8);
            let budget = args["budget"].as_u64().map(|b| b as usize);
            snap.map(level, budget).text.clone()
        }
        "tools" => format!(
            "{}\n",
            serde_json::to_string_pretty(&strata_core::tools::catalog()).unwrap()
        ),
        "call" => {
            let out = snap.call(s("tool"), &args["args"])?;
            format!("{}\n", serde_json::to_string(&out).unwrap())
        }
        _ => return Err(format!("unknown daemon command `{cmd}`")),
    })
}

fn edges(edges: Vec<&Edge>, json_out: bool) -> String {
    if json_out {
        format!("{}\n", serde_json::to_string(&edges).unwrap())
    } else {
        edges.iter().map(|e| format!("{}\n", e.to_line())).collect()
    }
}
