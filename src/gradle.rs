//! `build`: the project's Gradle wrapper, reduced to compiler errors and warnings.

use crate::adb::run_timeout;
use crate::{R, sdk};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The directory with `gradlew`: here, or `android/` for Flutter and React Native.
pub fn project(start: &Path) -> R<PathBuf> {
    [start.to_path_buf(), start.join("android")]
        .into_iter()
        .find(|d| d.join("gradlew").exists())
        .ok_or_else(|| format!("no gradlew in {} or its android/ folder", start.display()).into())
}

/// Compiler errors and warnings (Kotlin `e:`/`w:`, javac, AAPT, Gradle's own failure).
pub fn issues(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = output.lines().peekable();
    while let Some(l) = lines.next() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("e: ").or_else(|| t.strip_prefix("w: ")) {
            let rest = rest.strip_prefix("file://").unwrap_or(rest);
            out.push(format!("{}: {rest}", if t.starts_with('e') { "error" } else { "warning" }));
        } else if t.contains(": error:") || t.contains(": warning:") || t.starts_with("ERROR:") || t.contains("AAPT: error") {
            out.push(t.to_string());
        } else if t == "* What went wrong:" {
            // Gradle's own reason, e.g. a missing SDK or a failed dependency.
            let reason: Vec<&str> = lines.by_ref().map(str::trim).take_while(|l| !l.is_empty() && !l.starts_with("* Try:")).collect();
            if !reason.is_empty() && !out.iter().any(|i| i.starts_with("error")) {
                out.push(format!("gradle: {}", reason.join(" ")));
            }
        }
    }
    out
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub struct Built {
    pub ok: bool,
    pub issues: Vec<String>,
    pub apk: Option<PathBuf>,
}

/// Run `gradlew :module:assembleVariant` in `dir`.
pub fn build(dir: &Path, module: &str, variant: &str) -> R<Built> {
    let java = sdk::java_home()?;
    let task = format!(":{module}:assemble{}", cap(variant));
    let mut c = Command::new(dir.join("gradlew"));
    c.args([&task, "--console=plain"]).current_dir(dir).env("JAVA_HOME", &java).env("ANDROID_HOME", sdk::root());
    // A failed build exits non-zero; its output is what we want either way.
    let (ok, text) = match run_timeout(&mut c, 1800) {
        Ok(out) => (true, String::from_utf8_lossy(&out).into_owned()),
        Err(e) => (false, e.to_string()),
    };
    let apk = ok.then(|| newest_apk(&dir.join(module).join("build/outputs/apk").join(variant))).flatten();
    let mut issues = issues(&text);
    if !ok && issues.is_empty() {
        issues.push(format!("gradle failed: {}", text.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    Ok(Built { ok, issues, apk })
}

fn newest_apk(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "apk"))
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kotlin_and_java_errors_are_kept() {
        let out = "\
> Task :app:compileDebugKotlin FAILED
e: file:///src/app/MainActivity.kt:12:5 Unresolved reference 'foo'.
w: file:///src/app/MainActivity.kt:3:1 Parameter 'x' is never used
/src/app/Util.java:7: error: ';' expected
BUILD FAILED in 3s
* What went wrong:
Execution failed for task ':app:compileDebugKotlin'.

* Try:";
        assert_eq!(
            issues(out),
            [
                "error: /src/app/MainActivity.kt:12:5 Unresolved reference 'foo'.",
                "warning: /src/app/MainActivity.kt:3:1 Parameter 'x' is never used",
                "/src/app/Util.java:7: error: ';' expected",
            ]
        );
    }

    #[test]
    fn gradle_failures_without_compiler_errors_say_why() {
        let out = "* What went wrong:\nSDK location not found.\n\n* Try:";
        assert_eq!(issues(out), ["gradle: SDK location not found."]);
    }

    #[test]
    fn variants_name_gradle_tasks() {
        assert_eq!(cap("debug"), "Debug");
    }
}
