//! Per-repo daemon: holds the index in memory and answers `tesselate` clients over a Unix
//! socket. Started by the client (`tesselate-daemon <root> <socket> <idle-secs>`), never by
//! hand; `idle-secs` is the effective `daemon.idle_timeout_secs` (0: never exit when idle).
//!
//! Protocol: one request per connection, one JSON line each way.
//! Request `{"build", "include", "cmd", "args"}`; reply `{"ok": text}`, `{"err": message}`,
//! or `{"stale_build": true}` when the client was built against a different daemon binary
//! (the daemon then exits so the client can start a fresh one).

use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tesselate_core::graph::Edge;
use tesselate_core::snapshot::Snapshot;
use tesselate_core::walk::WalkOptions;

/// Snapshots kept per distinct `index.include`; older ones are dropped past this.
const MAX_SNAPSHOTS: usize = 4;

/// How long a query waits for the watcher to catch up before it stops trusting it.
const BARRIER_TIMEOUT: Duration = Duration::from_secs(2);

/// (events that may have changed a file, events that mean the watcher can no longer be
/// trusted): a snapshot built at one value is current while both are unchanged.
type Seen = (u64, u64);

struct Daemon {
    root: PathBuf,
    socket: PathBuf,
    build: String,
    snapshots: Mutex<HashMap<String, Entry>>,
    last_request: Mutex<Instant>,
    /// Requests being answered; the daemon never exits idle while one is open.
    in_flight: AtomicUsize,
    /// `None` when the OS watcher could not start: the size/mtime check is used instead.
    watch: Option<Watch>,
}

struct Entry {
    snap: Arc<Snapshot>,
    built: Instant,
    seen: Seen,
}

/// What the watcher thread has recorded.
struct Events {
    /// Sentinel file names start with this; it carries the pid because every daemon on the
    /// machine watches the same socket directory.
    sentinel_prefix: String,
    changes: AtomicU64,
    lost: AtomicU64,
    /// Barrier sentinels the watcher has seen and no query has claimed yet.
    synced: Mutex<HashSet<u64>>,
    arrived: Condvar,
}

/// A recursive watch on the repo. inotify delivers events to the watcher thread after the
/// write that caused them, so a query that follows an edit straight away could read a stale
/// count. [`Watch::sync`] closes that gap: events are ordered within one watcher, so once the
/// sentinel written into the socket's directory is seen, every earlier event has been.
struct Watch {
    events: Arc<Events>,
    next_sync: AtomicU64,
    /// The socket's directory, outside the repo: where sentinels are written.
    dir: PathBuf,
    _watcher: notify::RecommendedWatcher,
}

impl Events {
    fn record(&self, res: notify::Result<notify::Event>, root: &Path, dir: &Path, lose: bool) {
        let Ok(ev) = res else {
            self.lost.fetch_add(1, Ordering::SeqCst);
            return;
        };
        if ev.need_rescan() {
            self.lost.fetch_add(1, Ordering::SeqCst);
        }
        if matches!(ev.kind, EventKind::Access(_)) {
            return;
        }
        for p in &ev.paths {
            if p.parent() == Some(dir) {
                let n = p.file_name().and_then(|n| {
                    n.to_str()?
                        .strip_prefix(&self.sentinel_prefix)?
                        .parse()
                        .ok()
                });
                if let (EventKind::Create(_), Some(n)) = (ev.kind, n) {
                    self.synced.lock().unwrap().insert(n);
                    self.arrived.notify_all();
                }
            } else if p == dir {
                // Writing a sentinel touches the directory itself.
            } else if let Ok(rel) = p.strip_prefix(root) {
                // ponytail: git internals and `target` (cargo output, which the walker also
                // skips at any depth) are skipped by name; the walker's own ignore rules
                // would be exact, and an `index.include` under `target` is missed.
                let noise = rel
                    .components()
                    .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | "target")));
                if !noise {
                    let counter = if lose { &self.lost } else { &self.changes };
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            } else {
                // Neither in the repo nor the socket directory: the watcher is reporting
                // something we cannot place, so stop trusting "no change".
                self.lost.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// True once sentinel `n` has been seen, false if the watcher did not get to it in time.
    fn wait_for(&self, n: u64) -> bool {
        let seen = self.synced.lock().unwrap();
        let (mut seen, timeout) = self
            .arrived
            .wait_timeout_while(seen, BARRIER_TIMEOUT, |s| !s.contains(&n))
            .unwrap();
        !timeout.timed_out() && seen.remove(&n)
    }
}

impl Watch {
    fn start(root: &Path, socket: &Path) -> notify::Result<Watch> {
        let dir = socket.parent().unwrap_or(Path::new("/")).to_path_buf();
        let events = Arc::new(Events {
            sentinel_prefix: format!(".sync-{}-", std::process::id()),
            changes: AtomicU64::new(0),
            lost: AtomicU64::new(0),
            synced: Mutex::default(),
            arrived: Condvar::new(),
        });
        // Test hook: treat every event as lost, to exercise the full-rebuild fallback.
        let lose = std::env::var_os("TESSELATE_WATCHER_LOSE_EVENTS").is_some();
        let (sink, r, d) = (events.clone(), root.to_path_buf(), dir.clone());
        let mut watcher = notify::recommended_watcher(move |res| sink.record(res, &r, &d, lose))?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        Ok(Watch {
            events,
            next_sync: AtomicU64::new(0),
            dir,
            _watcher: watcher,
        })
    }

    /// What the watcher has seen up to now, including any edit made before this call.
    fn sync(&self) -> Seen {
        let n = self.next_sync.fetch_add(1, Ordering::SeqCst) + 1;
        let sentinel = self.dir.join(format!("{}{n}", self.events.sentinel_prefix));
        let caught_up = std::fs::write(&sentinel, b"").is_ok() && self.events.wait_for(n);
        let _ = std::fs::remove_file(&sentinel);
        if !caught_up {
            self.events.lost.fetch_add(1, Ordering::SeqCst);
        }
        (
            self.events.changes.load(Ordering::SeqCst),
            self.events.lost.load(Ordering::SeqCst),
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, root, socket, idle] = &args[..] else {
        eprintln!("usage: tesselate-daemon <root> <socket> <idle-secs>");
        std::process::exit(2);
    };
    let idle: u64 = idle.parse().unwrap_or_else(|_| {
        eprintln!("idle-secs must be a whole number of seconds, got `{idle}`");
        std::process::exit(2);
    });
    let exe = std::env::current_exe().expect("own executable path");
    // The client holds the spawn lock, so whatever is at the path is a dead daemon's socket.
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).unwrap_or_else(|e| {
        eprintln!("bind {socket}: {e}");
        std::process::exit(1);
    });
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .expect("restrict socket to its owner");
    let watch = Watch::start(Path::new(root), Path::new(socket))
        .map_err(|e| eprintln!("file watcher unavailable, polling mtimes instead: {e}"))
        .ok();
    let daemon = Arc::new(Daemon {
        root: root.into(),
        socket: socket.into(),
        build: tesselate_config::build_id(&exe).expect("own executable is readable"),
        snapshots: Mutex::default(),
        last_request: Mutex::new(Instant::now()),
        in_flight: AtomicUsize::new(0),
        watch,
    });
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
            Some(e) => format!(
                " files={} builds={} reindexed={} age={}s",
                e.snap.indexed_files(),
                e.snap.builds(),
                e.snap.reindexed(),
                e.built.elapsed().as_secs()
            ),
            None => " (not indexed yet)".into(),
        };
        format!(
            "running pid={} build={}{index}\n",
            std::process::id(),
            self.build
        )
    }

    /// Whether `snap`, built when the watcher had seen `built_at`, still matches the repo: no
    /// watcher event since, or (with no watcher) no indexed file with a different size or mtime.
    fn is_current(&self, snap: &Snapshot, built_at: Seen, seen: Seen) -> bool {
        match &self.watch {
            // The counters only grow, so a snapshot built at or after `seen` is current: a
            // slower concurrent query must not rebuild over a newer one.
            Some(_) => built_at.0 >= seen.0 && built_at.1 >= seen.1,
            None => !snap.is_stale(),
        }
    }

    /// The snapshot for `include`, brought up to date if the repo changed since it was built.
    fn snapshot(&self, include: &str) -> Arc<Snapshot> {
        // ponytail: every query, cache hit or not, writes and removes one sentinel file to
        // wait out in-flight watcher events; cheap on tmpfs, so no batching yet.
        // Read before anything is built, so an edit landing mid-build is seen next time.
        let seen = self.watch.as_ref().map_or((0, 0), Watch::sync);
        let cached = self
            .snapshots
            .lock()
            .unwrap()
            .get(include)
            .map(|e| (e.snap.clone(), e.seen));
        if let Some((snap, built_at)) = &cached {
            if self.is_current(snap, *built_at, seen) {
                return snap.clone();
            }
        }
        // Updates are serialized: concurrent queries on a stale index wait for one update.
        let mut snaps = self.snapshots.lock().unwrap();
        let prev = snaps.get(include).map(|e| (e.snap.clone(), e.seen));
        if let Some((snap, built_at)) = &prev {
            if self.is_current(snap, *built_at, seen) {
                return snap.clone();
            }
        }
        let opts = WalkOptions::from_config_value(include);
        let s = Arc::new(match prev {
            // Lost events: nothing says which files changed, so start over.
            Some((p, built_at)) if seen.1 <= built_at.1 => p.update(&self.root, &opts),
            Some((p, _)) => p.rebuild(&self.root, &opts),
            None => Snapshot::build(&self.root, &opts),
        });
        if snaps.len() >= MAX_SNAPSHOTS && !snaps.contains_key(include) {
            let oldest = snaps
                .iter()
                .min_by_key(|(_, e)| e.built)
                .map(|(k, _)| k.clone());
            snaps.remove(&oldest.unwrap());
        }
        snaps.insert(
            include.to_string(),
            Entry {
                snap: s.clone(),
                built: Instant::now(),
                seen,
            },
        );
        s
    }
}

/// The exact stdout of the `tesselate` subcommand `cmd`.
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
            serde_json::to_string_pretty(&tesselate_core::tools::catalog()).unwrap()
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
