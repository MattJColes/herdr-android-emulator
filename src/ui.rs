//! On-screen elements from uiautomator, numbered for `tap N`, and found again before a tap.

use crate::adb::Device;
use crate::{R, state};
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    /// Class without its package: `Button`, `TextView`.
    pub class: String,
    pub text: String,
    pub desc: String,
    /// resource-id without its package: `sign_in`.
    pub id: String,
    /// Pixels: left, top, right, bottom.
    pub bounds: [f64; 4],
}

impl Element {
    pub fn center(&self) -> (f64, f64) {
        ((self.bounds[0] + self.bounds[2]) / 2.0, (self.bounds[1] + self.bounds[3]) / 2.0)
    }

    pub fn line(&self, n: usize) -> String {
        let label = if self.text.is_empty() { &self.desc } else { &self.text };
        let mut s = format!("[{n}] {}", self.class);
        if !label.is_empty() {
            s += &format!(" {label:?}");
        }
        if !self.id.is_empty() {
            s += &format!(" #{}", self.id);
        }
        s
    }

    fn same(&self, o: &Element) -> bool {
        self.class == o.class && self.text == o.text && self.desc == o.desc && self.id == o.id
    }
}

fn bounds(s: &str) -> Option<[f64; 4]> {
    // "[0,63][1080,210]"
    let n: Vec<f64> = s.split(['[', ']', ',']).filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
}

/// Elements worth naming: labelled, identified or clickable, and visible.
pub fn parse(xml: &str) -> R<Vec<Element>> {
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match r.read_event()? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == "node" => {
                let get = |k: &str| -> R<String> {
                    Ok(match e.try_get_attribute(k)? {
                        Some(a) => a.normalized_value(quick_xml::XmlVersion::Implicit1_0)?.into_owned(),
                        None => String::new(),
                    })
                };
                let class = get("class")?.rsplit('.').next().unwrap_or("").to_string();
                let text = get("text")?;
                let desc = get("content-desc")?;
                let id = get("resource-id")?.rsplit(":id/").next().unwrap_or("").to_string();
                let clickable = get("clickable")? == "true";
                let Some(b) = bounds(&get("bounds")?) else { continue };
                let visible = b[2] > b[0] && b[3] > b[1];
                if visible && (!text.is_empty() || !desc.is_empty() || (clickable && !id.is_empty())) {
                    out.push(Element { class, text, desc, id, bounds: b });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Dump the screen's hierarchy.
pub fn dump(d: &Device) -> R<Vec<Element>> {
    let out = crate::adb::adb(Some(&d.serial), &["exec-out", "uiautomator", "dump", "/dev/tty"], 30)?;
    let end = out.rfind("</hierarchy>").ok_or("uiautomator returned no hierarchy (is the screen locked or animating?)")?;
    let start = out.find('<').unwrap_or(0);
    parse(&out[start..end + "</hierarchy>".len()])
}

fn snapshot_path(avd: &str) -> std::path::PathBuf {
    state::device_dir(avd).join("ui.json")
}

/// `ui`: dump, remember the numbering for `tap N`, and return it.
pub fn list(d: &Device) -> R<Vec<Element>> {
    let els = dump(d)?;
    std::fs::create_dir_all(state::device_dir(&d.avd))?;
    std::fs::write(snapshot_path(&d.avd), serde_json::to_string(&els)?)?;
    Ok(els)
}

/// Element `n` of the last `ui`, found again on the current screen (nearest match if several).
pub fn find(d: &Device, n: usize) -> R<Element> {
    let snap: Vec<Element> =
        std::fs::read_to_string(snapshot_path(&d.avd)).ok().and_then(|s| serde_json::from_str(&s).ok()).ok_or("run `herdr-android ui` first")?;
    let want = snap.get(n.wrapping_sub(1)).ok_or_else(|| format!("no element {n}; the last `ui` listed {}", snap.len()))?;
    relocate(want, &dump(d)?).ok_or_else(|| format!("element {n} ({}) is no longer on screen; run `ui` again", want.line(n)).into())
}

fn relocate(want: &Element, now: &[Element]) -> Option<Element> {
    let (wx, wy) = want.center();
    now.iter()
        .filter(|e| e.same(want))
        .min_by(|a, b| {
            let d = |e: &Element| (e.center().0 - wx).powi(2) + (e.center().1 - wy).powi(2);
            d(a).total_cmp(&d(b))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0">
<node index="0" text="" resource-id="" class="android.widget.FrameLayout" content-desc="" clickable="false" bounds="[0,0][1080,2400]">
<node index="0" text="Welcome &amp; hello" resource-id="com.example.demo:id/title" class="android.widget.TextView" content-desc="" clickable="false" bounds="[100,200][980,300]" />
<node index="1" text="Sign in" resource-id="com.example.demo:id/sign_in" class="android.widget.Button" content-desc="" clickable="true" bounds="[100,400][980,520]" />
<node index="2" text="" resource-id="com.example.demo:id/menu" class="android.widget.ImageButton" content-desc="" clickable="true" bounds="[0,0][0,0]" />
<node index="3" text="" resource-id="" class="android.widget.ImageView" content-desc="Profile" clickable="false" bounds="[10,10][90,90]" />
</node></hierarchy>"#;

    #[test]
    fn named_visible_elements_are_listed() {
        let els = parse(XML).unwrap();
        let lines: Vec<String> = els.iter().enumerate().map(|(i, e)| e.line(i + 1)).collect();
        assert_eq!(lines, [r#"[1] TextView "Welcome & hello" #title"#, r#"[2] Button "Sign in" #sign_in"#, r#"[3] ImageView "Profile""#]);
        assert_eq!(els[1].center(), (540.0, 460.0));
    }

    #[test]
    fn a_moved_element_is_found_again() {
        let els = parse(XML).unwrap();
        let mut moved = els[1].clone();
        moved.bounds = [100.0, 900.0, 980.0, 1020.0];
        assert_eq!(relocate(&els[1], &[els[0].clone(), moved.clone()]), Some(moved));
    }

    #[test]
    fn a_gone_element_is_not_tapped() {
        let els = parse(XML).unwrap();
        assert_eq!(relocate(&els[1], &[els[0].clone()]), None);
    }
}
