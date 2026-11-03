//! Devices through adb: list, resolve the Current Device, install, launch, input, screenshots.

use crate::state::{self, Config};
use crate::{R, sdk, supervisor};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Device {
    /// adb serial, e.g. emulator-5554.
    pub serial: String,
    pub avd: String,
}

/// Run `cmd`, killing it after `secs`; returns stdout, or stderr as the error.
pub fn run_timeout(cmd: &mut Command, secs: u64) -> R<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let err_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{:?} timed out after {secs}s", cmd.get_program()).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (stdout, stderr) = (reader.join().unwrap_or_default(), err_reader.join().unwrap_or_default());
    if status.success() {
        Ok(stdout)
    } else {
        let msg = String::from_utf8_lossy(if stderr.is_empty() { &stdout } else { &stderr }).trim().to_string();
        Err(if msg.is_empty() { format!("{:?} failed ({status})", cmd.get_program()) } else { msg }.into())
    }
}

/// `adb [-s serial] args…` with a timeout; stdout as text.
pub fn adb(serial: Option<&str>, args: &[&str], secs: u64) -> R<String> {
    sdk::need(&sdk::adb())?;
    let mut c = Command::new(sdk::adb());
    if let Some(s) = serial {
        c.args(["-s", s]);
    }
    Ok(String::from_utf8_lossy(&run_timeout(c.args(args), secs)?).into_owned())
}

impl Device {
    pub fn shell(&self, args: &[&str]) -> R<String> {
        let mut a = vec!["shell"];
        a.extend_from_slice(args);
        adb(Some(&self.serial), &a, 60)
    }
}

/// Serials of emulators adb sees as online.
fn online() -> R<Vec<String>> {
    Ok(adb(None, &["devices"], 15)?
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once('\t'))
        .filter(|(s, st)| s.starts_with("emulator-") && *st == "device")
        .map(|(s, _)| s.to_string())
        .collect())
}

/// Running Devices: an online serial whose Supervisor knows its AVD.
pub fn running() -> R<Vec<Device>> {
    let online = online()?;
    Ok(supervisor::all().into_iter().filter(|s| online.contains(&s.serial)).map(|s| Device { serial: s.serial, avd: s.avd }).collect())
}

/// The Device a command acts on: `--device` (serial or AVD), the Current Device,
/// the only running one, or a fresh boot of the default AVD.
pub fn current(cfg: &Config, flag: Option<&str>) -> R<Device> {
    let all = running()?;
    if let Some(f) = flag {
        return all
            .into_iter()
            .find(|d| d.serial == f || d.avd == f)
            .ok_or_else(|| format!("no running device {f:?}; see `herdr-android devices`").into());
    }
    if let Some(cur) = state::load().device
        && let Some(d) = all.iter().find(|d| d.serial == cur)
    {
        return Ok(d.clone());
    }
    match all.len() {
        0 => {
            let avd = default_avd(cfg);
            eprintln!("no device running; booting {avd} (about 30s)…");
            boot(&avd)
        }
        1 => {
            let d = all.into_iter().next().unwrap();
            state::update(|s| s.device = Some(d.serial.clone()))?;
            Ok(d)
        }
        _ => Err(format!(
            "several devices are running ({}); pass --device",
            all.iter().map(|d| format!("{} {}", d.serial, d.avd)).collect::<Vec<_>>().join(", ")
        )
        .into()),
    }
}

pub fn default_avd(cfg: &Config) -> String {
    cfg.default_device.clone().unwrap_or_else(|| sdk::DEFAULT_AVD.into())
}

/// Boot `avd` (or reuse it if running) and make it the Current Device.
pub fn boot(avd: &str) -> R<Device> {
    sdk::need(&sdk::emulator())?;
    let d = match running()?.into_iter().find(|d| d.avd == avd) {
        Some(d) => d,
        None => {
            if !sdk::avds().iter().any(|a| a == avd) {
                return Err(format!("no AVD named {avd:?}; `herdr-android devices` lists them, `install-sdk` creates {}", sdk::DEFAULT_AVD).into());
            }
            supervisor::spawn(avd)?;
            wait_booted(avd, Duration::from_secs(300))?
        }
    };
    state::update(|s| s.device = Some(d.serial.clone()))?;
    Ok(d)
}

/// Wait for `avd`'s Supervisor to report a serial and Android to finish booting.
fn wait_booted(avd: &str, limit: Duration) -> R<Device> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(s) = supervisor::get(avd) {
            if !supervisor::alive(&s) {
                return Err(format!("the emulator for {avd} exited; see {}", supervisor::emulator_log(avd).display()).into());
            }
            let d = Device { serial: s.serial, avd: avd.into() };
            if d.shell(&["getprop", "sys.boot_completed"]).is_ok_and(|v| v.trim() == "1") {
                return Ok(d);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{avd} did not finish booting in {}s", limit.as_secs()).into())
}

/// Stop the emulator and wait for it to exit, so a `boot` straight after starts a fresh one.
pub fn shutdown(d: &Device) -> R<()> {
    adb(Some(&d.serial), &["emu", "kill"], 30)?;
    let start = Instant::now();
    while supervisor::get(&d.avd).is_some_and(|s| supervisor::alive(&s)) {
        if start.elapsed() > Duration::from_secs(60) {
            return Err(format!("{} did not exit within 60s", d.avd).into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

pub fn install(d: &Device, apk: &Path) -> R<()> {
    let apk = apk.to_str().ok_or("APK path is not UTF-8")?;
    adb(Some(&d.serial), &["install", "-r", "-t", apk], 300)?;
    state::event(&d.avd, "install", apk)?;
    Ok(())
}

/// Package name of an APK, from aapt2.
pub fn package_of(apk: &Path) -> R<String> {
    let out = run_timeout(Command::new(sdk::aapt2()?).args(["dump", "packagename"]).arg(apk), 60)?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// (Re)launch `pkg`'s launcher activity; it becomes the Current app. Returns its pid.
pub fn launch(d: &Device, pkg: &str) -> R<String> {
    let activity = d.shell(&["cmd", "package", "resolve-activity", "--brief", "-c", "android.intent.category.LAUNCHER", pkg])?;
    let activity = activity
        .lines()
        .last()
        .map(str::trim)
        .filter(|a| a.contains('/'))
        .ok_or_else(|| format!("{pkg} is not installed or has no launcher activity"))?;
    d.shell(&["am", "force-stop", pkg])?;
    let out = d.shell(&["am", "start", "-W", "-n", activity])?;
    if out.contains("Error") {
        return Err(out.trim().into());
    }
    state::update(|s| s.app = Some(pkg.into()))?;
    let pid = d.shell(&["pidof", pkg]).unwrap_or_default().trim().to_string();
    state::event(&d.avd, "launch", &format!("{pkg} (pid {pid})"))?;
    Ok(pid)
}

pub fn terminate(d: &Device, pkg: &str) -> R<()> {
    d.shell(&["am", "force-stop", pkg])?;
    state::event(&d.avd, "terminate", pkg)
}

pub fn deeplink(d: &Device, url: &str) -> R<()> {
    // `am` re-parses its arguments through the device shell, so quote the URL for it.
    let quoted = format!("'{}'", url.replace('\'', r"'\''"));
    let out = d.shell(&["am", "start", "-W", "-a", "android.intent.action.VIEW", "-d", &quoted])?;
    if out.contains("Error") {
        return Err(out.trim().into());
    }
    state::event(&d.avd, "deep link", url)
}

pub fn screenshot(d: &Device, path: &Path) -> R<()> {
    sdk::need(&sdk::adb())?;
    let png = run_timeout(Command::new(sdk::adb()).args(["-s", &d.serial, "exec-out", "screencap", "-p"]), 30)?;
    if !png.starts_with(b"\x89PNG") {
        return Err("screencap did not return a PNG".into());
    }
    std::fs::write(path, png)?;
    Ok(())
}

/// Screen size in pixels (honours `wm size` overrides).
pub fn screen_size(d: &Device) -> R<(f64, f64)> {
    let out = d.shell(&["wm", "size"])?;
    let line = out.lines().rfind(|l| l.contains("size:")).ok_or("cannot read the screen size")?;
    let (w, h) = line.rsplit(' ').next().and_then(|s| s.split_once('x')).ok_or("cannot read the screen size")?;
    Ok((w.trim().parse()?, h.trim().parse()?))
}

pub fn tap(d: &Device, x: f64, y: f64) -> R<()> {
    d.shell(&["input", "tap", &format!("{x:.0}"), &format!("{y:.0}")]).map(drop)
}

pub fn swipe(d: &Device, from: (f64, f64), to: (f64, f64), ms: u32) -> R<()> {
    let p = |v: f64| format!("{v:.0}");
    d.shell(&["input", "swipe", &p(from.0), &p(from.1), &p(to.0), &p(to.1), &ms.to_string()]).map(drop)
}

/// `input text` takes one shell word with spaces as %s; escape what the device shell would eat.
pub fn input_text_arg(text: &str) -> String {
    let mut s = String::new();
    for c in text.chars() {
        match c {
            ' ' => s.push_str("%s"),
            '%' => s.push_str(r"\%"),
            c if "\\'\"`$&|;<>()[]{}*?!~#".contains(c) => {
                s.push('\\');
                s.push(c);
            }
            c => s.push(c),
        }
    }
    s
}

pub fn type_text(d: &Device, text: &str) -> R<()> {
    if !text.is_ascii() {
        return Err("adb can only type ASCII; type other text in the view".into());
    }
    d.shell(&["input", "text", &input_text_arg(text)]).map(drop)
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Button {
    Back,
    Home,
    Recents,
    Power,
    Enter,
    VolumeUp,
    VolumeDown,
}

impl Button {
    pub fn keycode(self) -> &'static str {
        match self {
            Button::Back => "KEYCODE_BACK",
            Button::Home => "KEYCODE_HOME",
            Button::Recents => "KEYCODE_APP_SWITCH",
            Button::Power => "KEYCODE_POWER",
            Button::Enter => "KEYCODE_ENTER",
            Button::VolumeUp => "KEYCODE_VOLUME_UP",
            Button::VolumeDown => "KEYCODE_VOLUME_DOWN",
        }
    }
}

pub fn button(d: &Device, b: Button) -> R<()> {
    d.shell(&["input", "keyevent", b.keycode()]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_survives_the_device_shell() {
        assert_eq!(input_text_arg("hi there"), "hi%sthere");
        assert_eq!(input_text_arg("a&b's"), r"a\&b\'s");
        assert_eq!(input_text_arg("100%"), r"100\%");
    }

    #[test]
    fn timeouts_kill_the_command() {
        let err = run_timeout(Command::new("sleep").arg("5"), 1).unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn failures_report_stderr() {
        let err = run_timeout(Command::new("sh").args(["-c", "echo nope >&2; exit 3"]), 5).unwrap_err();
        assert_eq!(err.to_string(), "nope");
    }
}
