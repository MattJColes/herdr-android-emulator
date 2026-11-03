//! The Device in a terminal pane: gRPC screen frames drawn by `render`, a header with
//! buttons, clicks and drags sent as touches, and typing sent as keyboard input.

use crate::adb::{self, Device};
use crate::emu::{Emu, Frame};
use crate::icons::{self, Icon, Rgb};
use crate::render::{self, Cells, IMAGE_ID, Renderer, SYNC_OFF, SYNC_ON};
use crate::state::{self, Config};
use crate::{R, agents};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::{cursor, execute, queue, terminal};
use std::io::{Write, stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// The device image starts below the header and a spacer row (herdr places images on
/// whole rows, so the gap keeps the screen from butting against the buttons).
const TOP: u16 = 2;

/// Frames and stream ends carry the stream generation, so a replaced stream's last frames are dropped.
enum Msg {
    Frame(u64, Frame),
    Gone(u64, String),
    Say(String),
}

struct Term;

impl Term {
    fn enter() -> R<Term> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide, event::EnableMouseCapture)?;
        Ok(Term)
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let mut out = stdout();
        render::kitty_clear(&mut out);
        for id in chrome_ids() {
            let _ = write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
        }
        let _ = execute!(out, event::DisableMouseCapture, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Act {
    Back,
    Home,
    Recents,
    Rotate,
    Shot,
    Stop,
}

/// Left: Android's navigation buttons. Right: device controls.
const LEFT: [(Icon, Act); 3] = [(Icon::Back, Act::Back), (Icon::Home, Act::Home), (Icon::Recents, Act::Recents)];
const RIGHT: [(Icon, Act); 3] = [(Icon::Rotate, Act::Rotate), (Icon::Camera, Act::Shot), (Icon::Power, Act::Stop)];
/// Each button is 4 columns: the icon (2 columns) centred, or a pill over all 4 when hovered.
const BTN: u16 = 4;

/// herdr desktop's dark look. The bar itself uses the terminal's own background so it
/// blends into the pane. ponytail: fixed dark palette, read herdr's theme if light themes complain.
#[derive(Debug, Clone, Copy)]
struct Palette {
    fg: Rgb,
    muted: Rgb,
    hover: Rgb,
    ok: Rgb,
    err: Rgb,
}

const PAL: Palette = Palette { fg: (238, 238, 239), muted: (156, 158, 161), hover: (53, 57, 62), ok: (61, 220, 132), err: (224, 108, 117) };

const ICON_BASE: u32 = 6000;
const PILL_BASE: u32 = 6100;

fn chrome_ids() -> Vec<u32> {
    icons::ALL.iter().flat_map(|i| [ICON_BASE + i.index(), PILL_BASE + i.index()]).collect()
}

/// A clickable header span.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hit {
    x0: u16,
    x1: u16,
    icon: Icon,
    act: Act,
}

/// Where each button sits in a pane `cols` wide.
fn layout(cols: u16) -> Vec<Hit> {
    let at = |x0: u16, (icon, act): (Icon, Act)| Hit { x0, x1: x0 + BTN, icon, act };
    let right0 = cols.saturating_sub(1 + BTN * RIGHT.len() as u16).max(2 + BTN * LEFT.len() as u16);
    let left = LEFT.iter().enumerate().map(|(i, b)| at(1 + BTN * i as u16, *b));
    let right = RIGHT.iter().enumerate().map(|(i, b)| at(right0 + BTN * i as u16, *b));
    left.chain(right).collect()
}

fn rgb(fg: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m", fg.0, fg.1, fg.2)
}

/// Where the device image sits in the pane, in cells.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    left: u16,
    cols: u16,
    rows: u16,
}

impl Placed {
    /// Pane cell -> device pixel, if the cell is on the image.
    fn pixel_at(&self, col: u16, row: u16, screen: (f64, f64)) -> Option<(f64, f64)> {
        let (x, y) = (col.checked_sub(self.left)?, row.checked_sub(TOP)?);
        (x < self.cols && y < self.rows).then(|| ((x as f64 + 0.5) / self.cols as f64 * screen.0, (y as f64 + 0.5) / self.rows as f64 * screen.1))
    }
}

struct View {
    cfg: Config,
    dev: Device,
    owner: Option<String>,
    emu: Option<Emu>,
    cells: Cells,
    renderer: Renderer,
    tx: Sender<Msg>,
    /// Width of the running stream, if one runs.
    stream: Option<u32>,
    want_w: u32,
    generation: Arc<AtomicU64>,
    /// Frame size in device pixels; frames turn sideways when the device rotates.
    screen: Option<(f64, f64)>,
    placed: Option<Placed>,
    flip: bool,
    last_frame: Instant,
    message: String,
    message_at: Instant,
    press: Option<(u16, u16)>,
    typed: String,
    /// Mouse position, for button hover.
    hover: Option<(u16, u16)>,
    /// Button icons uploaded as Kitty images.
    icons_ready: bool,
    /// Header bytes as last written; redraw only on change.
    drawn: Vec<u8>,
}

impl View {
    fn say(&mut self, m: impl Into<String>) {
        self.message = m.into();
        self.message_at = Instant::now();
    }

    fn ensure_stream(&mut self) -> R<()> {
        if self.stream == Some(self.want_w) {
            return Ok(());
        }
        if self.emu.is_none() {
            self.emu = Some(Emu::connect(&self.dev.avd)?);
        }
        if self.screen.is_none() {
            self.screen = Some(adb::screen_size(&self.dev)?);
        }
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, tx2, current) = (self.tx.clone(), self.tx.clone(), self.generation.clone());
        let fps = self.cfg.fps.unwrap_or(15).clamp(1, 30);
        let gap = Duration::from_millis(1000 / fps as u64);
        let mut last = Instant::now() - gap;
        self.emu.as_ref().unwrap().stream(
            self.want_w,
            move |f| {
                if current.load(Ordering::SeqCst) != g {
                    return false;
                }
                // The emulator sends every frame; the terminal only needs `fps` of them.
                if last.elapsed() >= gap {
                    last = Instant::now();
                    return tx.send(Msg::Frame(g, f)).is_ok();
                }
                true
            },
            move |reason| drop(tx2.send(Msg::Gone(g, reason))),
        );
        self.stream = Some(self.want_w);
        Ok(())
    }

    /// Run `f` off the UI thread; failures show in the header.
    fn input(&self, f: impl FnOnce(&Emu, &Device) -> R<()> + Send + 'static) {
        let (dev, tx) = (self.dev.clone(), self.tx.clone());
        let Some(emu) = self.emu.clone() else {
            let _ = tx.send(Msg::Say("error: not connected to the emulator yet".into()));
            return;
        };
        std::thread::spawn(move || {
            if let Err(e) = f(&emu, &dev) {
                let _ = tx.send(Msg::Say(format!("error: {e}")));
            }
        });
    }

    fn title(&self) -> String {
        format!("{} · {}", self.dev.avd, self.dev.serial)
    }

    fn hovered(&self) -> Option<Hit> {
        let (c, r) = self.hover?;
        let cols = terminal::size().ok()?.0;
        layout(cols).into_iter().find(|h| r == 0 && (h.x0..h.x1).contains(&c))
    }

    /// The middle of the bar is the status line: a hovered button's name, else the
    /// latest message for a few seconds, else the device.
    fn status(&self) -> (String, Rgb) {
        if let Some(h) = self.hovered() {
            return (h.icon.name().into(), PAL.fg);
        }
        if !self.message.is_empty() && self.message_at.elapsed() < Duration::from_secs(6) {
            return (self.message.clone(), if self.message.starts_with("error") { PAL.err } else { PAL.ok });
        }
        (self.title(), PAL.muted)
    }

    fn header(&self) -> R<Vec<u8>> {
        let (cols, _) = terminal::size()?;
        let hits = layout(cols);
        let hot = self.hovered();
        let images = self.renderer == Renderer::Kitty && self.icons_ready;
        let mut out = Vec::from(SYNC_ON.as_bytes());
        queue!(out, cursor::MoveTo(0, 0), terminal::Clear(terminal::ClearType::CurrentLine))?;
        let mut slots = Vec::new();
        for h in &hits {
            queue!(out, cursor::MoveTo(h.x0, 0))?;
            let on = hot == Some(*h);
            if images {
                slots.push(if on { (h.x0, PILL_BASE + h.icon.index(), BTN) } else { (h.x0 + 1, ICON_BASE + h.icon.index(), 2) });
            } else if on {
                let (bg, fg) = (PAL.hover, PAL.fg);
                write!(out, "{}\x1b[48;2;{};{};{}m {} \x1b[0m", rgb(fg), bg.0, bg.1, bg.2, format_args!("{} ", h.icon.glyph()))?;
            } else {
                write!(out, "{} {}  \x1b[0m", rgb(PAL.muted), h.icon.glyph())?;
            }
        }
        // Status, centred between the two button groups.
        let (x0, x1) = (hits[LEFT.len() - 1].x1 + 1, hits[LEFT.len()].x0.saturating_sub(1));
        let room = x1.saturating_sub(x0) as usize;
        let (text, fg) = self.status();
        let text: String = if text.chars().count() > room { text.chars().take(room.saturating_sub(1)).chain(['…']).collect() } else { text };
        let pad = (room - text.chars().count().min(room)) / 2;
        queue!(out, cursor::MoveTo(x0 + pad as u16, 0))?;
        write!(out, "{}{text}\x1b[0m", rgb(fg))?;
        if images {
            // Re-place every icon: drop old placements (keeping the images), put new ones.
            for id in chrome_ids() {
                write!(out, "\x1b_Ga=d,d=i,i={id},q=2\x1b\\")?;
            }
            for (col, id, w) in slots {
                queue!(out, cursor::MoveTo(col, 0))?;
                write!(out, "\x1b_Ga=p,i={id},p=1,c={},r=1,C=1,q=2\x1b\\", self.cells.img_cols(w))?;
            }
        }
        out.extend_from_slice(SYNC_OFF.as_bytes());
        Ok(out)
    }

    /// Upload every icon, plain and hovered, once.
    fn upload_icons(&mut self) -> R<()> {
        if self.renderer != Renderer::Kitty {
            return Ok(());
        }
        use base64::Engine;
        let b64 = |png: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(png);
        // Twice the cell size, so icons stay crisp when the terminal scales them.
        let (cw, h) = ((self.cells.w * 2.0).round() as u32, (self.cells.h * 2.0).round() as u32);
        let mut out = Vec::new();
        for icon in icons::ALL {
            render::kitty_upload(&mut out, &b64(icon.png(PAL.muted, 2 * cw, h, 0.62)?), ICON_BASE + icon.index())?;
            render::kitty_upload(&mut out, &b64(icons::pill_png(icon, PAL.fg, PAL.hover, BTN as u32 * cw, h)?), PILL_BASE + icon.index())?;
        }
        let mut so = stdout().lock();
        so.write_all(&out)?;
        so.flush()?;
        self.icons_ready = true;
        Ok(())
    }

    fn button_at(&self, col: u16, row: u16) -> Option<Act> {
        let cols = terminal::size().ok()?.0;
        layout(cols).into_iter().find(|h| row == 0 && (h.x0..h.x1).contains(&col)).map(|h| h.act)
    }

    fn draw_header(&mut self) -> R<()> {
        let out = self.header()?;
        if out == self.drawn {
            return Ok(());
        }
        let mut so = stdout().lock();
        so.write_all(&out)?;
        so.flush()?;
        self.drawn = out;
        Ok(())
    }

    fn draw_frame(&mut self, f: &Frame) -> R<()> {
        let (cols, rows) = terminal::size()?;
        let rows = rows.saturating_sub(TOP).max(1);
        let (c, r) = render::fit(f.w as f64, f.h as f64, cols, rows, self.cells.aspect());
        let placed = Placed { left: (cols - c) / 2, cols: c, rows: r };
        let want = match self.renderer {
            Renderer::Kitty => c as f64 * self.cells.w,
            Renderer::Blocks => c as f64,
        };
        // Ask the emulator for frames close to the drawn size: bigger wastes encode time, smaller looks soft.
        let want = (want.round() as u32).clamp(64, 1440);
        if want.abs_diff(self.want_w) as f64 > self.want_w as f64 * 0.25 {
            self.want_w = want;
        }
        // A sideways frame means the device rotated: touches map onto the rotated size.
        if let Some((sw, sh)) = self.screen
            && (f.w > f.h) != (sw > sh)
        {
            self.screen = Some((sh, sw));
        }
        let mut out = Vec::from(SYNC_ON.as_bytes());
        if self.placed != Some(placed) {
            render::kitty_clear(&mut out);
            for row in TOP..TOP + rows {
                queue!(out, cursor::MoveTo(0, row), terminal::Clear(terminal::ClearType::CurrentLine))?;
            }
        }
        match self.renderer {
            Renderer::Kitty => {
                let (id, old) = if self.flip { (IMAGE_ID + 1, IMAGE_ID) } else { (IMAGE_ID, IMAGE_ID + 1) };
                self.flip = !self.flip;
                queue!(out, cursor::MoveTo(placed.left, TOP))?;
                render::kitty_swap(&mut out, &render::png_b64(f.w, f.h, &f.rgb)?, id, old, self.cells.img_cols(c), r)?;
            }
            Renderer::Blocks => render::blocks(&mut out, (f.w, f.h, &f.rgb), TOP, placed.left, c, r)?,
        }
        out.extend_from_slice(SYNC_OFF.as_bytes());
        self.placed = Some(placed);
        let mut so = stdout().lock();
        so.write_all(&out)?;
        so.flush()?;
        Ok(())
    }

    /// Returns false when the view should close.
    fn act(&mut self, act: Act) -> bool {
        match act {
            Act::Back => self.input(|e, _| e.key("GoBack")),
            Act::Home => self.input(|e, _| e.key("GoHome")),
            Act::Recents => self.input(|e, _| e.key("AppSwitch")),
            Act::Rotate => self.input(|_, d| adb::adb(Some(&d.serial), &["emu", "rotate"], 15).map(drop)),
            Act::Shot => {
                let (dev, owner, tx) = (self.dev.clone(), self.owner.clone(), self.tx.clone());
                self.say("taking a screenshot…");
                std::thread::spawn(move || {
                    let m = agents::send_screenshot(&dev, owner.as_deref()).unwrap_or_else(|e| format!("error: {e}"));
                    let _ = tx.send(Msg::Say(m));
                });
            }
            Act::Stop => {
                let _ = adb::shutdown(&self.dev);
                return false;
            }
        }
        true
    }

    fn flush_typing(&mut self) {
        if !self.typed.is_empty() {
            let text = std::mem::take(&mut self.typed);
            self.input(move |e, _| e.text(&text));
        }
    }

    fn on_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('q') if ctrl => return false,
            KeyCode::Char('b') if ctrl => return self.act(Act::Back),
            KeyCode::Char('o') if ctrl => return self.act(Act::Home),
            KeyCode::Char('x') if ctrl => return self.act(Act::Shot),
            KeyCode::Char(c) if !ctrl => self.typed.push(c),
            KeyCode::Enter | KeyCode::Backspace | KeyCode::Tab => {
                self.flush_typing();
                let key = match k.code {
                    KeyCode::Enter => "Enter",
                    KeyCode::Backspace => "Backspace",
                    _ => "Tab",
                };
                self.input(move |e, _| e.key(key));
            }
            _ => {}
        }
        true
    }

    fn on_mouse(&mut self, m: event::MouseEvent) -> bool {
        if matches!(m.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
            self.hover = Some((m.column, m.row));
        }
        if m.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(a) = self.button_at(m.column, m.row)
        {
            // Show the click's result, not the button's name, until the mouse moves.
            self.hover = None;
            return self.act(a);
        }
        let (Some(placed), Some(screen)) = (self.placed, self.screen) else { return true };
        let at = placed.pixel_at(m.column, m.row, screen);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.press = at.is_some().then_some((m.column, m.row)),
            MouseEventKind::Up(MouseButton::Left) => {
                let Some((c0, r0)) = self.press.take() else { return true };
                let Some(from) = placed.pixel_at(c0, r0, screen) else { return true };
                let to = placed
                    .pixel_at(m.column.clamp(placed.left, placed.left + placed.cols - 1), m.row.clamp(TOP, TOP + placed.rows - 1), screen)
                    .unwrap_or(from);
                let avd = self.dev.avd.clone();
                if c0.abs_diff(m.column) + r0.abs_diff(m.row) <= 1 {
                    let _ = state::event(&avd, "user tap", &format!("{:.0},{:.0}", from.0, from.1));
                    self.input(move |e, _| e.tap(from.0, from.1));
                } else {
                    let _ = state::event(&avd, "user swipe", &format!("{:.0},{:.0} -> {:.0},{:.0}", from.0, from.1, to.0, to.1));
                    self.input(move |e, _| e.drag(from, to, 250));
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                if let Some((x, y)) = at {
                    let dy = if m.kind == MouseEventKind::ScrollDown { -screen.1 * 0.08 } else { screen.1 * 0.08 };
                    let to = (x, (y + dy).clamp(1.0, screen.1 - 1.0));
                    self.input(move |e, _| e.drag((x, y), to, 120));
                }
            }
            _ => {}
        }
        true
    }
}

pub fn run(cfg: Config, device: Option<&str>, owner: Option<String>) -> R<()> {
    let _term = Term::enter()?;
    let mut so = stdout();
    write!(so, "\x1b[1;1H starting the Android device (about 30s the first time)…")?;
    so.flush()?;
    let dev = adb::current(&cfg, device)?;
    let (cells, renderer) = render::detect(&cfg);
    let (tx, rx) = mpsc::channel();
    let mut v = View {
        cfg,
        dev,
        owner,
        emu: None,
        cells,
        renderer,
        tx,
        stream: None,
        want_w: 540,
        generation: Arc::new(AtomicU64::new(0)),
        screen: None,
        placed: None,
        flip: false,
        last_frame: Instant::now(),
        message: "click to tap · drag to swipe · type to type · Ctrl-Q close".into(),
        message_at: Instant::now(),
        press: None,
        typed: String::new(),
        hover: None,
        icons_ready: false,
        drawn: Vec::new(),
    };
    if let Err(e) = v.upload_icons() {
        v.say(format!("error: icons: {e}"));
    }
    let result = event_loop(&mut v, &rx);
    // Stop the stream: its callback sees a new generation and returns false.
    v.generation.fetch_add(1, Ordering::SeqCst);
    result
}

fn event_loop(v: &mut View, rx: &Receiver<Msg>) -> R<()> {
    let mut last_tick = Instant::now() - Duration::from_secs(10);
    loop {
        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            if let Err(e) = v.ensure_stream() {
                v.say(format!("error: {e}"));
            }
            v.draw_header()?;
        }
        let mut latest = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Frame(g, f) if g == v.generation.load(Ordering::SeqCst) => latest = Some(f),
                Msg::Frame(..) => {}
                Msg::Gone(g, e) if g == v.generation.load(Ordering::SeqCst) => {
                    v.stream = None;
                    // The emulator may have gone: reconnect on the next tick.
                    v.emu = None;
                    if !e.is_empty() {
                        v.say(e);
                    }
                }
                Msg::Gone(..) => {}
                Msg::Say(s) => v.say(s),
            }
        }
        if let Some(f) = latest {
            v.last_frame = Instant::now();
            if let Err(e) = v.draw_frame(&f) {
                v.say(format!("error: {e}"));
            }
            // draw_frame picked a new width: restart the stream at it.
            if v.stream.is_some_and(|w| w != v.want_w) {
                last_tick = Instant::now() - Duration::from_secs(10);
            }
        }
        if !event::poll(Duration::from_millis(20))? {
            v.flush_typing();
            continue;
        }
        loop {
            let keep = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => v.on_key(k),
                Event::Mouse(m) => v.on_mouse(m),
                Event::Resize(..) => {
                    v.placed = None;
                    v.drawn.clear();
                    last_tick = Instant::now() - Duration::from_secs(10);
                    true
                }
                _ => true,
            };
            if !keep {
                return Ok(());
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
        v.flush_typing();
        v.draw_header()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_map_to_device_pixels() {
        let p = Placed { left: 10, cols: 32, rows: 64 };
        // Cell (16, 32) of a 32x64 image on a 1080x2160 px screen: its centre.
        assert_eq!(p.pixel_at(26, TOP + 32, (1080.0, 2160.0)), Some((556.875, 1096.875)));
    }

    #[test]
    fn cells_off_the_image_are_not_pixels() {
        let p = Placed { left: 10, cols: 40, rows: 80 };
        assert_eq!(p.pixel_at(9, TOP, (1080.0, 2160.0)), None);
        assert_eq!(p.pixel_at(50, TOP, (1080.0, 2160.0)), None);
        assert_eq!(p.pixel_at(20, 0, (1080.0, 2160.0)), None);
    }

    #[test]
    fn buttons_sit_at_both_ends_without_overlapping() {
        let hits = layout(80);
        assert_eq!(hits.len(), 6);
        assert_eq!((hits[0].x0, hits[0].act), (1, Act::Back));
        assert_eq!((hits[5].x1, hits[5].act), (79, Act::Stop));
        assert!(hits.windows(2).all(|w| w[0].x1 <= w[1].x0));
        // Too narrow: the right group stops at the left one instead of overlapping it.
        assert!(layout(10).windows(2).all(|w| w[0].x1 <= w[1].x0));
    }
}
