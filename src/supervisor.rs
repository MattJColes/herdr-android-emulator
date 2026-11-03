//! The Supervisor: a detached process per Device that runs the emulator and keeps its
//! logcat on disk per boot, because logcat's own buffer rolls over (ADR 0001).

use crate::adb::Device;
use crate::{R, sdk, state};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Log space kept per AVD, across boots; the oldest boot goes first.
const KEEP_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Supervisor {
    pub avd: String,
    pub serial: String,
    /// The Supervisor's own pid.
    pub pid: u32,
    pub emulator_pid: u32,
    pub boot: u32,
}

fn record(avd: &str) -> PathBuf {
    state::device_dir(avd).join("supervisor.json")
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: kill(2) with signal 0 only checks that the process exists.
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

pub fn alive(s: &Supervisor) -> bool {
    pid_alive(s.pid) && pid_alive(s.emulator_pid)
}

pub fn get(avd: &str) -> Option<Supervisor> {
    fs::read_to_string(record(avd)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Every live Supervisor.
pub fn all() -> Vec<Supervisor> {
    let Ok(dirs) = fs::read_dir(state::state_dir().join("devices")) else { return Vec::new() };
    dirs.flatten().filter_map(|e| get(&e.file_name().to_string_lossy())).filter(alive).collect()
}

pub fn emulator_log(avd: &str) -> PathBuf {
    state::boot_dir(avd, state::current_boot(avd)).join("emulator.log")
}

/// Start a detached Supervisor for `avd`; it outlives the command that started it.
pub fn spawn(avd: &str) -> R<()> {
    // A dead Supervisor's record would read as "the emulator exited" before the new one writes its own.
    if get(avd).is_some_and(|s| !alive(&s)) {
        let _ = fs::remove_file(record(avd));
    }
    Command::new(std::env::current_exe()?)
        .args(["supervise", avd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}

/// The lowest free emulator console port (even, 5554-5682): its serial is emulator-<port>.
fn free_port() -> R<u16> {
    let taken: Vec<String> = all().into_iter().map(|s| s.serial).collect();
    (5554..=5682)
        .step_by(2)
        .find(|p| {
            !taken.contains(&format!("emulator-{p}"))
                && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()
                && std::net::TcpListener::bind(("127.0.0.1", *p + 1)).is_ok()
                && std::net::TcpListener::bind(("127.0.0.1", grpc_port(*p))).is_ok()
        })
        .ok_or_else(|| "no free emulator port between 5554 and 5682".into())
}

/// Drop whole boots, oldest first, until the AVD's logs fit in KEEP_BYTES.
fn prune(avd: &str, keep: u32) {
    let size =
        |n: u32| fs::read_dir(state::boot_dir(avd, n)).map(|d| d.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0u64);
    let mut boots = state::boots(avd);
    let mut total: u64 = boots.iter().map(|&n| size(n)).sum();
    boots.sort();
    for n in boots {
        if total <= KEEP_BYTES || n == keep {
            break;
        }
        total -= size(n);
        let _ = fs::remove_dir_all(state::boot_dir(avd, n));
    }
}

/// The Supervisor itself: boot the emulator, then copy logcat to disk until it exits.
pub fn run(avd: &str) -> R<()> {
    if let Some(s) = get(avd).filter(alive) {
        return Err(format!("{avd} is already running as {}", s.serial).into());
    }
    let boot = state::boots(avd).into_iter().max().unwrap_or(0) + 1;
    let dir = state::boot_dir(avd, boot);
    fs::create_dir_all(&dir)?;
    prune(avd, boot);
    let port = free_port()?;
    sdk::enable_keyboard(avd)?;
    let log = fs::File::create(dir.join("emulator.log"))?;
    let mut emu = Command::new(sdk::emulator())
        .args([
            "-avd",
            avd,
            "-port",
            &port.to_string(),
            "-grpc",
            &grpc_port(port).to_string(),
            "-grpc-use-token",
            "-no-window",
            "-no-boot-anim",
            "-no-audio",
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let me = Supervisor { avd: avd.into(), serial: format!("emulator-{port}"), pid: std::process::id(), emulator_pid: emu.id(), boot };
    fs::write(record(avd), serde_json::to_string_pretty(&me)?)?;
    let d = Device { serial: me.serial.clone(), avd: avd.into() };
    let mut booted = false;
    let mut since: Option<String> = None;
    loop {
        if emu.try_wait()?.is_some() {
            break;
        }
        if !booted {
            if d.shell(&["getprop", "sys.boot_completed"]).is_ok_and(|v| v.trim() == "1") {
                booted = true;
                state::event(avd, "boot", &format!("{avd} as {}", d.serial))?;
            } else {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }
        since = copy_logcat(&d, &dir, since.as_deref()).or(since);
        std::thread::sleep(Duration::from_secs(1));
    }
    state::event(avd, "shutdown", avd)?;
    let _ = fs::remove_file(record(avd));
    Ok(())
}

/// Append logcat to the boot's file until logcat exits; returns the last timestamp seen,
/// so a restarted logcat (`-T`) doesn't repeat lines.
fn copy_logcat(d: &Device, dir: &std::path::Path, since: Option<&str>) -> Option<String> {
    let mut args = vec!["-s", &d.serial, "logcat", "-v", "threadtime", "-v", "epoch", "-b", "main,system,crash"];
    if let Some(t) = since {
        args.extend(["-T", t]);
    }
    let mut child = Command::new(sdk::adb()).args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut file = fs::OpenOptions::new().create(true).append(true).open(dir.join("logcat.txt")).ok()?;
    let mut last = None;
    let reader = std::io::BufReader::new(child.stdout.take()?);
    for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
        if let Some(t) = line.split_whitespace().next().filter(|t| t.contains('.') && t.chars().all(|c| c.is_ascii_digit() || c == '.')) {
            if since.is_some_and(|s| s == t) && last.is_none() {
                continue;
            }
            last = Some(t.to_string());
        }
        // ponytail: KEEP_BYTES is enforced per boot at startup only; a single very long boot can exceed it.
        let _ = writeln!(file, "{line}");
    }
    let _ = child.wait();
    last
}

/// The gRPC port for a console port (8554 for emulator-5554, the emulator's own default).
/// Passed explicitly: without `-grpc`, emulator 37 doesn't always start its gRPC server.
fn grpc_port(console: u16) -> u16 {
    console + 3000
}

/// The emulator's gRPC port and token, from the discovery file it writes a few seconds after it starts.
pub fn grpc_endpoint(avd: &str) -> R<(u16, String)> {
    let s = get(avd).filter(alive).ok_or_else(|| format!("{avd} is not running"))?;
    let name = format!("pid_{}.ini", s.emulator_pid);
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut dirs = vec![home.join("Library/Caches/TemporaryItems/avd/running")];
    if let Some(x) = std::env::var_os("XDG_RUNTIME_DIR") {
        dirs.push(PathBuf::from(x).join("avd/running"));
    }
    dirs.push(std::env::temp_dir().join(format!("android-{}", std::env::var("USER").unwrap_or_default())).join("avd/running"));
    dirs.push(home.join(".android/avd/running"));
    let ini =
        dirs.iter().map(|d| d.join(&name)).find_map(|p| fs::read_to_string(p).ok()).ok_or("the emulator has not published its gRPC endpoint yet")?;
    let get = |k: &str| ini.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string());
    let port = get("grpc.port=").ok_or("the emulator has no gRPC port; was it started by herdr-android?")?.parse()?;
    let token = get("grpc.token=").ok_or("the emulator's gRPC port has no token")?;
    Ok((port, token))
}
