//! Where the Android SDK lives, the JDK its tools need, and `install-sdk`.

use crate::R;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The AVD `install-sdk` creates and `boot` falls back to.
pub const DEFAULT_AVD: &str = "herdr-default";
const API: &str = "36";
const BUILD_TOOLS: &str = "36.0.0";
const REPO: &str = "https://dl.google.com/android/repository";
/// cmdline-tools 23.0 (sdkmanager updates itself from here), with sha1 from repository2-3.xml.
const CMDLINE_TOOLS: [(&str, &str, &str); 2] = [
    ("macos", "commandlinetools-mac-15641748_latest.zip", "b62a5d8cf63ded173b47be867be4ee058ceda6df"),
    ("linux", "commandlinetools-linux-15641748_latest.zip", "63523a02a975a81102238566f2a16c057d52301e"),
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is not set"))
}

/// `ANDROID_HOME`, then `ANDROID_SDK_ROOT`, then the place Android Studio uses.
pub fn root() -> PathBuf {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| std::env::var_os(v).filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .next()
        .unwrap_or_else(|| if cfg!(target_os = "macos") { home().join("Library/Android/sdk") } else { home().join("Android/Sdk") })
}

pub fn adb() -> PathBuf {
    root().join("platform-tools/adb")
}

pub fn emulator() -> PathBuf {
    root().join("emulator/emulator")
}

fn sdkmanager() -> PathBuf {
    root().join("cmdline-tools/latest/bin/sdkmanager")
}

fn avdmanager() -> PathBuf {
    root().join("cmdline-tools/latest/bin/avdmanager")
}

/// The newest `build-tools/*/aapt2`, for reading an APK's package name.
pub fn aapt2() -> R<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root().join("build-tools")).map_err(|_| no_sdk())?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().rev().map(|d| d.join("aapt2")).find(|p| p.exists()).ok_or_else(no_sdk)
}

pub fn no_sdk() -> Box<dyn std::error::Error> {
    format!("no Android SDK at {}; run `herdr-android install-sdk` or set ANDROID_HOME", root().display()).into()
}

/// Fail early with the fix when a tool from the SDK is missing.
pub fn need(tool: &Path) -> R<()> {
    if tool.exists() { Ok(()) } else { Err(no_sdk()) }
}

/// A JDK 17+ home: `JAVA_HOME`, Android Studio's own JDK, macOS's java_home, Homebrew's openjdk,
/// or `java` on PATH.
pub fn java_home() -> R<PathBuf> {
    let mut found: Vec<PathBuf> = std::env::var_os("JAVA_HOME").filter(|s| !s.is_empty()).map(PathBuf::from).into_iter().collect();
    found.extend([
        PathBuf::from("/Applications/Android Studio.app/Contents/jbr/Contents/Home"),
        home().join("Applications/Android Studio.app/Contents/jbr/Contents/Home"),
        PathBuf::from("/opt/android-studio/jbr"),
        home().join("android-studio/jbr"),
    ]);
    if let Ok(o) = Command::new("/usr/libexec/java_home").args(["-v", "17+"]).stderr(Stdio::null()).output() {
        found.extend(o.status.success().then(|| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())));
    }
    for brew in ["/opt/homebrew/opt", "/usr/local/opt", "/home/linuxbrew/.linuxbrew/opt"] {
        for name in ["openjdk@21", "openjdk@17", "openjdk"] {
            found.push(Path::new(brew).join(name));
        }
    }
    if let Ok(o) = Command::new("sh").args(["-c", "command -v java"]).output() {
        let java = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        if let Some(home) = std::fs::canonicalize(&java).ok().and_then(|p| p.parent()?.parent().map(Path::to_path_buf)) {
            found.push(home);
        }
    }
    found
        .into_iter()
        .find(|h| java_major(h).is_some_and(|v| v >= 17))
        .ok_or_else(|| "the Android SDK tools need JDK 17 or later: `brew install openjdk@17` (macOS) or `sudo apt install openjdk-17-jdk` (Linux), then run this again".into())
}

fn java_major(home: &Path) -> Option<u32> {
    let out = Command::new(home.join("bin/java")).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let v = text.split('"').nth(1)?;
    let mut parts = v.split('.');
    let first: u32 = parts.next()?.parse().ok()?;
    // Java 8 and older report "1.8.0".
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// The system image for this machine's CPU.
fn image() -> String {
    let abi = if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" };
    format!("system-images;android-{API};google_apis;{abi}")
}

fn run(cmd: &mut Command, what: &str) -> R<()> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status})").into()) }
}

fn sha1_of(path: &Path) -> R<String> {
    let mut h = Sha1::new();
    h.update(std::fs::read(path)?);
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download and unpack cmdline-tools into `cmdline-tools/latest`.
fn install_cmdline_tools() -> R<()> {
    let os = if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let (_, file, sha) = CMDLINE_TOOLS.iter().find(|(o, ..)| *o == os).ok_or("unsupported OS")?;
    let tmp = std::env::temp_dir().join(format!("herdr-android-sdk-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let zip = tmp.join(file);
    eprintln!("downloading {file}…");
    run(
        Command::new("curl")
            .args(["-fL", "--retry", "3", "--connect-timeout", "30", "--max-time", "1800", "-o"])
            .arg(&zip)
            .arg(format!("{REPO}/{file}")),
        "curl",
    )?;
    if sha1_of(&zip)? != *sha {
        return Err(format!("{file} does not match its published checksum; not installing it").into());
    }
    run(Command::new("unzip").args(["-q", "-o"]).arg(&zip).arg("-d").arg(&tmp), "unzip")?;
    let dest = root().join("cmdline-tools/latest");
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(tmp.join("cmdline-tools"), &dest)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// Where AVDs live: `ANDROID_AVD_HOME`, else `~/.android/avd`.
pub fn avd_dir() -> PathBuf {
    std::env::var_os("ANDROID_AVD_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".android/avd"))
}

/// When `avd` last ran: the emulator rewrites files in its `.avd` folder on every boot.
pub fn last_used(avd: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(avd_dir().join(format!("{avd}.avd"))).and_then(|m| m.modified()).ok()
}

pub fn avds() -> Vec<String> {
    let Ok(out) = Command::new(emulator()).arg("-list-avds").stderr(Stdio::null()).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("INFO")).map(String::from).collect()
}

/// The emulator drops keyboard input (the View's typing and system keys) unless the AVD
/// has a hardware keyboard. Returns the config with `hw.keyboard=yes`, or None if it has one.
fn with_keyboard(config: &str) -> Option<String> {
    if config.lines().any(|l| l.trim() == "hw.keyboard=yes") {
        return None;
    }
    let mut lines: Vec<&str> = config.lines().filter(|l| !l.trim_start().starts_with("hw.keyboard=")).collect();
    lines.push("hw.keyboard=yes");
    Some(lines.join("\n") + "\n")
}

/// Give `avd` a hardware keyboard so the View can type into it.
pub fn enable_keyboard(avd: &str) -> R<()> {
    let path = avd_dir().join(format!("{avd}.avd/config.ini"));
    let Ok(config) = std::fs::read_to_string(&path) else { return Ok(()) };
    if let Some(new) = with_keyboard(&config) {
        std::fs::write(&path, new)?;
    }
    Ok(())
}

/// Install everything a Device needs and create the Default AVD; safe to run again.
pub fn install() -> R<String> {
    // An existing SDK (ours or Android Studio's) with an AVD is enough; don't touch it.
    if adb().exists() && emulator().exists() && aapt2().is_ok() && !avds().is_empty() {
        return Ok(format!("Android SDK already at {} with an AVD; nothing to install", root().display()));
    }
    let java = java_home()?;
    let path = format!("{}:{}", java.join("bin").display(), std::env::var("PATH").unwrap_or_default());
    if !sdkmanager().exists() {
        install_cmdline_tools()?;
    }
    let sdk = |args: &[&str]| {
        let mut c = Command::new(sdkmanager());
        c.args(args).arg(format!("--sdk_root={}", root().display())).env("JAVA_HOME", &java).env("PATH", &path);
        c
    };
    eprintln!("accepting the Android SDK licences…");
    let mut lic = sdk(&["--licenses"]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn()?;
    // sdkmanager asks once per unaccepted licence; answer yes to all of them.
    {
        use std::io::Write;
        let mut stdin = lic.stdin.take().unwrap();
        let _ = stdin.write_all("y\n".repeat(50).as_bytes());
    }
    lic.wait()?;
    let image = image();
    let packages = ["platform-tools", "emulator", &format!("platforms;android-{API}"), &format!("build-tools;{BUILD_TOOLS}"), &image];
    eprintln!("installing {} (a few GB the first time)…", packages.join(", "));
    run(sdk(&["--install"]).args(packages), "sdkmanager --install")?;
    if !avds().iter().any(|a| a == DEFAULT_AVD) {
        let mut c = Command::new(avdmanager());
        c.args(["create", "avd", "-n", DEFAULT_AVD, "-k", &image, "-d", "pixel_8"]).env("JAVA_HOME", &java).env("PATH", &path).stdin(Stdio::piped());
        // avdmanager asks whether to make a custom hardware profile.
        let mut child = c.spawn()?;
        {
            use std::io::Write;
            let _ = child.stdin.take().unwrap().write_all(b"no\n");
        }
        if !child.wait()?.success() {
            return Err("avdmanager create avd failed".into());
        }
    }
    Ok(format!("Android SDK ready at {}; AVD {DEFAULT_AVD} ({image})", root().display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboards_are_turned_on_once() {
        assert_eq!(with_keyboard("a=1\nhw.keyboard=no\n").as_deref(), Some("a=1\nhw.keyboard=yes\n"));
        assert_eq!(with_keyboard("a=1").as_deref(), Some("a=1\nhw.keyboard=yes\n"));
        assert_eq!(with_keyboard("hw.keyboard=yes\nhw.keyboard.lid=yes"), None);
    }

    #[test]
    fn image_matches_this_cpu() {
        let img = image();
        assert!(img.starts_with("system-images;android-36;google_apis;"));
        assert!(img.ends_with(if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" }));
    }
}
