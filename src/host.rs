//! The two herdr front ends, behind one interface (from herdr-web-browser):
//! - `Tui`: the `herdr` terminal server (plugins, `herdr agent prompt`).
//! - `Desktop`: Herdr.app on herdr-core (`herdr-app`, `hc send`, programs.json).

use crate::R;
use crate::adb::run_timeout;
use crate::state;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Tui,
    Desktop,
}

fn env(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

/// Run a CLI with a deadline; stdout parsed as JSON (Null when it isn't).
pub fn run(bin: &str, args: &[&str]) -> R<Value> {
    let out = run_timeout(Command::new(bin).args(args), 15).map_err(|e| format!("{bin} {}: {e}", args.first().unwrap_or(&"")))?;
    Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
}

fn herdr(args: &[&str]) -> R<Value> {
    run(&env("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()), args)
}

/// The caller's host from the env each one injects, else the desktop if its window answers.
pub fn host() -> Host {
    if env("HERDR_PANE_ID").is_some() || env("HERDR_ENV").is_some() {
        Host::Tui
    } else if env("HC_TERMINAL_ID").is_some() || run("herdr-app", &["status"]).is_ok() {
        Host::Desktop
    } else {
        Host::Tui
    }
}

/// The agent a view opened from here sits beside: the calling agent, else the focused one.
pub fn owner(host: Host) -> Option<String> {
    match host {
        Host::Desktop => env("HC_TERMINAL_ID").or_else(|| run("herdr-app", &["status"]).ok()?["agent"].as_str().map(String::from)),
        Host::Tui => env("HERDR_PANE_ID"),
    }
}

fn exe() -> R<String> {
    Ok(std::fs::canonicalize(std::env::current_exe()?)?.display().to_string())
}

/// Open the view in a split beside `owner` (or the focused pane).
pub fn open_view(host: Host, owner: Option<&str>) -> R<String> {
    match host {
        Host::Tui => {
            herdr(&["plugin", "pane", "open", "--plugin", state::PLUGIN_ID, "--entrypoint", "android"])?;
            Ok("showing the device in a herdr view".into())
        }
        Host::Desktop => {
            let mut argv = vec![exe()?];
            if let Some(o) = owner {
                argv.extend(["--owner".into(), o.to_string()]);
            }
            argv.push("pane".into());
            let mut args = vec!["do".to_string(), "pane.split".into(), "--direction".into(), "right".into()];
            if let Some(p) = owner.and_then(|o| pane_of(&run("herdr-app", &["ls"]).ok()?, o)) {
                args.extend(["--pane".into(), p]);
            }
            args.extend(["--argv".into(), serde_json::to_string(&argv)?]);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let v = run("herdr-app", &refs)?;
            Ok(format!("showing the device in a herdr view (terminal {})", v["terminal"].as_str().unwrap_or("?")))
        }
    }
}

/// The desktop pane and tab that hold `terminal`, from `herdr-app ls`.
fn place(ls: &Value, terminal: &str) -> Option<(String, String)> {
    ls["spaces"].as_array()?.iter().flat_map(|sp| sp["tabs"].as_array().into_iter().flatten()).find_map(|tab| {
        let t = tab["terminals"].as_array()?.iter().find(|t| t["terminal"] == terminal)?;
        let pane = t["pane"].as_str().filter(|p| !p.is_empty())?;
        Some((tab["id"].as_str().unwrap_or_default().to_string(), pane.to_string()))
    })
}

fn pane_of(ls: &Value, terminal: &str) -> Option<String> {
    place(ls, terminal).map(|(_, p)| p)
}

/// herdr desktop's + menu starts programs in a new tab: move the view into a split beside
/// the last focused agent and close that tab. Returns false (stay put) with no agent to sit by.
/// ponytail: "last focused agent" stands in for "the tab you were on"; herdr reports no tab history.
pub fn split_beside_agent() -> R<bool> {
    let me = env("HC_TERMINAL_ID").ok_or("not in a herdr desktop terminal")?;
    let status = run("herdr-app", &["status"])?;
    let Some(agent) = status["agent"].as_str().filter(|a| *a != me) else { return Ok(false) };
    let ls = run("herdr-app", &["ls"])?;
    let (Some(pane), Some((my_tab, _))) = (pane_of(&ls, agent), place(&ls, &me)) else { return Ok(false) };
    let argv = serde_json::to_string(&[exe()?.as_str(), "--owner", agent, "pane"])?;
    let made = run("herdr-app", &["do", "pane.split", "--direction", "right", "--pane", &pane, "--argv", &argv])?;
    if let Some(t) = made["terminal"].as_str() {
        drop(run("herdr-app", &["do", "pane.focus", "--terminal", t]));
    }
    // Last: closing this tab ends this process.
    run("herdr-app", &["do", "tab.close", "--id", &my_tab, "--confirmed", "true"])?;
    Ok(true)
}

/// Type `text` into `agent`'s prompt.
pub fn prompt(host: Host, agent: &str, text: &str) -> R<()> {
    match host {
        Host::Tui => herdr(&["agent", "prompt", agent, text]).map(drop),
        Host::Desktop => run("hc", &["send", agent, text]).map(drop),
    }
}

fn app_home() -> PathBuf {
    std::env::var_os("HERDR_APP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/herdr-app"))
}

/// Add **Android** to herdr desktop's + menu (programs.json), if herdr desktop is installed.
pub fn install_desktop() -> R<Option<String>> {
    let home = app_home();
    if !home.is_dir() {
        return Ok(None);
    }
    let icon_dir = home.join("icons/herdr-android");
    std::fs::create_dir_all(&icon_dir)?;
    std::fs::write(icon_dir.join("android.svg"), ICON)?;
    let path = home.join("programs.json");
    let mut doc: Value = match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}; not changing it", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    let programs = doc.as_object_mut().ok_or("programs.json is not an object")?.entry("programs").or_insert_with(|| json!({}));
    programs.as_object_mut().ok_or("programs.json `programs` is not an object")?.insert(
        "herdr-android".into(),
        json!({ "name": "Android", "icon": "icons/herdr-android/android.svg", "tint": true, "tile": "#3ddc84", "color": "#073042", "run": [exe()?, "menu-open"] }),
    );
    std::fs::write(&path, serde_json::to_vec_pretty(&doc)?)?;
    Ok(Some(format!("herdr desktop: Android in the + menu ({})", path.display())))
}

/// Take **Android** back out of programs.json.
pub fn uninstall_desktop() -> R<Option<String>> {
    let path = app_home().join("programs.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}; not changing it", path.display()))?;
    let removed = doc.pointer_mut("/programs").and_then(Value::as_object_mut).and_then(|p| p.remove("herdr-android")).is_some();
    let _ = std::fs::remove_dir_all(app_home().join("icons/herdr-android"));
    if !removed {
        return Ok(None);
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&doc)?)?;
    Ok(Some(format!("removed Android from {}", path.display())))
}

/// A one-colour phone glyph for the + menu tile.
const ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" d="M8 2h8a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zm0 3v13h8V5H8zm3 14.5a1 1 0 1 0 2 0 1 1 0 0 0-2 0z"/></svg>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_found_in_their_tab() {
        let ls = json!({ "spaces": [ { "tabs": [
            { "id": "tb1", "terminals": [ { "terminal": "t3", "pane": "p9" } ] },
            { "id": "tb2", "terminals": [ { "terminal": "t4", "pane": "" } ] } ] } ] });
        assert_eq!(place(&ls, "t3"), Some(("tb1".into(), "p9".into())));
        assert_eq!(place(&ls, "t4"), None);
        assert_eq!(place(&ls, "t5"), None);
    }
}
