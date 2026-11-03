//! View button icons in herdr desktop's style (24-unit stroke icons, round caps),
//! rasterized for Kitty graphics; glyphs stand in where images aren't available.
//! Copied from herdr-web-browser, with Android's navigation icons.

use crate::R;
use resvg::{tiny_skia, usvg};

pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Back,
    Home,
    Recents,
    Rotate,
    Camera,
    Power,
}

pub const ALL: [Icon; 6] = [Icon::Back, Icon::Home, Icon::Recents, Icon::Rotate, Icon::Camera, Icon::Power];

impl Icon {
    fn body(self) -> &'static str {
        match self {
            Icon::Back => r#"<path d="M16.5 5v14L6 12z"/>"#,
            Icon::Home => r#"<circle cx="12" cy="12" r="7"/>"#,
            Icon::Recents => r#"<rect x="5.5" y="5.5" width="13" height="13" rx="2.5"/>"#,
            Icon::Rotate => r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>"#,
            Icon::Camera => r#"<path d="M4 8.5h3l1.8-2.5h6.4L17 8.5h3V19H4z"/><circle cx="12" cy="13.5" r="3.2"/>"#,
            Icon::Power => r#"<path d="M12 3v9"/><path d="M18.36 6.64a9 9 0 1 1-12.73 0"/>"#,
        }
    }

    /// Fallback when the terminal can't show images.
    pub fn glyph(self) -> &'static str {
        match self {
            Icon::Back => "◁",
            Icon::Home => "○",
            Icon::Recents => "□",
            Icon::Rotate => "↻",
            Icon::Camera => "◉",
            Icon::Power => "⏻",
        }
    }

    /// What the button does, shown while it's hovered.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Back => "Back",
            Icon::Home => "Home",
            Icon::Recents => "Recent apps",
            Icon::Rotate => "Rotate",
            Icon::Camera => "Send screenshot to agent",
            Icon::Power => "Shut the device down",
        }
    }

    pub fn index(self) -> u32 {
        ALL.iter().position(|x| *x == self).unwrap_or(0) as u32
    }

    /// PNG of the icon centered in a `w`×`h` px transparent box, filling `scale` of its height.
    pub fn png(self, color: Rgb, w: u32, h: u32, scale: f32) -> R<Vec<u8>> {
        draw(self, color, None, w, h, scale)
    }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// A hovered button: the icon on a rounded pill, like herdr desktop's hover state.
pub fn pill_png(icon: Icon, fg: Rgb, bg: Rgb, w: u32, h: u32) -> R<Vec<u8>> {
    draw(icon, fg, Some(bg), w, h, 0.62)
}

fn draw(icon: Icon, fg: Rgb, pill: Option<Rgb>, w: u32, h: u32, scale: f32) -> R<Vec<u8>> {
    let side = h as f32 * scale;
    let k = side / 24.0;
    let (dx, dy) = ((w as f32 - side) / 2.0, (h as f32 - side) / 2.0);
    let inset = h as f32 * 0.1;
    let rect = pill.map_or(String::new(), |bg| {
        format!(
            r#"<rect x="{inset}" y="{inset}" width="{}" height="{}" rx="{}" fill="{}"/>"#,
            w as f32 - 2.0 * inset,
            h as f32 - 2.0 * inset,
            (h as f32 - 2.0 * inset) * 0.32,
            hex(bg)
        )
    });
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{rect}<g transform="translate({dx} {dy}) scale({k})" fill="none" stroke="{}" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">{}</g></svg>"#,
        hex(fg),
        icon.body()
    );
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())?;
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("bad icon size")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Ok(pixmap.encode_png()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_with_visible_pixels() {
        for icon in ALL {
            let px = tiny_skia::Pixmap::decode_png(&icon.png((255, 255, 255), 34, 44, 0.62).unwrap()).unwrap();
            assert!(px.pixels().iter().any(|p| p.alpha() > 0), "{icon:?} is blank");
        }
    }

    #[test]
    fn pills_fill_their_middle() {
        let px = tiny_skia::Pixmap::decode_png(&pill_png(Icon::Home, (255, 255, 255), (9, 9, 9), 64, 40).unwrap()).unwrap();
        assert_eq!(px.pixel(8, 20).unwrap().alpha(), 255);
        assert_eq!(px.pixel(0, 0).unwrap().alpha(), 0);
    }
}
