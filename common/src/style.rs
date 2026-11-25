//! The UI style shared by the WinRust programs: %ProgramData%\WinRust\style.txt holds its name (the topbar gear menu writes it).
//! Programs re-read it at most once a second and repaint at once on the broadcast `set_style` sends (`msg()`).
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, RegisterWindowMessageW};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Normal,
    Terminal, // monospace, green on black, square corners
    Dock,     // the bar is a floating pill in the middle
    Tiling,   // a thin status strip at the top, windows side by side
    Road,     // "White road": light ivory, faint triangles in the background
    Ruby,     // deep ruby, a smooth glossy gradient
    Win98,    // BETA: classic grey, navy accent, square corners, Tahoma
    Win7,     // BETA: Aero-like glass blue
    Win11,    // the stock Windows 11 dark taskbar: Start first, centered, flat, no gear
}

pub const ALL: [Style; 9] = [Style::Normal, Style::Terminal, Style::Dock, Style::Tiling, Style::Road, Style::Ruby, Style::Win98, Style::Win7, Style::Win11];

/// The look of a fresh install and of "Reset everything to defaults".
pub const DEFAULT: Style = Style::Win11;

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Normal => "normal",
            Style::Terminal => "terminal",
            Style::Dock => "dock",
            Style::Tiling => "tiling",
            Style::Road => "road",
            Style::Ruby => "ruby",
            Style::Win98 => "win98",
            Style::Win7 => "win7",
            Style::Win11 => "win11",
        }
    }
    /// Unknown or empty text (no file yet, a hand edit) is the default style.
    pub fn parse(s: &str) -> Style {
        ALL.into_iter().find(|v| v.name().eq_ignore_ascii_case(s.trim())).unwrap_or(DEFAULT)
    }
    /// Square corners everywhere.
    // only the terminal keeps the monospace, square look; tiling uses the same sans and rounded shapes as the rest
    pub fn square(self) -> bool {
        matches!(self, Style::Terminal | Style::Win98)
    }
    /// Still unfinished: the menu marks it with a red BETA.
    pub fn beta(self) -> bool {
        matches!(self, Style::Win98 | Style::Win7)
    }
    pub fn mono(self) -> bool {
        self == Style::Terminal
    }
    /// Dark text on a light background.
    pub fn light(self) -> bool {
        matches!(self, Style::Road | Style::Win98)
    }
}

static CUR: AtomicU8 = AtomicU8::new(u8::MAX); // index into ALL; MAX = not read yet
static READ_AT: AtomicU64 = AtomicU64::new(0); // when the file was last read, ms since the epoch

fn file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("style.txt"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn store(s: Style) {
    CUR.store(ALL.iter().position(|&v| v == s).unwrap_or(0) as u8, Relaxed);
    READ_AT.store(now_ms(), Relaxed);
}

/// Reads the file again now (on the "style changed" broadcast).
pub fn reload() {
    // WINRUST_STYLE overrides the file for this process only (to look at a style without switching every window)
    let name = std::env::var("WINRUST_STYLE").ok().or_else(|| file().and_then(|f| std::fs::read_to_string(f).ok()));
    store(Style::parse(&name.unwrap_or_default()));
}

/// The current style. The file is read again at most once a second, so a program that missed the broadcast still follows.
pub fn style() -> Style {
    if CUR.load(Relaxed) == u8::MAX || now_ms().saturating_sub(READ_AT.load(Relaxed)) > 1000 {
        reload();
    }
    ALL[CUR.load(Relaxed) as usize % ALL.len()]
}

/// Switches this process, writes the choice and tells the other WinRust windows to repaint in it.
pub fn set_style(s: Style) {
    store(s);
    if let Some(f) = file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, s.name());
    }
    unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
}

fn autohide_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("autohide.txt"))
}

/// The bar hides when the pointer is away and comes back at the screen edge (off unless the file says 1). Re-read once a second.
pub fn autohide() -> bool {
    if now_ms().saturating_sub(HIDE_AT.load(Relaxed)) > 1000 {
        HIDE.store(autohide_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|s| s.trim() == "1") as u8, Relaxed);
        HIDE_AT.store(now_ms(), Relaxed);
    }
    HIDE.load(Relaxed) == 1
}

static HIDE: AtomicU8 = AtomicU8::new(0);
static HIDE_AT: AtomicU64 = AtomicU64::new(0);

pub fn set_autohide(on: bool) {
    HIDE.store(on as u8, Relaxed);
    HIDE_AT.store(now_ms(), Relaxed);
    if let Some(f) = autohide_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, if on { "1" } else { "0" });
    }
}

fn left_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("align-left.txt"))
}

static LEFT: AtomicU8 = AtomicU8::new(0);
static LEFT_AT: AtomicU64 = AtomicU64::new(0);

/// Start and the pinned apps sit at the bar's left end, like Windows 10 (off unless the file says 1). Re-read once a second.
pub fn align_left() -> bool {
    if now_ms().saturating_sub(LEFT_AT.load(Relaxed)) > 1000 {
        LEFT.store(left_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|s| s.trim() == "1") as u8, Relaxed);
        LEFT_AT.store(now_ms(), Relaxed);
    }
    LEFT.load(Relaxed) == 1
}

pub fn set_align_left(on: bool) {
    LEFT.store(on as u8, Relaxed);
    LEFT_AT.store(now_ms(), Relaxed);
    if let Some(f) = left_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, if on { "1" } else { "0" });
    }
}

/// Which screen edge the topbar sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Bottom,
    Left,
    Right,
}

impl Side {
    pub fn parse(s: &str) -> Side {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Side::Left,
            "right" => Side::Right,
            _ => Side::Bottom,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Side::Bottom => "bottom",
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

fn side_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("side.txt"))
}

static SIDE: AtomicU8 = AtomicU8::new(0);
static SIDE_AT: AtomicU64 = AtomicU64::new(0);

fn side_code(s: Side) -> u8 {
    match s {
        Side::Bottom => 0,
        Side::Left => 1,
        Side::Right => 2,
    }
}

/// The bar's edge (bottom unless %ProgramData%\WinRust\side.txt says left or right). Re-read once a second.
pub fn side() -> Side {
    if now_ms().saturating_sub(SIDE_AT.load(Relaxed)) > 1000 {
        let s = side_file().and_then(|f| std::fs::read_to_string(f).ok()).map_or(Side::Bottom, |s| Side::parse(&s));
        SIDE.store(side_code(s), Relaxed);
        SIDE_AT.store(now_ms(), Relaxed);
    }
    match SIDE.load(Relaxed) {
        1 => Side::Left,
        2 => Side::Right,
        _ => Side::Bottom,
    }
}

pub fn set_side(s: Side) {
    SIDE.store(side_code(s), Relaxed);
    SIDE_AT.store(now_ms(), Relaxed);
    if let Some(f) = side_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, s.name());
    }
}

/// The broadcast message a window gets when the style changed: call `reload()` and repaint.
pub fn msg() -> u32 {
    static M: AtomicU32 = AtomicU32::new(0);
    match M.load(Relaxed) {
        0 => {
            let n: Vec<u16> = "WinRustStyleChanged\0".encode_utf16().collect();
            let m = unsafe { RegisterWindowMessageW(n.as_ptr()) };
            M.store(m, Relaxed);
            m
        }
        m => m,
    }
}

/// Colours of one style, as COLORREF (0x00BBGGRR) like GDI wants them.
#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: u32,
    pub line: u32,
    pub hover: u32,
    pub active: u32,
    pub accent: u32,
    pub text: u32,
    pub dim: u32,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// In the order of `ALL`. Neutral, low-contrast surfaces and one restrained accent (the look of macOS / modern fintech apps).
pub const PALS: [Pal; 9] = [
    // normal: graphite with the Apple system blue
    Pal { bg: rgb(22, 22, 24), line: rgb(46, 46, 50), hover: rgb(36, 36, 40), active: rgb(44, 44, 49), accent: rgb(10, 132, 255), text: rgb(245, 245, 247), dim: rgb(142, 142, 147) },
    // terminal: green on black
    Pal { bg: rgb(8, 11, 9), line: rgb(30, 48, 35), hover: rgb(16, 28, 20), active: rgb(22, 38, 27), accent: rgb(74, 222, 128), text: rgb(204, 232, 212), dim: rgb(98, 132, 108) },
    // dock: midnight ink with indigo
    Pal { bg: rgb(17, 18, 25), line: rgb(40, 42, 56), hover: rgb(29, 31, 42), active: rgb(37, 40, 54), accent: rgb(124, 134, 255), text: rgb(240, 241, 246), dim: rgb(138, 142, 160) },
    // tiling: dark with amber
    Pal { bg: rgb(16, 17, 20), line: rgb(38, 40, 46), hover: rgb(27, 29, 34), active: rgb(35, 37, 43), accent: rgb(245, 182, 82), text: rgb(224, 226, 231), dim: rgb(120, 125, 138) },
    // white road: warm ivory with gold
    Pal { bg: rgb(247, 245, 240), line: rgb(226, 220, 207), hover: rgb(238, 233, 222), active: rgb(232, 225, 210), accent: rgb(176, 134, 52), text: rgb(28, 26, 22), dim: rgb(124, 117, 104) },
    // ruby: deep garnet surfaces, a bright ruby accent, warm rose-white text
    Pal { bg: rgb(26, 7, 13), line: rgb(72, 22, 36), hover: rgb(46, 13, 24), active: rgb(60, 17, 31), accent: rgb(232, 52, 84), text: rgb(252, 238, 241), dim: rgb(178, 126, 139) },
    // win98: button-face grey, navy title colour
    Pal { bg: rgb(192, 192, 192), line: rgb(128, 128, 128), hover: rgb(212, 208, 200), active: rgb(160, 160, 160), accent: rgb(0, 0, 128), text: rgb(0, 0, 0), dim: rgb(64, 64, 64) },
    // win7: Aero glass blue
    Pal { bg: rgb(31, 58, 96), line: rgb(72, 108, 154), hover: rgb(58, 92, 138), active: rgb(74, 112, 160), accent: rgb(120, 190, 255), text: rgb(255, 255, 255), dim: rgb(176, 198, 226) },
    // win11: the dark taskbar's #202020 base, white 6 % / 9 % hover and focus plates, the default dark-mode accent
    Pal { bg: rgb(32, 32, 32), line: rgb(48, 48, 48), hover: rgb(45, 45, 45), active: rgb(52, 52, 52), accent: rgb(76, 194, 255), text: rgb(255, 255, 255), dim: rgb(157, 157, 157) },
];

/// Status colours shared by every style (Apple's dark-mode system green, yellow, red).
pub const GOOD: u32 = rgb(48, 209, 88);
pub const WARN: u32 = rgb(255, 214, 10);
pub const BAD: u32 = rgb(255, 69, 58);

pub fn pal() -> &'static Pal {
    let base = &PALS[ALL.iter().position(|&s| s == style()).unwrap_or(0)];
    let Some(o) = custom_colors() else { return base };
    // one leaked copy per (style, overrides): a handful at most, and pal() has to hand out a reference
    static CACHE: std::sync::Mutex<Vec<(usize, [u32; 7], &'static Pal)>> = std::sync::Mutex::new(Vec::new());
    let key = base as *const Pal as usize;
    let mut v = CACHE.lock().unwrap();
    if let Some(e) = v.iter().find(|e| e.0 == key && e.1 == o) {
        return e.2;
    }
    let p: &'static Pal = Box::leak(Box::new(custom_pal(base, o)));
    v.push((key, o, p));
    p
}

/// The style's palette with the user's colours on top. Lines, hover and pressed colours that were not set follow the
/// background and text colours, so changing just the background still gives a consistent bar.
fn custom_pal(base: &Pal, o: [u32; 7]) -> Pal {
    let get = |i: usize, d: u32| if o[i] == u32::MAX { d } else { o[i] };
    let bg = get(0, base.bg);
    let text = get(5, base.text);
    let derive = o[0] != u32::MAX || o[5] != u32::MAX;
    Pal {
        bg,
        line: get(1, if derive { mix(bg, text, 0.12) } else { base.line }),
        hover: get(2, if derive { mix(bg, text, 0.07) } else { base.hover }),
        active: get(3, if derive { mix(bg, text, 0.12) } else { base.active }),
        accent: get(4, base.accent),
        text,
        dim: get(6, if derive { mix(bg, text, 0.55) } else { base.dim }),
    }
}

/// Names of the colours the user can set, in the order of `Pal`'s fields (bg, line, hover, active, accent, text, dim).
pub const COLOR_NAMES: [&str; 7] = ["bg", "line", "hover", "active", "accent", "text", "dim"];

fn colors_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("colors.txt"))
}

static COLORS: std::sync::Mutex<[u32; 7]> = std::sync::Mutex::new([u32::MAX; 7]); // COLORREF, u32::MAX = the style's own
static COLORS_AT: AtomicU64 = AtomicU64::new(0);

fn read_colors() -> [u32; 7] {
    parse_colors(&colors_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default())
}

/// `bg=RRGGBB` pairs, one per line or separated by `;` (colors.txt and the templates); unknown names and junk are skipped.
fn parse_colors(text: &str) -> [u32; 7] {
    let mut o = [u32::MAX; 7];
    for l in text.split(['\n', ';']) {
        if let Some((k, v)) = l.split_once('=') {
            if let (Some(i), Ok(v)) = (COLOR_NAMES.iter().position(|n| *n == k.trim()), u32::from_str_radix(v.trim(), 16)) {
                if v <= 0xFFFFFF { o[i] = rgb((v >> 16) as u8, (v >> 8) as u8, v as u8); }
            }
        }
    }
    o
}

fn colors_text(c: &[u32; 7], sep: &str) -> String {
    c.iter().enumerate().filter(|(_, v)| **v != u32::MAX).map(|(i, v)| format!("{}={:02X}{:02X}{:02X}{sep}", COLOR_NAMES[i], v & 255, (v >> 8) & 255, (v >> 16) & 255)).collect()
}

/// The user's colours (one per `COLOR_NAMES`, u32::MAX = not set), None when none is set. Re-read once a second.
pub fn custom_colors() -> Option<[u32; 7]> {
    let mut c = COLORS.lock().unwrap();
    if now_ms().saturating_sub(COLORS_AT.load(Relaxed)) > 1000 {
        *c = read_colors();
        COLORS_AT.store(now_ms(), Relaxed);
    }
    c.iter().any(|v| *v != u32::MAX).then_some(*c)
}

/// Sets colour `i` (COLORREF; None = back to the style's own) and tells the other WinRust windows to repaint.
pub fn set_custom_color(i: usize, c: Option<u32>) {
    let mut cur = read_colors();
    cur[i] = c.unwrap_or(u32::MAX);
    save_colors(cur);
}

/// Back to the style's own colours and opacity.
pub fn reset_custom() {
    save_colors([u32::MAX; 7]);
    set_bar_opacity(100);
}

fn save_colors(c: [u32; 7]) {
    let text = colors_text(&c, "\n");
    if let Some(f) = colors_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = if text.is_empty() { std::fs::remove_file(f) } else { std::fs::write(f, text) };
    }
    *COLORS.lock().unwrap() = c;
    COLORS_AT.store(now_ms(), Relaxed);
    unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
}

fn opacity_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("bar-opacity.txt"))
}

static OPACITY: AtomicU8 = AtomicU8::new(100);
static OPACITY_AT: AtomicU64 = AtomicU64::new(0);

/// How opaque the topbar is, percent (30..=100). Re-read once a second.
pub fn bar_opacity() -> u8 {
    if now_ms().saturating_sub(OPACITY_AT.load(Relaxed)) > 1000 {
        let v = opacity_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| s.trim().parse::<u8>().ok()).map_or(100, |v| v.clamp(30, 100));
        OPACITY.store(v, Relaxed);
        OPACITY_AT.store(now_ms(), Relaxed);
    }
    OPACITY.load(Relaxed)
}

pub fn set_bar_opacity(v: u8) {
    let v = v.clamp(30, 100);
    OPACITY.store(v, Relaxed);
    OPACITY_AT.store(now_ms(), Relaxed);
    if let Some(f) = opacity_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, v.to_string());
    }
}

/// A saved look (a "template"): the style, the user's colours and the bar's opacity. One per line of
/// %ProgramData%\WinRust\themes.txt: `name<TAB>style<TAB>opacity<TAB>bg=RRGGBB;text=RRGGBB;`.
#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    pub name: String,
    pub style: Style,
    pub colors: [u32; 7],
    pub opacity: u8,
}

impl Theme {
    fn line(&self) -> String {
        format!("{}\t{}\t{}\t{}", self.name.replace(['\t', '\r', '\n'], " "), self.style.name(), self.opacity, colors_text(&self.colors, ";"))
    }
    fn parse(l: &str) -> Option<Theme> {
        let mut f = l.trim_end_matches('\r').splitn(4, '\t');
        let name = f.next().filter(|n| !n.trim().is_empty())?.to_string();
        let style = Style::parse(f.next()?);
        let opacity = f.next()?.trim().parse::<u8>().ok()?.clamp(30, 100);
        Some(Theme { name, style, colors: parse_colors(f.next().unwrap_or("")), opacity })
    }
    /// What is on screen now.
    pub fn current(name: &str) -> Theme {
        Theme { name: name.into(), style: style(), colors: read_colors(), opacity: bar_opacity() }
    }
    /// Switches every WinRust window to this look.
    pub fn apply(&self) {
        set_bar_opacity(self.opacity);
        save_colors(self.colors);
        set_style(self.style);
    }
}

fn themes_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("themes.txt"))
}

/// The saved templates, oldest first.
pub fn themes() -> Vec<Theme> {
    themes_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default().lines().filter_map(Theme::parse).collect()
}

pub fn save_themes(t: &[Theme]) {
    if let Some(f) = themes_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, t.iter().map(|t| t.line() + "\n").collect::<String>());
    }
}

/// Everything the gear menu sets, back to how a fresh install looks (the saved templates stay).
pub fn reset_all() {
    set_side(Side::Bottom);
    set_bar_size(1);
    set_autohide(false);
    set_align_left(false);
    set_alttab_per_monitor(false);
    Theme { name: String::new(), style: DEFAULT, colors: [u32::MAX; 7], opacity: 100 }.apply();
}

/// A raised surface (a card, a tile): pure white on the light style, a step up from the background on the dark ones.
pub fn card() -> u32 {
    if style().light() { rgb(255, 255, 255) } else { mix(pal().bg, pal().text, 0.06) }
}

/// Text and glyphs drawn on an accent fill.
pub fn on_accent() -> u32 {
    if style().light() || style() == Style::Ruby { rgb(255, 255, 255) } else { pal().bg }
}

/// `a` moved toward `b` by `t` (0..1), per channel.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        (x + (y - x) * t.clamp(0.0, 1.0)).round() as u32
    };
    ch(0) | ch(8) << 8 | ch(16) << 16
}

/// The White road background over `r`: faceted ivory triangles, a few of them traced in gold.
/// Does nothing in the other styles. Needs GDI+ started (GdiplusStartup) by the program.
pub unsafe fn backdrop(dc: windows_sys::Win32::Graphics::Gdi::HDC, r: windows_sys::Win32::Foundation::RECT, scale: f32) {
    use windows_sys::Win32::Graphics::GdiPlus::*;
    let ruby = style() == Style::Ruby;
    if !style().light() && !ruby {
        return;
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    if unsafe { GdipCreateFromHDC(dc, &mut g) } != 0 || g.is_null() {
        return;
    }
    let argb = |c: u32, a: f32| ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24 | (c & 255) << 16 | (c & 0xFF00) | (c >> 16 & 255);
    if ruby {
        // smooth: a vertical ruby gradient (lighter on top, near-black garnet below) and a soft gloss over the upper third
        let (x, y, w, h) = (r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1));
        unsafe {
            GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            let mut br: *mut GpLineGradient = std::ptr::null_mut();
            GdipCreateLineBrushI(&Point { X: x, Y: y }, &Point { X: x, Y: y + h }, argb(mix(pal().bg, rgb(150, 18, 42), 0.62), 1.0), argb(mix(pal().bg, rgb(0, 0, 0), 0.25), 1.0), WrapModeTileFlipXY, &mut br);
            GdipFillRectangleI(g, br as *mut GpBrush, x, y, w, h);
            GdipDeleteBrush(br as *mut GpBrush);
            let gh = (h / 3).max(1);
            GdipCreateLineBrushI(&Point { X: x, Y: y }, &Point { X: x, Y: y + gh }, argb(rgb(255, 255, 255), 0.09), argb(rgb(255, 255, 255), 0.0), WrapModeTileFlipXY, &mut br);
            GdipFillRectangleI(g, br as *mut GpBrush, x, y, w, gh);
            GdipDeleteBrush(br as *mut GpBrush);
            GdipDeleteGraphics(g);
        }
        return;
    }
    let (x0, y0, w, h) = (r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32);
    let s = 58.0 * scale.max(0.5);
    let (cols, rows) = ((w / s) as i32 + 2, (h / (s * 0.866)) as i32 + 2);
    // a lattice of equilateral-ish triangles; each vertex nudged by a hash so the facets look cut by hand
    let pt = |i: i32, j: i32| {
        let (hx, hy) = (hash(i, j, 1) - 0.5, hash(i, j, 2) - 0.5);
        let off = if j & 1 == 1 { s * 0.5 } else { 0.0 };
        PointF { X: x0 + i as f32 * s - off + hx * s * 0.34, Y: y0 + j as f32 * s * 0.866 + hy * s * 0.3 - s * 0.4 }
    };
    let gold = pal().accent;
    unsafe {
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        let mut br: *mut GpSolidFill = std::ptr::null_mut();
        GdipCreateSolidFill(0, &mut br);
        let mut pen: *mut GpPen = std::ptr::null_mut();
        GdipCreatePen1(0, 1.0, UnitPixel, &mut pen);
        for j in 0..rows {
            for i in 0..cols {
                let (a, b, c, d) = if j & 1 == 0 { (pt(i, j), pt(i + 1, j), pt(i, j + 1), pt(i + 1, j + 1)) } else { (pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i + 2, j + 1)) };
                for (k, tri) in [[a, b, c], [b, d, c]].iter().enumerate() {
                    let v = hash(i, j, 3 + k as u32);
                    // light from the top left: most facets near white, some a touch warmer or greyer
                    let shade = if v < 0.5 { mix(pal().bg, rgb(255, 255, 255), 0.25 + v) } else { mix(pal().bg, rgb(226, 214, 190), (v - 0.5) * 0.45) };
                    GdipSetSolidFillColor(br, argb(shade, 1.0));
                    GdipFillPolygon(g, br as *mut GpBrush, tri.as_ptr(), 3, FillModeAlternate);
                    // facet edges: a hairline of white; one triangle in nine traced in gold
                    let (edge, wd) = if hash(i, j, 7 + k as u32) > 0.84 { (argb(gold, 0.6), 1.0 * scale) } else { (argb(rgb(255, 255, 255), 0.7), 0.6) };
                    GdipSetPenColor(pen, edge);
                    GdipSetPenWidth(pen, wd);
                    GdipDrawPolygon(g, pen, tri.as_ptr(), 3);
                }
            }
        }
        GdipDeletePen(pen);
        GdipDeleteBrush(br as *mut GpBrush);
        GdipDeleteGraphics(g);
    }
}

/// 0..1 from a lattice point (stable between paints).
fn hash(i: i32, j: i32, salt: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B1) ^ (j as u32).wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn font_file(name: &str) -> bool {
    std::env::var_os("WINDIR").is_some_and(|w| PathBuf::from(w).join("Fonts").join(name).is_file())
}

/// The proportional UI font: Windows 11's Segoe UI Variable where it is installed, else Segoe UI.
pub fn sans_face() -> &'static str {
    static F: std::sync::OnceLock<&str> = std::sync::OnceLock::new();
    F.get_or_init(|| if font_file("SegUIVar.ttf") { "Segoe UI Variable Text" } else { "Segoe UI" })
}

/// The UI font of the current style.
pub fn ui_face() -> &'static str {
    match style() {
        s if s.mono() => "Consolas",
        Style::Win98 => "Tahoma",
        _ => sans_face(),
    }
}

/// The icon font: Segoe Fluent Icons (Windows 11, same code points) or Segoe MDL2 Assets.
pub fn icon_face() -> &'static str {
    static F: std::sync::OnceLock<&str> = std::sync::OnceLock::new();
    F.get_or_init(|| if font_file("SegoeIcons.ttf") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_junk_is_the_default() {
        for s in ALL {
            assert_eq!(Style::parse(s.name()), s);
        }
        assert_eq!(Style::parse(" Tiling\r\n"), Style::Tiling);
        assert_eq!(Style::parse(""), Style::Win11);
        assert_eq!(Style::parse("neon"), Style::Win11);
        assert_eq!(PALS.len(), ALL.len());
    }

    #[test]
    fn hash_is_stable_and_in_range() {
        assert_eq!(hash(3, 4, 1), hash(3, 4, 1));
        assert!((0..50).all(|i| (0.0..=1.0).contains(&hash(i, -i, 2))));
        assert_eq!(mix(rgb(0, 0, 0), rgb(200, 100, 50), 0.5), rgb(100, 50, 25));
    }

    #[test]
    fn theme_line_round_trips() {
        let mut c = [u32::MAX; 7];
        c[0] = rgb(1, 2, 3);
        c[4] = rgb(250, 128, 0);
        let t = Theme { name: "Ночь\tтест".into(), style: Style::Ruby, colors: c, opacity: 70 };
        let back = Theme::parse(&t.line()).unwrap();
        assert_eq!(back.name, "Ночь тест");
        assert_eq!((back.style, back.colors, back.opacity), (Style::Ruby, c, 70));
        assert_eq!(Theme::parse("a\tnormal\t5\t").unwrap().opacity, 30);
        assert!(Theme::parse("").is_none() && Theme::parse("x\tnormal").is_none());
        let mut one = [u32::MAX; 7];
        one[0] = rgb(1, 2, 3);
        assert_eq!(parse_colors("bg=010203\nzz=1;text=GG"), one);
    }

    #[test]
    fn side_parses_and_defaults_to_bottom() {
        assert_eq!(Side::parse(" Left
"), Side::Left);
        assert_eq!(Side::parse("right"), Side::Right);
        assert_eq!(Side::parse(""), Side::Bottom);
        assert_eq!(Side::parse("top"), Side::Bottom);
    }
}

fn size_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("bar-size.txt"))
}

static SIZE: AtomicU8 = AtomicU8::new(1);
static SIZE_AT: AtomicU64 = AtomicU64::new(0);

/// Height class of the topbar: 0 small, 1 normal (default), 2 large. Re-read once a second.
pub fn bar_size() -> u8 {
    if now_ms().saturating_sub(SIZE_AT.load(Relaxed)) > 1000 {
        let v = size_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| s.trim().parse::<u8>().ok()).filter(|v| *v <= 2).unwrap_or(1);
        SIZE.store(v, Relaxed);
        SIZE_AT.store(now_ms(), Relaxed);
    }
    SIZE.load(Relaxed)
}

pub fn set_bar_size(v: u8) {
    SIZE.store(v.min(2), Relaxed);
    SIZE_AT.store(now_ms(), Relaxed);
    if let Some(f) = size_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, v.min(2).to_string());
    }
}

fn mon_file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("alttab-monitor.txt"))
}

static MON: AtomicU8 = AtomicU8::new(0);
static MON_AT: AtomicU64 = AtomicU64::new(0);

/// Alt+Tab lists only the windows of the monitor the active window is on (off unless the file says 1). Re-read once a second.
pub fn alttab_per_monitor() -> bool {
    if now_ms().saturating_sub(MON_AT.load(Relaxed)) > 1000 {
        MON.store(mon_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|s| s.trim() == "1") as u8, Relaxed);
        MON_AT.store(now_ms(), Relaxed);
    }
    MON.load(Relaxed) == 1
}

pub fn set_alttab_per_monitor(on: bool) {
    MON.store(on as u8, Relaxed);
    MON_AT.store(now_ms(), Relaxed);
    if let Some(f) = mon_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, if on { "1" } else { "0" });
    }
}
