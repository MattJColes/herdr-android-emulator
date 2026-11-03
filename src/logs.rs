//! Reading the logcat the Supervisor keeps: log entries for an app, and Crashes and ANRs.

use crate::state;
use serde::Serialize;
use std::collections::HashSet;

/// One logcat line, `-v threadtime -v epoch`: `1700000000.123  1234  1240 E Tag: message`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub t: f64,
    pub boot: u32,
    pub pid: u32,
    pub level: char,
    pub tag: String,
    pub msg: String,
}

pub fn parse(line: &str, boot: u32) -> Option<Entry> {
    let mut it = line.split_whitespace();
    let t = it.next()?.parse().ok()?;
    let pid = it.next()?.parse().ok()?;
    let _tid: u32 = it.next()?.parse().ok()?;
    let level = it.next()?.chars().next()?;
    // The rest is `Tag: message`; the first " L " is the level, as the fields before it are numbers.
    let after_level = line.find(&format!(" {level} "))? + 3;
    let (tag, msg) = line[after_level..].split_once(':')?;
    Some(Entry { t, boot, pid, level, tag: tag.trim().into(), msg: msg.strip_prefix(' ').unwrap_or(msg).into() })
}

/// Every entry of one boot, oldest first.
pub fn read_boot(avd: &str, boot: u32) -> Vec<Entry> {
    let text = std::fs::read_to_string(state::boot_dir(avd, boot).join("logcat.txt")).unwrap_or_default();
    text.lines().filter_map(|l| parse(l, boot)).collect()
}

/// Boots oldest first, or just the current one.
pub fn read(avd: &str, all_boots: bool) -> Vec<Entry> {
    let mut boots = state::boots(avd);
    boots.sort();
    if !all_boots {
        boots = boots.into_iter().rev().take(1).collect();
    }
    boots.into_iter().flat_map(|b| read_boot(avd, b)).collect()
}

/// PIDs a package has had, from ActivityManager's `Start proc 1234:com.x/u0a190 for …` lines.
pub fn pids_of(entries: &[Entry], pkg: &str) -> HashSet<(u32, u32)> {
    entries
        .iter()
        .filter(|e| e.tag == "ActivityManager")
        .filter_map(|e| {
            let rest = e.msg.strip_prefix("Start proc ")?;
            let (pid, name) = rest.split_once(':')?;
            let name = name.split('/').next()?;
            (name == pkg || name.starts_with(&format!("{pkg}:"))).then(|| pid.parse().ok().map(|p| (e.boot, p)))?
        })
        .collect()
}

pub struct Query<'a> {
    pub app: Option<&'a str>,
    pub since: f64,
    pub errors: bool,
    /// Only used when `app` is None: include debug/verbose noise from every process.
    pub all_levels: bool,
}

/// Entries matching `q`. `extra_pids` covers an app process that was running before its start line was kept.
pub fn query(entries: &[Entry], q: &Query, extra_pids: &[(u32, u32)]) -> Vec<Entry> {
    let pids = q.app.map(|a| {
        let mut p = pids_of(entries, a);
        p.extend(extra_pids.iter().copied());
        p
    });
    entries
        .iter()
        .filter(|e| e.t >= q.since)
        .filter(|e| match (&pids, q.errors) {
            (_, true) => matches!(e.level, 'E' | 'F'),
            (Some(_), false) => e.level != 'V',
            (None, false) => q.all_levels || !matches!(e.level, 'V' | 'D'),
        })
        .filter(|e| pids.as_ref().is_none_or(|p| p.contains(&(e.boot, e.pid))))
        .cloned()
        .collect()
}

pub fn line(e: &Entry) -> String {
    format!("{} {} {}: {}", state::clock(e.t), e.level, e.tag, e.msg)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Crash {
    pub t: f64,
    pub boot: u32,
    /// "crash" (Java/Kotlin exception), "native crash" (signal) or "ANR".
    pub kind: &'static str,
    pub app: String,
    /// The exception, signal or ANR reason.
    pub what: String,
    pub frames: Vec<String>,
}

const FRAMES: usize = 12;

/// Crashes and ANRs in `entries`, oldest first.
pub fn crashes(entries: &[Entry]) -> Vec<Crash> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let e = &entries[i];
        // Consecutive lines from the same process and tag make one report.
        let group = || entries[i..].iter().take_while(|x| x.pid == e.pid && x.tag == e.tag).collect::<Vec<_>>();
        if e.tag == "AndroidRuntime" && e.msg.starts_with("FATAL EXCEPTION") {
            let g = group();
            let app = g.iter().find_map(|x| x.msg.strip_prefix("Process: ")).and_then(|p| p.split(',').next()).unwrap_or("").to_string();
            let body: Vec<&str> = g.iter().skip(1).map(|x| x.msg.as_str()).filter(|m| !m.starts_with("Process: ")).collect();
            let what = body.first().copied().unwrap_or("").to_string();
            let frames = body
                .iter()
                .skip(1)
                .filter(|m| m.trim_start().starts_with("at ") || m.starts_with("Caused by"))
                .take(FRAMES)
                .map(|m| m.trim().to_string())
                .collect();
            out.push(Crash { t: e.t, boot: e.boot, kind: "crash", app, what, frames });
            i += g.len();
        } else if e.tag == "ActivityManager" && e.msg.starts_with("ANR in ") {
            let g = group();
            let app = e.msg["ANR in ".len()..].split_whitespace().next().unwrap_or("").to_string();
            let what = g.iter().find_map(|x| x.msg.strip_prefix("Reason: ")).unwrap_or("not responding").to_string();
            out.push(Crash { t: e.t, boot: e.boot, kind: "ANR", app, what, frames: Vec::new() });
            i += g.len();
        } else if e.tag == "DEBUG" && e.msg.contains("*** *** ***") {
            let g = group();
            let app = g.iter().find_map(|x| x.msg.split(">>> ").nth(1)?.split(" <<<").next()).unwrap_or("").to_string();
            let what = g.iter().find(|x| x.msg.starts_with("signal ")).map(|x| x.msg.clone()).unwrap_or_default();
            let frames = g.iter().map(|x| x.msg.trim()).filter(|m| m.starts_with('#')).take(FRAMES).map(String::from).collect();
            out.push(Crash { t: e.t, boot: e.boot, kind: "native crash", app, what, frames });
            i += g.len();
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
1700000000.100  500  520 I ActivityManager: Start proc 4242:com.example.demo/u0a190 for top-activity {com.example.demo/com.example.demo.MainActivity}
1700000000.200 4242 4242 D Demo    : signing in
1700000000.300 4242 4242 E AndroidRuntime: FATAL EXCEPTION: main
1700000000.300 4242 4242 E AndroidRuntime: Process: com.example.demo, PID: 4242
1700000000.300 4242 4242 E AndroidRuntime: java.lang.IllegalStateException: sign in failed
1700000000.300 4242 4242 E AndroidRuntime: \tat com.example.demo.MainActivity.signIn(MainActivity.kt:31)
1700000000.300 4242 4242 E AndroidRuntime: \tat android.view.View.performClick(View.java:8000)
1700000000.400  999  999 I Other   : unrelated
1700000001.000  500  530 E ActivityManager: ANR in com.example.demo (com.example.demo/.MainActivity)
1700000001.000  500  530 E ActivityManager: PID: 4243
1700000001.000  500  530 E ActivityManager: Reason: Input dispatching timed out
1700000002.000 4300 4300 F DEBUG   : *** *** *** *** *** *** *** *** *** *** *** *** *** *** *** ***
1700000002.000 4300 4300 F DEBUG   : pid: 4250, tid: 4250, name: le.demo  >>> com.example.demo <<<
1700000002.000 4300 4300 F DEBUG   : signal 11 (SIGSEGV), code 1 (SEGV_MAPERR), fault addr 0x0
1700000002.000 4300 4300 F DEBUG   :       #00 pc 0000000000012345  /data/app/lib/arm64/libdemo.so (crash+12)
";

    fn entries() -> Vec<Entry> {
        LOG.lines().filter_map(|l| parse(l, 1)).collect()
    }

    #[test]
    fn lines_parse_with_padded_tags() {
        let e = parse("1700000000.200 4242 4242 D Demo    : signing in: now", 3).unwrap();
        assert_eq!((e.pid, e.level, e.tag.as_str(), e.msg.as_str(), e.boot), (4242, 'D', "Demo", "signing in: now", 3));
    }

    #[test]
    fn app_logs_follow_its_pids() {
        let q = Query { app: Some("com.example.demo"), since: 0.0, errors: false, all_levels: false };
        let got = query(&entries(), &q, &[]);
        assert!(got.iter().all(|e| e.pid == 4242));
        assert_eq!(got.len(), 6);
    }

    #[test]
    fn crashes_anrs_and_native_crashes_are_found() {
        let c = crashes(&entries());
        assert_eq!(c.iter().map(|c| c.kind).collect::<Vec<_>>(), ["crash", "ANR", "native crash"]);
        assert_eq!(c[0].app, "com.example.demo");
        assert_eq!(c[0].what, "java.lang.IllegalStateException: sign in failed");
        assert_eq!(c[0].frames[0], "at com.example.demo.MainActivity.signIn(MainActivity.kt:31)");
        assert_eq!(c[1].what, "Input dispatching timed out");
        assert_eq!(c[2].app, "com.example.demo");
        assert!(c[2].what.starts_with("signal 11"));
        assert_eq!(c[2].frames.len(), 1);
    }
}
