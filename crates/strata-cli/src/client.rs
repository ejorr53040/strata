//! Thin client for the per-repo daemon (`strata-daemon`): finds or starts the daemon for a
//! repo and relays one request. Never links the index, so it starts in a few milliseconds.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a freshly spawned daemon gets to start listening.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

enum Reply {
    Ok(String),
    Err(String),
    /// The daemon belongs to another build and has exited.
    StaleBuild,
}

/// Where one repo's daemon lives: `$XDG_RUNTIME_DIR/strata/<hash of root>.{sock,lock,log}`.
struct Paths {
    root: PathBuf,
    socket: PathBuf,
    lock: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn of(root: &Path) -> Result<Paths, String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or("XDG_RUNTIME_DIR is not set; strata keeps its per-repo daemon socket there")?;
        let dir = PathBuf::from(runtime).join("strata");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let root = root
            .canonicalize()
            .map_err(|e| format!("{}: {e}", root.display()))?;
        let name = format!("{:016x}", fnv1a(root.as_os_str().as_encoded_bytes()));
        let socket = dir.join(format!("{name}.sock"));
        // sockaddr_un holds 108 bytes, NUL included.
        let len = socket.as_os_str().len();
        if len > 107 {
            return Err(format!(
                "XDG_RUNTIME_DIR is too long for a Unix socket path ({len} bytes, limit 107): {}",
                socket.display()
            ));
        }
        Ok(Paths {
            socket,
            lock: dir.join(format!("{name}.lock")),
            log: dir.join(format!("{name}.log")),
            root,
        })
    }
}

/// Stable across builds (unlike `DefaultHasher`), so every client names a repo's socket alike.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

fn daemon_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.with_file_name("strata-daemon"))
}

/// Run `cmd` against `root`'s daemon, starting (or replacing) the daemon as needed. Returns
/// the text to print.
pub fn query(root: &Path, include: &str, cmd: &str, args: Value) -> Result<String, String> {
    let paths = Paths::of(root)?;
    let exe = daemon_exe()?;
    let build = strata_config::build_id(&exe)?;
    let req = json!({"build": build, "include": include, "cmd": cmd, "args": args});
    // A daemon may exit between our connect and its reply (stale build, idle exit): retry.
    for _ in 0..3 {
        match send(&paths.socket, &req) {
            Some(Reply::Ok(text)) => return Ok(text),
            Some(Reply::Err(e)) => return Err(e),
            Some(Reply::StaleBuild) | None => spawn(&paths, &exe)?,
        }
    }
    Err(format!(
        "the strata daemon did not answer; see {}",
        paths.log.display()
    ))
}

/// `status` / `stop` for an already running daemon; never starts one.
pub fn control(root: &Path, include: &str, cmd: &str) -> Result<String, String> {
    let paths = Paths::of(root)?;
    match send(&paths.socket, &json!({"include": include, "cmd": cmd})) {
        Some(Reply::Ok(text)) => Ok(text),
        Some(Reply::Err(e)) => Err(e),
        Some(Reply::StaleBuild) | None => Ok("not running\n".into()),
    }
}

/// One request/reply; None when no daemon is listening or it hung up without replying.
fn send(socket: &Path, req: &Value) -> Option<Reply> {
    let mut stream = UnixStream::connect(socket).ok()?;
    writeln!(stream, "{req}").ok()?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let reply: Value = serde_json::from_str(&line).ok()?;
    if let Some(text) = reply["ok"].as_str() {
        Some(Reply::Ok(text.to_string()))
    } else if let Some(e) = reply["err"].as_str() {
        Some(Reply::Err(e.to_string()))
    } else if reply["stale_build"] == true {
        Some(Reply::StaleBuild)
    } else {
        None
    }
}

/// Start the daemon unless another client already did. The lock makes concurrent cold
/// queries start exactly one.
fn spawn(paths: &Paths, exe: &Path) -> Result<(), String> {
    let lock =
        std::fs::File::create(&paths.lock).map_err(|e| format!("{}: {e}", paths.lock.display()))?;
    lock.lock()
        .map_err(|e| format!("{}: {e}", paths.lock.display()))?;
    if UnixStream::connect(&paths.socket).is_ok() {
        return Ok(());
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .map_err(|e| format!("{}: {e}", paths.log.display()))?;
    let _ = std::fs::set_permissions(&paths.log, std::fs::Permissions::from_mode(0o600));
    let mut child = std::process::Command::new(exe)
        .arg(&paths.root)
        .arg(&paths.socket)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        // Own process group: Ctrl-C on the client's terminal must not kill the daemon.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
    let start = Instant::now();
    while start.elapsed() < SPAWN_TIMEOUT {
        if UnixStream::connect(&paths.socket).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the strata daemon exited at startup ({status}); see {}",
                paths.log.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!(
        "the strata daemon did not start within {}s; see {}",
        SPAWN_TIMEOUT.as_secs(),
        paths.log.display()
    ))
}
