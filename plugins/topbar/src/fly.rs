// Own flyouts instead of the stock Windows ones: Start (search + all apps + power), Wi-Fi, volume, battery
// (+ brightness), calendar. One popup window; what it shows depends on `Kind`.
use super::*;
use std::sync::Mutex;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicI32, Ordering::Relaxed},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Quick,
    Wifi,
    Cal,
    Start,
    Menu, // right-click menu of the bar
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
    VolSlide,
    BrightSlide,
    Mute,
    WifiSwitch,
    Tile(u8),
    WifiMore,
    Net(usize),
    NetOff, // the connected network's Disconnect button (shown on hover)
    Ctx(u8), // an item of the Wi-Fi panel's right-click menu
    Settings(&'static str),
    Menu(u8),
    CalPrev,
    CalNext,
    CalToday,
    App(usize), // index into the filtered list
    Power(u8),  // 0 lock, 1 sleep, 2 restart, 3 shut down
    Keyboard,   // the on-screen keyboard button in the Start search field
}

thread_local! {
    /// The monitor of the bar that was clicked last, when it is a second monitor's bar (None: the main bar).
    pub(crate) static ORIGIN: Cell<Option<RECT>> = const { Cell::new(None) };
}

/// Where flyouts open: (x offset, y offset from the main bar's place, monitor width) of the clicked bar's monitor.
unsafe fn origin() -> (i32, i32, i32) {
    match ORIGIN.get() {
        Some(m) if !vert() => (m.left, m.bottom - GetSystemMetrics(SM_CYSCREEN) + dock_gap(), m.right - m.left),
        _ => (0, 0, GetSystemMetrics(SM_CXSCREEN)),
    }
}

/// The touch keyboard (TabTip), else the classic on-screen keyboard. Windows opens it by itself only for the Explorer shell.
fn touch_keyboard() {
    let tip = std::env::var_os("CommonProgramFiles").map(|p| std::path::PathBuf::from(p).join(r"microsoft shared\ink\TabTip.exe"));
    let exe = tip.filter(|p| p.is_file()).unwrap_or_else(|| "osk.exe".into());
    let _ = std::process::Command::new(exe).spawn();
}

struct Net {
    ssid: String,
    q: i32,
    secure: bool,
    profile: Option<String>,
    connected: bool,
    auth: i32,
    g: windows_sys::core::GUID, // the adapter that sees it
}

struct Conn {
    ssid: String,
    auth: i32,
    q: i32,
    rx: u32, // Mbps
    tx: u32,
    phy: &'static str,
}

fn auth_name(a: i32) -> &'static str {
    match a {
        1 => t!("Открытая", "Open", ja: "オープン", pt: "Aberta", zh: "开放"),
        2 => "WEP",
        3 => "WPA",
        4 => "WPA‑Personal",
        6 => "WPA2",
        7 => "WPA2‑Personal",
        8 => "WPA3",
        9 => "WPA3‑Personal",
        _ => t!("Защищено", "Secured", ja: "保護あり", pt: "Protegida", zh: "已加密"),
    }
}

fn phy_name(t: i32) -> &'static str {
    match t {
        7 => "Wi‑Fi 4",
        8 => "Wi‑Fi 5",
        10 => "Wi‑Fi 6",
        11 => "Wi‑Fi 7",
        4..=6 => "802.11a/b/g",
        _ => "Wi‑Fi",
    }
}

static APPS: Mutex<Vec<(String, Vec<u16>)>> = Mutex::new(Vec::new()); // Start menu shortcuts: (name, .lnk path)
pub static PREVIEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false); // `--preview`: stays open without focus
static BRIGHT: AtomicI32 = AtomicI32::new(-1); // -1 = no controllable display (desktop PC)
const ROWS: usize = 11;

thread_local! {
    static FLY: Cell<Option<Kind>> = const { Cell::new(None) };
    static FH: Cell<HWND> = const { Cell::new(null_mut()) };
    static DEACT: Cell<(Option<Kind>, u64)> = const { Cell::new((None, 0)) }; // panel closed by losing focus, and when
    static CAL: Cell<(i32, i32)> = const { Cell::new((2026, 1)) };
    static NETS: RefCell<Vec<Net>> = const { RefCell::new(Vec::new()) };
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
    static PASS: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
    static NOTE: RefCell<String> = const { RefCell::new(String::new()) };
    static CONNECTING: RefCell<Option<(String, u64)>> = const { RefCell::new(None) }; // the network being joined and since when: its row animates
    static CTX: RefCell<Option<(String, i32, i32, u64)>> = const { RefCell::new(None) }; // Wi-Fi right-click menu: network (by name: the list reorders), x, y (px), opened at
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static SCROLL: Cell<usize> = const { Cell::new(0) };
    static SELALL: Cell<bool> = const { Cell::new(false) }; // Ctrl+A in the search: the next key replaces the query
    static ICONS: RefCell<HashMap<Vec<u16>, HICON>> = RefCell::new(HashMap::new()); // by path: the app list is reloaded and reordered
    static SEL: Cell<usize> = const { Cell::new(0) }; // Start: the row Enter opens (arrow keys move it)
    static OVER: Cell<Option<Act>> = const { Cell::new(None) };
    static DRAG: Cell<u8> = const { Cell::new(0) }; // 0 none, 1 volume slider, 2 brightness slider
    static MENU_X: Cell<i32> = const { Cell::new(0) };
    static CONFIRM: Cell<i32> = const { Cell::new(-1) };
    static FIN: Cell<(i32, i32)> = const { Cell::new((0, 0)) }; // where the open flyout rests
    static T0: Cell<u64> = const { Cell::new(0) }; // when it started opening
    static JUST: RefCell<(String, u64)> = const { RefCell::new((String::new(), 0)) }; // the app pinned last and when: its row pulses
    static GHOST: Cell<(HWND, u64, (i32, i32))> = const { Cell::new((null_mut(), 0, (0, 0))) }; // the closing flyout: window, start, position
}

/// January..December in the UI language; Japanese and Chinese name the month by its number (see `month_title`).
fn month_name(m: i32) -> &'static str {
    [
        t!("Январь", "January", ja: "1月", pt: "Janeiro", zh: "1月"),
        t!("Февраль", "February", ja: "2月", pt: "Fevereiro", zh: "2月"),
        t!("Март", "March", ja: "3月", pt: "Março", zh: "3月"),
        t!("Апрель", "April", ja: "4月", pt: "Abril", zh: "4月"),
        t!("Май", "May", ja: "5月", pt: "Maio", zh: "5月"),
        t!("Июнь", "June", ja: "6月", pt: "Junho", zh: "6月"),
        t!("Июль", "July", ja: "7月", pt: "Julho", zh: "7月"),
        t!("Август", "August", ja: "8月", pt: "Agosto", zh: "8月"),
        t!("Сентябрь", "September", ja: "9月", pt: "Setembro", zh: "9月"),
        t!("Октябрь", "October", ja: "10月", pt: "Outubro", zh: "10月"),
        t!("Ноябрь", "November", ja: "11月", pt: "Novembro", zh: "11月"),
        t!("Декабрь", "December", ja: "12月", pt: "Dezembro", zh: "12月"),
    ][(m.clamp(1, 12) - 1) as usize]
}

/// The calendar header: "Сентябрь 2026", "2026年9月".
fn month_title(y: i32, m: i32) -> String {
    match winrust_common::lang!() {
        winrust_common::Lang::Ja | winrust_common::Lang::Zh => format!("{y}年{m}月"),
        _ => format!("{} {y}", month_name(m)),
    }
}

/// Weekday initials of the calendar's first row, Monday first.
fn weekday_initials() -> [&'static str; 7] {
    [
        t!("Пн", "Mo", ja: "月", pt: "seg", zh: "一"),
        t!("Вт", "Tu", ja: "火", pt: "ter", zh: "二"),
        t!("Ср", "We", ja: "水", pt: "qua", zh: "三"),
        t!("Чт", "Th", ja: "木", pt: "qui", zh: "四"),
        t!("Пт", "Fr", ja: "金", pt: "sex", zh: "五"),
        t!("Сб", "Sa", ja: "土", pt: "sáb", zh: "六"),
        t!("Вс", "Su", ja: "日", pt: "dom", zh: "日"),
    ]
}

// ---------- data ----------

/// Background: collects the Start menu shortcuts once.
pub fn load_apps() {
    fn walk(dir: &std::path::Path, depth: u8, out: &mut Vec<(String, Vec<u16>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, depth + 1, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !name.to_lowercase().contains("uninstall") {
                    out.push((name, wide(&p.to_string_lossy())));
                }
            }
        }
    }
    let mut v = Vec::new();
    for (var, sub) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
        if let Some(b) = std::env::var_os(var) {
            walk(&PathBuf::from(b).join(sub), 0, &mut v);
        }
    }
    // Store apps (Settings, Calculator, Terminal, Photos...) have no .lnk: the shell's Apps folder lists them
    let have: std::collections::HashSet<String> = v.iter().map(|a| a.0.to_lowercase()).collect();
    for (name, id) in apps_folder() {
        let ok = id.contains('!') || id.to_lowercase().ends_with(".exe") || id.starts_with("Microsoft.Windows."); // apps, not docs / web links
        if ok && !have.contains(&name.to_lowercase()) && !name.to_lowercase().contains("uninstall") {
            v.push((name, wide(&format!(r"shell:AppsFolder\{id}"))));
        }
    }
    v.sort_by_key(|a| a.0.to_lowercase());
    v.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    *APPS.lock().unwrap() = v;
}

/// (display name, app id) of everything in the shell's Apps folder (what the Windows Start menu lists).
pub fn apps_folder() -> Vec<(String, String)> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING},
    };
    let mut out = vec![];
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // this runs on its own thread
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut items = [None];
            let mut got = 0u32;
            if en.Next(&mut items, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(it) = items[0].take() else { break };
            let name = |k: SIGDN| {
                it.GetDisplayName(k).ok().map(|p| {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                })
            };
            if let (Some(n), Some(id)) = (name(SIGDN_NORMALDISPLAY), name(SIGDN_PARENTRELATIVEPARSING)) {
                out.push((n, id));
            }
        }
    }
    out
}

const EN_KEYS: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
const RU_KEYS: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";

/// The same keys typed in the other layout: "сркщьу" -> "chrome", "ntktuhfv" -> "телеграм".
fn flip_layout(q: &str) -> String {
    let (en, ru): (Vec<char>, Vec<char>) = (EN_KEYS.chars().collect(), RU_KEYS.chars().collect());
    q.chars().map(|c| en.iter().position(|&x| x == c).map(|i| ru[i]).or_else(|| ru.iter().position(|&x| x == c).map(|i| en[i])).unwrap_or(c)).collect()
}

/// How well an app name matches the query (both lower-case): 0 the name starts with it, 1 a word does, 2 the word
/// initials do ("vsc" = Visual Studio Code), 3 it is somewhere inside; None = no match.
fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return None;
    }
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if name.starts_with(q) {
        Some(0)
    } else if words.iter().any(|w| w.starts_with(q)) {
        Some(1)
    } else if q.chars().count() >= 2 && initials.starts_with(q) {
        Some(2)
    } else if name.contains(q) {
        Some(3)
    } else {
        None
    }
}

// ---------- apps pinned to the Start panel (empty at first; added from right-click menus) ----------

static PINNED: Mutex<Vec<String>> = Mutex::new(Vec::new()); // paths of .lnk / .exe files, in the order they were added

fn pin_file() -> PathBuf {
    appdata_file("topbar-start.txt")
}

fn pinned_load() {
    *PINNED.lock().unwrap() = std::fs::read_to_string(pin_file()).map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default();
}

fn wstr(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

pub fn start_pinned(path: &str) -> bool {
    PINNED.lock().unwrap().iter().any(|p| p.eq_ignore_ascii_case(path))
}

/// Files dropped on the open Start panel are added to it; the panel stays open and the new row pulses.
unsafe fn on_files(hw: HWND, hd: HDROP) {
    let n = DragQueryFileW(hd, u32::MAX, null_mut(), 0);
    let mut added = vec![];
    for i in 0..n {
        let len = DragQueryFileW(hd, i, null_mut(), 0) as usize;
        let mut b = vec![0u16; len + 1];
        DragQueryFileW(hd, i, b.as_mut_ptr(), b.len() as u32);
        let path = String::from_utf16_lossy(&b[..len]);
        if start_pin(&path) {
            added.push(std::path::Path::new(&path).file_stem().unwrap_or_default().to_string_lossy().to_string());
        }
    }
    DragFinish(hd);
    note(&match added.len() {
        0 => t!("Уже в Пуске", "Already in Start", ja: "すでにスタートにあります", pt: "Já está no Iniciar", zh: "已在开始菜单中").to_string(),
        1 => format!("Added to Start: {}", added[0]),
        n => format!("Added {n} items to Start"),
    });
    KillTimer(hw, 6);
    InvalidateRect(hw, null(), 0);
    force_fg(hw); // active again, so a later click elsewhere closes it as usual
}

/// Adds the app to the Start panel unless it is already there (a drop on the Start button). true = it was added.
pub fn start_pin(path: &str) -> bool {
    if start_pinned(path) {
        return false;
    }
    start_toggle(path);
    JUST.set((path.to_string(), unsafe { now_ms() }));
    true
}

pub fn is_open(k: Kind) -> bool {
    FLY.get() == Some(k)
}

pub fn any_open() -> bool {
    FLY.get().is_some()
}

/// Short status line shown at the bottom of the open panel (Start: "Added to Start: ...").
pub fn note(s: &str) {
    NOTE.set(s.to_string());
}

/// Adds the app to the Start panel, or removes it when it is already there.
pub fn start_toggle(path: &str) {
    let mut p = PINNED.lock().unwrap();
    match p.iter().position(|x| x.eq_ignore_ascii_case(path)) {
        Some(i) => drop(p.remove(i)),
        None => p.push(path.to_string()),
    }
    let _ = std::fs::write(pin_file(), p.join("\n"));
}

static EXE_GEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Programs that have no Start menu shortcut (a loose loliland.exe): asks rust-search for .exe files matching what is typed
/// and adds them to the app list, where the normal ranking picks them up. Debounced; the panel repaints when they arrive.
fn find_loose_exes(hw: HWND) {
    let q = QUERY.with_borrow(|q| q.trim().to_string());
    let gen_ = EXE_GEN.fetch_add(1, Relaxed) + 1;
    let rs = PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default()).join(r"rs-search\rs.exe");
    if q.chars().count() < 2 || !rs.is_file() {
        return;
    }
    let hw = hw as isize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if EXE_GEN.load(Relaxed) != gen_ {
            return; // typed on: a newer search is coming
        }
        // straight to the service's pipe: no rs.exe process per query (and no full disk scan when the service is down)
        let found = winrust_common::search::ask_all(&q);
        let mut apps = APPS.lock().unwrap();
        let mut added = false;
        for line in found.iter().filter(|l| l.to_lowercase().ends_with(".exe")).take(12) {
            let w = wide(line);
            if apps.iter().any(|a| a.1 == w) {
                continue;
            }
            let name = std::path::Path::new(line).file_stem().unwrap_or_default().to_string_lossy().to_string();
            apps.push((name, w));
            added = true;
        }
        drop(apps);
        if added {
            unsafe { InvalidateRect(hw as HWND, null(), 0) };
        }
    });
}

fn filtered() -> Vec<usize> {
    let q = QUERY.with_borrow(|q| q.to_lowercase());
    let mut apps = APPS.lock().unwrap();
    if q.is_empty() {
        let pinned = PINNED.lock().unwrap();
        // nothing pinned yet: every app, A to Z, like Windows' "All apps" (an empty Start read as "no apps found", #72)
        if pinned.is_empty() {
            return (0..apps.len()).collect();
        }
        // nothing typed: only what the user pinned (an app that is not a Start menu shortcut joins the list on demand)
        return pinned
            .iter()
            .map(|p| {
                let w = wide(p);
                apps.iter().position(|a| a.1 == w).unwrap_or_else(|| {
                    let stem = std::path::Path::new(p).file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let name: String = stem.chars().take(1).flat_map(char::to_uppercase).chain(stem.chars().skip(1)).collect();
                    apps.push((name, w));
                    apps.len() - 1
                })
            })
            .collect();
    }
    // typed in the wrong layout: those matches come after the direct ones
    let fq = flip_layout(&q);
    let mut v: Vec<(u8, u32, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let n = a.0.to_lowercase();
            let s = score(&n, &q).or_else(|| score(&n, &fq).map(|s| s + 4))?;
            Some((s, u32::MAX - uses(&n), i))
        })
        .collect();
    v.sort_unstable(); // best match, then most launched, then alphabetical (the list is sorted by name)
    v.into_iter().map(|x| x.2).collect()
}

/// The built-in panel through the video driver (`\\.\LCD`, what Windows' own slider talks to): needs no admin rights and no
/// PowerShell. IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS 0x230498 answers (policy, AC %, DC %); IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS
/// 0x23049C takes the same three bytes (policy 3: both). No panel (a desktop PC) = the open fails.
fn lcd_io(code: u32, input: Option<[u8; 3]>) -> Option<[u8; 3]> {
    use windows_sys::Win32::{
        Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
        System::IO::DeviceIoControl,
    };
    unsafe {
        let h = CreateFileW(wide(r"\\.\LCD").as_ptr(), 0xC000_0000, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let (mut out, mut n) = ([0u8; 3], 0u32);
        let ok = match input {
            Some(i) => DeviceIoControl(h, code, i.as_ptr().cast(), 3, null_mut(), 0, &mut n, null_mut()),
            None => DeviceIoControl(h, code, null(), 0, out.as_mut_ptr().cast(), 3, &mut n, null_mut()),
        } != 0;
        CloseHandle(h);
        ok.then_some(out)
    }
}

fn read_brightness() {
    std::thread::spawn(|| {
        let ac = unsafe {
            let mut s: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = zeroed();
            windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut s) != 0 && s.ACLineStatus == 1
        };
        let v = lcd_io(0x23_0498, None).map(|b| if ac { b[1] } else { b[2] } as i32).filter(|v| *v <= 100).or_else(|| {
            // no answer from the driver: WMI (some panels only show up there)
            let o = Command::new("powershell").args(["-NoProfile", "-Command", "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue).CurrentBrightness"]).creation_flags(NO_WINDOW).output();
            o.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        });
        BRIGHT.store(v.unwrap_or(-1), Relaxed);
    });
}

fn set_brightness(v: i32) {
    std::thread::spawn(move || {
        let b = v.clamp(0, 100) as u8;
        if lcd_io(0x23_049C, Some([3, b, b])).is_none() {
            let c = format!("Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=1;Brightness={v}}}");
            let _ = Command::new("powershell").args(["-NoProfile", "-Command", &c]).creation_flags(NO_WINDOW).output();
        }
    });
}

/// Runs `f` for every Wi-Fi adapter with one open WLAN handle.
unsafe fn for_each_wlan(mut f: impl FnMut(HANDLE, &windows_sys::core::GUID)) {
    let (mut h, mut ver) = (null_mut(), 0u32);
    if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
        return;
    }
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    if WlanEnumInterfaces(h, null(), &mut list) == 0 {
        let l = &*list;
        for i in 0..l.dwNumberOfItems as usize {
            f(h, &(*l.InterfaceInfo.as_ptr().add(i)).InterfaceGuid);
        }
        WlanFreeMemory(list as *const _);
    }
    WlanCloseHandle(h, null());
}

/// Runs `f` with an open WLAN handle for the adapter `g`.
unsafe fn with_iface<T>(g: &windows_sys::core::GUID, f: impl FnOnce(HANDLE, &windows_sys::core::GUID) -> T) -> Option<T> {
    let (mut h, mut ver) = (null_mut(), 0u32);
    if WlanOpenHandle(2, null(), &mut ver, &mut h) != 0 {
        return None;
    }
    let r = f(h, g);
    WlanCloseHandle(h, null());
    Some(r)
}

/// The visible networks and the current connection of one adapter.
unsafe fn read_one(h: HANDLE, g: &windows_sys::core::GUID) -> (Vec<Net>, Option<Conn>) {
    {
        let mut out: Vec<Net> = Vec::new();
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
        if WlanGetAvailableNetworkList(h, g, 0, null(), &mut list) == 0 {
            let l = &*list;
            for i in 0..l.dwNumberOfItems as usize {
                let n = &*l.Network.as_ptr().add(i);
                let ssid = String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..n.dot11Ssid.uSSIDLength as usize]).to_string();
                if ssid.is_empty() {
                    continue;
                }
                let pn = String::from_utf16_lossy(&n.strProfileName[..n.strProfileName.iter().position(|&c| c == 0).unwrap_or(0)]);
                let net = Net { ssid, q: n.wlanSignalQuality as i32, secure: n.bSecurityEnabled != 0, profile: (n.dwFlags & 2 != 0 && !pn.is_empty()).then_some(pn), connected: n.dwFlags & 1 != 0, auth: n.dot11DefaultAuthAlgorithm as i32, g: *g };
                match out.iter_mut().find(|o| o.ssid == net.ssid) {
                    Some(o) if o.q < net.q => *o = net,
                    Some(_) => {}
                    None => out.push(net),
                }
            }
            WlanFreeMemory(list as *const _);
        }
        out.sort_by_key(|n| (!n.connected, -n.q));
        let (mut size, mut data) = (0u32, null_mut());
        let conn = (WlanQueryInterface(h, g, wlan_intf_opcode_current_connection, null(), &mut size, &mut data, null_mut()) == 0).then(|| {
            let at = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let a = &at.wlanAssociationAttributes;
            let ssid = String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..a.dot11Ssid.uSSIDLength as usize]).to_string();
            let c = Conn { ssid, auth: at.wlanSecurityAttributes.dot11AuthAlgorithm as i32, q: a.wlanSignalQuality as i32, rx: a.ulRxRate / 1000, tx: a.ulTxRate / 1000, phy: phy_name(a.dot11PhyType as i32) };
            WlanFreeMemory(data);
            c
        });
        // the network we are on can be missing from the scan list (hidden / not rescanned yet): show it anyway
        if let Some(c) = &conn {
            match out.iter().position(|n| n.ssid == c.ssid) {
                Some(i) => {
                    out[i].connected = true;
                    out.swap(0, i);
                }
                None => out.insert(0, Net { ssid: c.ssid.clone(), q: c.q, secure: c.auth != 1, profile: Some(c.ssid.clone()), connected: true, auth: c.auth, g: *g }),
            }
        }
        (out, conn)
    }
}

unsafe fn read_nets() {
    let mut nets: Vec<Net> = vec![];
    let mut conn: Option<Conn> = None;
    for_each_wlan(|h, g| {
        let (o, c) = read_one(h, g);
        for n in o {
            // one entry per network: the adapter that is connected to it, else the one with the better signal
            match nets.iter_mut().find(|x| x.ssid == n.ssid) {
                Some(x) if (n.connected && !x.connected) || (!x.connected && x.q < n.q) => *x = n,
                Some(_) => {}
                None => nets.push(n),
            }
        }
        if conn.is_none() {
            conn = c;
        }
    });
    nets.sort_by_key(|n| (!n.connected, -n.q));
    nets.truncate(7);
    NETS.set(nets);
    CONN.set(conn);
    // the joining animation ends when the network shows up as connected, or gives up after 25 s
    let on = NETS.with_borrow(|n| n.iter().find(|n| n.connected).map(|n| n.ssid.clone()));
    if let Some((ssid, t0)) = CONNECTING.with_borrow(|c| c.clone()) {
        if on.as_deref() == Some(ssid.as_str()) {
            CONNECTING.set(None);
            NOTE.set(String::new());
        } else if now_ms().saturating_sub(t0) > 25_000 {
            CONNECTING.set(None);
            NOTE.set(tf!("Не удалось подключиться к «{ssid}»", "Could not connect to “{ssid}”", ja: "「{ssid}」に接続できませんでした", pt: "Não foi possível conectar a “{ssid}”", zh: "无法连接到“{ssid}”"));
        }
    }
}

unsafe fn start_connecting(ssid: &str) {
    CONNECTING.set(Some((ssid.to_string(), now_ms())));
    NOTE.set(String::new());
}

/// Deletes the saved profile of `ssid` (Windows drops the connection if it is the current one).
unsafe fn forget(ssid: &str) {
    let Some(p) = NETS.with_borrow(|n| n.iter().find(|n| n.ssid == ssid).and_then(|n| n.profile.clone())) else { return };
    let w = wide(&p);
    let mut ok = false;
    for_each_wlan(|h, g| ok |= WlanDeleteProfile(h, g, w.as_ptr(), null()) == 0);
    NOTE.set(if ok { tf!("Сеть «{ssid}» забыта", "Forgot “{ssid}”", ja: "「{ssid}」を削除しました", pt: "“{ssid}” esquecida", zh: "已忘记“{ssid}”") } else { tf!("Не удалось забыть «{ssid}»", "Could not forget “{ssid}”", ja: "「{ssid}」を削除できませんでした", pt: "Não foi possível esquecer “{ssid}”", zh: "无法忘记“{ssid}”") });
    read_nets();
}

unsafe fn wifi_connect(profile: &str, g: &windows_sys::core::GUID) {
    let p = wide(profile);
    with_iface(g, |h, g| {
        let params = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: p.as_ptr(), pDot11Ssid: null_mut(), pDesiredBssidList: null_mut(), dot11BssType: dot11_BSS_type_any, dwFlags: 0 };
        WlanConnect(h, g, &params, null());
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Saves a profile for a new network and connects. shortcut: only open and WPA/WPA2/WPA3 personal (password) networks; enterprise ones go to Windows settings.
unsafe fn wifi_join(n: &Net, pass: &str) {
    let (auth, key) = match n.auth {
        1 => ("open", String::new()),
        4 => ("WPAPSK", pass.into()),
        7 => ("WPA2PSK", pass.into()),
        _ => ("WPA3SAE", pass.into()),
    };
    let sec = if auth == "open" {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        format!("<authEncryption><authentication>{auth}</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>passphrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", esc(&key))
    };
    let name = esc(&n.ssid);
    let xml = format!("<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name><SSIDConfig><SSID><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM><security>{sec}</security></MSM></WLANProfile>");
    let x = wide(&xml);
    let mut reason = 0u32;
    let ok = with_iface(&n.g, |h, g| WlanSetProfile(h, g, 0, x.as_ptr(), null(), 1, null_mut(), &mut reason) == 0).unwrap_or(false);
    if ok {
        wifi_connect(&n.ssid, &n.g);
        start_connecting(&n.ssid);
    } else {
        NOTE.set(t!("Не удалось сохранить сеть (неверный формат пароля?)", "Could not save the network (wrong password format?)", ja: "ネットワークを保存できませんでした (パスワードの形式が違いませんか?)", pt: "Não foi possível salvar a rede (formato de senha errado?)", zh: "无法保存网络（密码格式有误？）").into());
    }
}

// ---------- geometry ----------

fn size(k: Kind) -> (i32, i32) {
    match k {
        Kind::Quick => (360, 252 + 44 * sliders()),
        Kind::Cal => (330, 340),
        Kind::Wifi => (360, wifi_rows().2),
        Kind::Start => (560, 640),
        Kind::Menu => (MW, menu_rows().1),
    }
}

/// Wi-Fi panel rows in logical px: (top, bottom) per network (the connected one is a taller card),
/// y of the "Other networks" label, and the panel height.
fn wifi_rows() -> (Vec<(i32, i32)>, Option<i32>, i32) {
    NETS.with_borrow(|n| {
        let (mut y, mut rows, mut label) = (64, vec![], None);
        for net in n.iter() {
            if net.connected {
                rows.push((y, y + 60));
                y += 74;
            } else {
                if label.is_none() {
                    label = Some(y);
                    y += 26;
                }
                rows.push((y, y + 46));
                y += 46;
            }
        }
        (rows, label, y + if n.is_empty() { 70 } else { 10 } + 44)
    })
}

/// Quick settings: how many slider rows (volume, plus brightness on a laptop).
fn sliders() -> i32 {
    if BRIGHT.load(Relaxed) >= 0 { 2 } else { 1 }
}

const CTX_W: i32 = 196; // right-click menu width, logical px
const CTX_ROW: i32 = 34;

/// The right-click menu's items for a network: (label, what it does: 0 connect, 1 disconnect, 2 forget, 3 known networks).
fn ctx_items(ssid: &str) -> Vec<(&'static str, u8)> {
    NETS.with_borrow(|n| {
        let Some(n) = n.iter().find(|n| n.ssid == ssid) else { return vec![] };
        let mut v = vec![if n.connected { (t!("Отключиться", "Disconnect", ja: "切断", pt: "Desconectar", zh: "断开连接"), 1) } else { (t!("Подключиться", "Connect", ja: "接続", pt: "Conectar", zh: "连接"), 0) }];
        if n.profile.is_some() {
            v.push((t!("Забыть сеть", "Forget network", ja: "ネットワークを削除", pt: "Esquecer rede", zh: "忘记网络"), 2));
        }
        v.push((t!("Известные сети…", "Known networks…", ja: "既知のネットワーク…", pt: "Redes conhecidas…", zh: "已知网络…"), 3));
        v
    })
}

fn ctx_rects() -> Vec<(RECT, Act)> {
    CTX.with_borrow(|c| match c {
        Some((ssid, x, y, _)) => (0..ctx_items(ssid).len() as i32).map(|j| (RECT { left: x + sc(6), top: y + sc(6 + CTX_ROW * j), right: x + sc(CTX_W - 6), bottom: y + sc(6 + CTX_ROW * (j + 1)) }, Act::Ctx(j as u8))).collect(),
        None => vec![],
    })
}

fn r(l: i32, t: i32, rr: i32, b: i32) -> RECT {
    RECT { left: sc(l), top: sc(t), right: sc(rr), bottom: sc(b) }
}

fn rects(k: Kind) -> Vec<(RECT, Act)> {
    let (w, h) = size(k);
    match k {
        Kind::Quick => {
            // a 3x2 grid of tiles; the Wi-Fi tile has a round "more" button in its corner (found first: act_at searches backwards)
            let mut v = vec![];
            for i in 0..6u8 {
                let (tl, tt) = (16 + 112 * (i as i32 % 3), 16 + 84 * (i as i32 / 3));
                v.push((r(tl, tt, tl + 104, tt + 76), Act::Tile(i)));
                if i == 0 {
                    v.push((r(tl + 66, tt + 8, tl + 96, tt + 38), Act::WifiMore));
                }
            }
            let mut y = 188;
            if BRIGHT.load(Relaxed) >= 0 {
                v.push((r(64, y, w - 24, y + 40), Act::BrightSlide));
                y += 44;
            }
            v.push((r(24, y, 60, y + 40), Act::Mute));
            v.push((r(64, y, w - 24, y + 40), Act::VolSlide));
            v.push((r(w - 54, h - 50, w - 14, h - 10), Act::Settings("ms-settings:")));
            v
        }
        Kind::Cal => vec![(r(w - 84, 10, w - 52, 42), Act::CalPrev), (r(w - 48, 10, w - 16, 42), Act::CalNext), (r(16, 10, w - 100, 42), Act::CalToday)],
        Kind::Wifi => {
            let rows = wifi_rows().0;
            let mut v: Vec<_> = rows.iter().enumerate().map(|(i, &(t, b))| (r(12, t, w - 12, b), Act::Net(i))).collect();
            if let Some(i) = NETS.with_borrow(|n| n.iter().position(|n| n.connected)) {
                let (t, b) = rows[i];
                v.push((r(w - 12 - 108, t + 14, w - 24, b - 14), Act::NetOff));
            }
            v.push((r(w - 58, 18, w - 16, 42), Act::WifiSwitch));
            v.push((r(12, h - 44, w - 12, h - 8), Act::Settings("ms-settings:network-wifi")));
            v.extend(ctx_rects()); // last: on top of the rows
            v
        }
        Kind::Menu => {
            let heads: Vec<bool> = MITEMS.with_borrow(|m| m.iter().map(|it| it.head).collect());
            menu_rows().0.into_iter().enumerate().filter(|(i, _)| !heads[*i]).map(|(i, (t, b))| (r(6, t, w - 6, b), Act::Menu(i as u8))).collect()
        }
        Kind::Start => {
            let f = filtered();
            let s = SCROLL.get();
            let mut v: Vec<_> = (0..ROWS).filter(|i| s + i < f.len()).map(|i| (r(12, 76 + 40 * i as i32, w - 12, 76 + 40 * (i as i32 + 1)), Act::App(s + i))).collect();
            v.extend((0..4u8).map(|i| (r(w - 16 - 4 * 44 + 44 * i as i32, h - 52, w - 16 - 4 * 44 + 44 * (i as i32 + 1) - 4, h - 12), Act::Power(i))));
            v.push((r(w - 58, 24, w - 26, 52), Act::Keyboard)); // inside the search field, at its right end
            v
        }
    }
}

fn act_at(k: Kind, x: i32, y: i32) -> Option<(RECT, Act)> {
    rects(k).into_iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

// ---------- open / close ----------

pub fn init(hi: HINSTANCE) {
    pinned_load();
    unsafe {
        let class = wide("topbar_fly");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hi, hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: class.as_ptr(), ..zeroed() };
        RegisterClassW(&wc);
    }
}

/// One row of a menu flyout: a label (with a window's icon when it stands for a window), a dim keyboard hint on the right,
/// a dot when it is the current choice, red when it destroys something, a separator line above it when `sep`.
pub struct MItem {
    pub icon: HICON,
    pub label: String,
    pub hint: &'static str,
    pub on: bool,
    pub danger: bool,
    pub sep: bool,
    pub head: bool,  // a section caption: small dim text, not clickable
    pub beta: bool,  // a red BETA at the right end
    pub swatch: u32, // a colour dot before the label (COLORREF), u32::MAX = none
}

impl MItem {
    pub fn head(label: impl Into<String>) -> Self {
        MItem { head: true, ..MItem::new(label) }
    }
    pub fn beta(mut self, on: bool) -> Self {
        self.beta = on;
        self
    }
    pub fn swatch(mut self, c: u32) -> Self {
        self.swatch = c;
        self
    }
    pub fn new(label: impl Into<String>) -> Self {
        MItem { icon: null_mut(), label: label.into(), hint: "", on: false, danger: false, sep: false, head: false, beta: false, swatch: u32::MAX }
    }
    pub fn hint(mut self, h: &'static str) -> Self {
        self.hint = h;
        self
    }
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn sep(mut self) -> Self {
        self.sep = true;
        self
    }
}

type Pick = Box<dyn FnOnce(usize)>;
thread_local! {
    static MITEMS: RefCell<Vec<MItem>> = const { RefCell::new(Vec::new()) };
    static MHEAD: RefCell<Option<(HICON, &'static str, String, String)>> = const { RefCell::new(None) }; // icon (or a glyph when null), title, subtitle
    static MPICK: RefCell<Option<Pick>> = const { RefCell::new(None) };
}
const MW: i32 = 272; // menu width
const MROW: i32 = 38;
const MHEAD_H: i32 = 58;

/// Menu flyout above the bar, centered on screen x `cx`. `pick` gets the index of the chosen item (not called on cancel).
/// A right click while a menu is open replaces it.
pub fn menu(cx: i32, head: Option<(HICON, &'static str, String, String)>, items: Vec<MItem>, pick: impl FnOnce(usize) + 'static) {
    if is_open(Kind::Menu) {
        close_now();
    }
    MITEMS.set(items);
    MHEAD.set(head);
    MPICK.set(Some(Box::new(pick)));
    MENU_X.set(if vert() { cx } else { cx - sc(MW) / 2 }); // along a side bar it holds the anchor y
    toggle(Kind::Menu);
}

/// (top, bottom) of each menu item in logical px, and the panel height.
fn menu_rows() -> (Vec<(i32, i32)>, i32) {
    let mut y = if MHEAD.with_borrow(|h| h.is_some()) { MHEAD_H + 9 } else { 6 };
    let rows = MITEMS.with_borrow(|m| {
        m.iter()
            .map(|it| {
                if it.sep {
                    y += 9;
                }
                let h = if it.head { 28 } else { MROW };
                y += h;
                (y - h, y)
            })
            .collect()
    });
    (rows, y + 6)
}

/// Right-click menu of an empty spot of the bar.
pub fn context_menu(x: i32) {
    let pu = crate::purge_installed();
    let mut items = vec![
        MItem::new(t!("Параметры панели задач", "Taskbar settings", ja: "タスク バーの設定", pt: "Configurações da barra de tarefas", zh: "任务栏设置")).sep(),
        MItem::new(t!("Параметры Windows", "Windows Settings", ja: "Windows の設定", pt: "Configurações do Windows", zh: "Windows 设置")).hint("Win+I"),
        MItem::new(t!("Персонализация", "Personalization", ja: "個人用設定", pt: "Personalização", zh: "个性化")),
        MItem::new(t!("Диспетчер задач", "Task Manager", ja: "タスク マネージャー", pt: "Gerenciador de Tarefas", zh: "任务管理器")).hint("Ctrl+Shift+Esc").sep(),
        MItem::new(t!("Чистильщик диска", "Disk Cleaner", ja: "ディスク クリーナー", pt: "Limpador de Disco", zh: "磁盘清理")),
        MItem::new(t!("Закрыть панель", "Close the bar", ja: "バーを閉じる", pt: "Fechar a barra", zh: "关闭任务栏")).danger().sep(),
    ];
    if pu {
        items.insert(5, MItem::new(t!("Удалить мусор Windows (необратимо)", "Remove Windows junk (irreversible)", ja: "Windows の不要なものを削除 (元に戻せません)", pt: "Remover lixo do Windows (irreversível)", zh: "删除 Windows 垃圾文件（不可恢复）")).danger());
    }
    menu(x, None, items, move |i| unsafe {
        let i = if pu && i == 5 { 99 } else { i };
        match i {
            0 => crate::settings::style_menu(),
            1 => open(&wide("ms-settings:")),
            2 => open(&wide("ms-settings:personalization")),
            // with rtaskmgr-win32 installed this is our task manager; a full path because explorer.exe does not search PATH
            3 => open(&wide(&format!(r"{}\System32\taskmgr.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())))),
            4 => open_cleaner(),
            99 => crate::open_purge(),
            _ => _ = PostMessageW(FindWindowW(wide("topbar").as_ptr(), null()), WM_CLOSE, 0, 0), // quits the bar, brings the Windows taskbar back
        }
    });
}

/// Starts the fade-out of the open flyout (it stays as a ghost window until the fade ends).
pub fn close() {
    let h = FH.replace(null_mut());
    FLY.set(None);
    if h.is_null() {
        return;
    }
    unsafe {
        kill_ghost();
        GHOST.set((h, now_ms(), FIN.get()));
        KillTimer(h, 1);
        KillTimer(h, 2);
        SetTimer(h, 3, 15, None);
    }
}

/// Closes without the fade (another flyout replaces it, or a fullscreen app needs the screen).
pub fn close_now() {
    close();
    unsafe { kill_ghost() };
}

unsafe fn kill_ghost() {
    let (g, _, _) = GHOST.replace((null_mut(), 0, (0, 0)));
    if !g.is_null() {
        DestroyWindow(g);
    }
}

/// One frame of a flyout fade: opening rises up from the bar while fading in, closing fades out while sinking back down.
unsafe fn anim_step(hw: HWND) {
    let now = now_ms();
    let (g, gt0, gpos) = GHOST.get();
    if hw == g {
        let t = (now.saturating_sub(gt0) as f32 / 150.0).min(1.0);
        let q = t * t;
        SetLayeredWindowAttributes(hw, 0, ((1.0 - q) * 255.0) as u8, LWA_ALPHA);
        SetWindowPos(hw, null_mut(), gpos.0, gpos.1 + (q * sc(10) as f32) as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        if t >= 1.0 {
            kill_ghost();
        }
    } else if hw == FH.get() {
        let t = (now.saturating_sub(T0.get()) as f32 / 220.0).min(1.0);
        let p = 1.0 - (1.0 - t).powi(3);
        SetLayeredWindowAttributes(hw, 0, (p * 255.0) as u8, LWA_ALPHA);
        let (x, y) = FIN.get();
        SetWindowPos(hw, null_mut(), x, y + ((1.0 - p) * sc(14) as f32) as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        if t >= 1.0 {
            KillTimer(hw, 3);
        }
    } else {
        KillTimer(hw, 3);
    }
}

/// toggle() for a click on the panel's own bar button. The click can deactivate the panel first (closing it), and a
/// plain toggle then opened it again with its animation (#140): a panel of this kind closed that way just now stays closed.
pub fn toggle_click(k: Kind) {
    let (dk, t) = DEACT.replace((None, 0));
    if dk == Some(k) && unsafe { now_ms() }.saturating_sub(t) < 400 {
        return;
    }
    toggle(k);
}

pub fn toggle(k: Kind) {
    // The panel counts as open only while its window is alive: Alt+F4 (or anything else destroying the window behind our
    // back) leaves the state saying "open", and then every Win press would only "close" a dead window and never open a new
    // one — the button looks dead until the bar restarts.
    if FLY.get() == Some(k) && unsafe { IsWindow(FH.get()) } != 0 {
        return close();
    }
    // No debounce here: a Win press right after the panel closed (or while it is still fading out) must open it. The bar is
    // WS_EX_NOACTIVATE, so the click that closes the panel never comes back as a second toggle of its own.
    close();
    unsafe {
        HV.with_borrow_mut(|m| m.retain(|k, _| *k < 1_000_000)); // flyout keys are >= 1_000_000: no stale glow from the last panel
        CTX.set(None);
        QUERY.set(String::new());
        SELALL.set(false);
        SCROLL.set(0);
        SEL.set(0);
        PASS.set(None);
        NOTE.set(String::new());
        CONFIRM.set(-1);
        OVER.set(None);
        let mut t: SYSTEMTIME = zeroed();
        GetLocalTime(&mut t);
        CAL.set((t.wYear as i32, t.wMonth as i32));
        match k {
            Kind::Wifi => {
                for_each_wlan(|h, g| _ = WlanScan(h, g, null(), null(), null()));
                read_nets();
            }
            Kind::Quick => {
                read_brightness();
                read_nets();
                crate::quick::refresh();
            }
            Kind::Start => {
                pinned_load(); // the search card (rs-launcher) can add to this list from another process
                std::thread::spawn(load_apps); // apps installed since the last open show up (the panel repaints every second)
            }
            _ => {}
        }
        let (w, h) = (sc(size(k).0), sc(size(k).1));
        let (ox, oy, sw) = origin();
        let x = if vert() { side_x(w) } else { ox + match k {
            Kind::Start => (sw - w) / 2,
            Kind::Menu => MENU_X.get().min(sw - w - sc(8)).max(sc(8)),
            _ => sw - w - sc(8),
        } };
        let class = wide("topbar_fly");
        let fy = if vert() && k == Kind::Menu { (MENU_X.get() - h / 2).clamp(sc(8), GetSystemMetrics(SM_CYSCREEN) - h - sc(8)) } else { pop_y(h) + oy }; // above the bar (below it at the top of the screen)
        let hw = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, class.as_ptr(), null(), WS_POPUP, x, fy + sc(14), w, h, null_mut(), null_mut(), GetModuleHandleW(null()), null());
        SetLayeredWindowAttributes(hw, 0, 0, LWA_ALPHA); // starts invisible, anim_step fades it in
        if k == Kind::Start {
            // accept file drops (an app dragged onto the panel is added to Start); the bar may be elevated, so lift UIPI for them
            DragAcceptFiles(hw, 1);
            for m in [WM_DROPFILES, 0x49 /* WM_COPYGLOBALDATA */, WM_COPYDATA] {
                ChangeWindowMessageFilterEx(hw, m, MSGFLT_ALLOW, null_mut());
            }
        }
        FIN.set((x, fy));
        T0.set(now_ms());
        SetWindowRgn(hw, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
        FH.set(hw);
        FLY.set(Some(k));
        ShowWindow(hw, SW_SHOW);
        force_fg(hw);
        SetTimer(hw, 1, 1000, None);
        SetTimer(hw, 3, 15, None);
    }
}

// ---------- actions ----------

unsafe fn slide(a: Act, x: i32, rect: RECT) {
    let f = ((x - rect.left - sc(8)) as f32 / (rect.right - rect.left - sc(16)) as f32).clamp(0.0, 1.0);
    if a == Act::VolSlide {
        set_volume((f * 100.0).round() as i32);
    } else {
        BRIGHT.store(((f * 100.0) as i32).max(1), Relaxed);
    }
}

/// Wi-Fi radio on/off (the Wi-Fi tile), software radio of every adapter (first PHY of each).
unsafe fn wifi_radio(on: bool) {
    crate::quick::set_wifi(on, move || unsafe { wifi_radio_wlan(on) });
}

/// The adapter's own software radio switch (the fallback): it cannot undo what Windows' radio manager switched off.
unsafe fn wifi_radio_wlan(on: bool) {
    for_each_wlan(|h, g| {
        let s = WLAN_PHY_RADIO_STATE { dwPhyIndex: 0, dot11SoftwareRadioState: if on { dot11_radio_state_on } else { dot11_radio_state_off }, dot11HardwareRadioState: dot11_radio_state_on };
        WlanSetInterface(h, g, wlan_intf_opcode_radio_state, size_of::<WLAN_PHY_RADIO_STATE>() as u32, &s as *const _ as *const _, null());
    });
}

/// Starts the i-th app of the filtered Start list and counts the launch (search ranks by it). false = no such row.
unsafe fn launch(i: usize) -> bool {
    let Some(app) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap().get(ai).cloned()) else { return false };
    if wstr(&app.1).starts_with("shell:") {
        bump_name(&app.0); // open() counts only .lnk files
    }
    open(&app.1);
    true
}

/// Right-click menu of a Start app: open, open as administrator, show the shortcut's folder.
unsafe fn app_menu(hw: HWND, i: usize, x: i32, y: i32) {
    let Some(lnk) = filtered().get(i).and_then(|&ai| APPS.lock().unwrap().get(ai).map(|a| a.1.clone())) else { return };
    let m = CreatePopupMenu();
    let path = wstr(&lnk);
    let file = !path.starts_with("shell:"); // Store apps have no file to run elevated or to show in a folder
    let pin_label = if start_pinned(&path) { t!("Убрать из Пуска", "Remove from Start", ja: "スタートから削除", pt: "Remover do Iniciar", zh: "从开始菜单移除") } else { t!("Добавить в Пуск", "Add to Start", ja: "スタートに追加", pt: "Adicionar ao Iniciar", zh: "添加到开始菜单") };
    let menu = [
        (1, t!("Открыть", "Open", ja: "開く", pt: "Abrir", zh: "打开")),
        (2, t!("Запуск от имени администратора", "Run as administrator", ja: "管理者として実行", pt: "Executar como administrador", zh: "以管理员身份运行")),
        (3, t!("Расположение файла", "Open file location", ja: "ファイルの場所を開く", pt: "Abrir local do arquivo", zh: "打开文件所在位置")),
        (4, pin_label),
    ];
    for (id, label) in menu {
        if file || id == 1 || id == 4 {
            AppendMenuW(m, MF_STRING, id, wide(label).as_ptr());
        }
    }
    let mut pt = POINT { x, y };
    ClientToScreen(hw, &mut pt);
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hw, null());
    DestroyMenu(m);
    match cmd {
        1 => _ = launch(i),
        2 => {
            ShellExecuteW(null_mut(), wide("runas").as_ptr(), lnk.as_ptr(), null(), null(), SW_SHOWNORMAL);
        }
        3 => {
            let p = String::from_utf16_lossy(&lnk[..lnk.len().saturating_sub(1)]);
            winrust_common::reveal(&p);
        }
        4 => {
            start_toggle(&path);
            return; // stay open, the list just changed
        }
        _ => return,
    }
    close();
}

unsafe fn run_act(a: Act, rect: RECT, x: i32) {
    match a {
        Act::VolSlide | Act::BrightSlide => {
            DRAG.set(if a == Act::VolSlide { 1 } else { 2 });
            SetCapture(FH.get());
            slide(a, x, rect);
        }
        Act::WifiMore => toggle(Kind::Wifi),
        Act::WifiSwitch => wifi_radio(WIFI.get() < 0 && NETS.with_borrow(|n| n.is_empty())),
        Act::Tile(i) => match i {
            0 => wifi_radio(WIFI.get() < 0),
            5 => {
                close();
                keys(&[VK_LWIN, VK_CONTROL, b'L' as u16]); // live captions
            }
            1 => match crate::quick::bluetooth() {
                Some(on) => crate::quick::set(false, !on),
                None => {
                    // no answer from the radio list (still reading, or Windows hides it from us): do not sit silent, show Windows' page
                    crate::quick::refresh();
                    NOTE.set(t!("Нет данных о Bluetooth, открываю параметры", "No Bluetooth state, opening Settings", ja: "Bluetooth の状態を取得できません。設定を開きます", pt: "Sem estado do Bluetooth, abrindo as Configurações", zh: "无法获取蓝牙状态，正在打开设置").into());
                    open(&wide("ms-settings:bluetooth"));
                    close();
                }
            },
            2 => crate::quick::set(true, !crate::quick::airplane().unwrap_or(false)),
            3 => {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                let (nar, mag) = (crate::quick::running("Narrator.exe"), crate::quick::running("Magnify.exe"));
                let (sticky, filter) = (crate::quick::sticky_keys(), crate::quick::filter_keys());
                let items = vec![
                    MItem::new(t!("Экранный диктор", "Narrator", ja: "ナレーター", pt: "Narrador", zh: "讲述人")).hint("Win+Ctrl+Enter").on(nar),
                    MItem::new(t!("Экранная лупа", "Magnifier", ja: "拡大鏡", pt: "Lupa", zh: "放大镜")).hint("Win +/Esc").on(mag),
                    MItem::new(t!("Залипание клавиш", "Sticky keys", ja: "固定キー機能", pt: "Teclas de Aderência", zh: "粘滞键")).on(sticky).sep(),
                    MItem::new(t!("Фильтрация ввода", "Filter keys", ja: "フィルター キー機能", pt: "Teclas de Filtro", zh: "筛选键")).on(filter),
                ];
                menu(pt.x, None, items, move |i| match i {
                    0 => keys(&[VK_LWIN, VK_CONTROL, VK_RETURN]),
                    1 => keys(if mag { &[VK_LWIN, VK_ESCAPE] } else { &[VK_LWIN, VK_OEM_PLUS] }),
                    2 => crate::quick::set_keys(false, !sticky),
                    _ => crate::quick::set_keys(true, !filter),
                });
            }
            _ => crate::quick::set_energy_saver(!crate::quick::energy_saver()),
        },
        Act::Mute => toggle_mute(),
        Act::Menu(i) => {
            close();
            if let Some(pick) = MPICK.take() {
                pick(i as usize);
            }
        }
        Act::Settings(u) => {
            open(&wide(u));
            close();
        }
        Act::Keyboard => touch_keyboard(), // the panel stays open: the keys go to its search field
        Act::CalPrev | Act::CalNext => {
            let (mut y, mut m) = CAL.get();
            m += if a == Act::CalNext { 1 } else { -1 };
            (y, m) = if m < 1 { (y - 1, 12) } else if m > 12 { (y + 1, 1) } else { (y, m) };
            CAL.set((y, m));
        }
        Act::CalToday => {
            let mut t: SYSTEMTIME = zeroed();
            GetLocalTime(&mut t);
            CAL.set((t.wYear as i32, t.wMonth as i32));
        }
        Act::Net(i) => {
            let n = NETS.with_borrow(|n| n.get(i).map(|n| (n.connected, n.profile.clone(), n.auth, n.secure, n.ssid.clone(), n.g)));
            let Some((connected, profile, auth, secure, ssid, iface)) = n else { return };
            if connected {
                // the card is information; its Disconnect button is the action
            } else if let Some(p) = profile {
                wifi_connect(&p, &iface);
                start_connecting(&ssid);
            } else if !secure {
                NETS.with_borrow(|l| wifi_join(&l[i], ""));
            } else if matches!(auth, 4 | 7 | 8 | 9) {
                PASS.set(Some((i, String::new())));
            } else {
                open(&wide("ms-settings:network-wifi"));
                close();
            }
        }
        Act::NetOff => {
            for_each_wlan(|h, g| _ = WlanDisconnect(h, g, null()));
            NOTE.set(t!("Отключено", "Disconnected", ja: "切断", pt: "Desconectado", zh: "已断开").into());
        }
        Act::Ctx(j) => {
            let Some((ssid, ..)) = CTX.take() else { return };
            match ctx_items(&ssid).get(j as usize).map(|i| i.1) {
                Some(0) => {
                    if let Some(i) = NETS.with_borrow(|n| n.iter().position(|n| n.ssid == ssid)) {
                        run_act(Act::Net(i), rect, x);
                    }
                }
                Some(1) => run_act(Act::NetOff, rect, x),
                Some(2) => forget(&ssid),
                Some(3) => {
                    open(&wide("ms-settings:network-wifisettings"));
                    close();
                }
                _ => {}
            }
        }
        Act::App(i) => {
            if launch(i) {
                close();
            }
        }
        Act::Power(p) => {
            if p >= 2 && CONFIRM.get() != p as i32 {
                CONFIRM.set(p as i32); // first click only arms it
                return;
            }
            close();
            match p {
                0 => {
                    let _ = Command::new("rundll32.exe").args(["user32.dll,LockWorkStation"]).creation_flags(NO_WINDOW).spawn();
                }
                1 => {
                    // not `rundll32 powrprof.dll,SetSuspendState`: rundll32 passes garbage arguments and the PC hibernates
                    // instead of sleeping whenever hibernation is on. On its own thread: the call returns only after wake-up.
                    std::thread::spawn(|| sleep_now());
                }
                2 => {
                    let _ = Command::new("shutdown").args(["/r", "/t", "0"]).creation_flags(NO_WINDOW).spawn();
                }
                _ => {
                    let _ = Command::new("shutdown").args(["/s", "/t", "0"]).creation_flags(NO_WINDOW).spawn();
                }
            }
        }
    }
}

/// Sleep (S3 / modern standby), never hibernate. SetSuspendState needs the shutdown privilege switched on in our token.
pub fn sleep_now() {
    use windows_sys::Win32::{
        Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES},
        System::{Power::SetSuspendState, Threading::{GetCurrentProcess, OpenProcessToken}},
    };
    unsafe {
        let mut tok = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut tok) != 0 {
            let mut tp: TOKEN_PRIVILEGES = zeroed();
            tp.PrivilegeCount = 1;
            tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            if LookupPrivilegeValueW(null(), wide("SeShutdownPrivilege").as_ptr(), &mut tp.Privileges[0].Luid) != 0 {
                AdjustTokenPrivileges(tok, 0, &tp, 0, null_mut(), null_mut());
            }
            CloseHandle(tok);
        }
        SetSuspendState(false, false, false);
    }
}

// ---------- painting ----------

/// A slider: a thick rounded track filled in the accent up to `frac`, and a white knob with a soft shadow.
unsafe fn track(dc: HDC, rc: RECT, frac: f32) {
    let (l, rr, m, th) = (rc.left + sc(10), rc.right - sc(10), (rc.top + rc.bottom) / 2, sc(3));
    fill(dc, RECT { left: l, top: m - th, right: rr, bottom: m + th }, mix(card(), pal().text, 0.12), 2 * th);
    let x = l + ((rr - l) as f32 * frac.clamp(0.0, 1.0)) as i32;
    fill(dc, RECT { left: l, top: m - th, right: x.max(l + 2 * th), bottom: m + th }, pal().accent, 2 * th);
    let kr = sc(10);
    fill(dc, RECT { left: x - kr - 1, top: m - kr, right: x + kr + 1, bottom: m + kr + sc(2) }, mix(card(), 0, 0.22), 2 * kr + 2);
    fill(dc, RECT { left: x - kr, top: m - kr, right: x + kr, bottom: m + kr }, if style().light() { rgb(255, 255, 255) } else { mix(pal().text, card(), 0.04) }, 2 * kr);
}

/// A raised rounded surface: a hairline edge (a soft shadow on the light style) around `base`.
unsafe fn surface(dc: HDC, rc: RECT, base: u32, rad: i32) {
    let light = style().light();
    if light {
        fill(dc, RECT { top: rc.top + sc(1), bottom: rc.bottom + sc(2), ..rc }, mix(pal().bg, pal().dim, 0.16), rad);
    }
    fill(dc, rc, if light { mix(pal().bg, pal().line, 0.9) } else { mix(base, pal().text, 0.07) }, rad);
    fill(dc, RECT { left: rc.left + 1, top: rc.top + 1, right: rc.right - 1, bottom: rc.bottom - 1 }, base, rad - 2);
}

fn inset(rc: RECT, d: i32) -> RECT {
    RECT { left: rc.left + d, top: rc.top + d, right: rc.right - d, bottom: rc.bottom - d }
}

unsafe fn paint(hw: HWND, k: Kind) {
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hw, &mut ps);
    let mut cr: RECT = zeroed();
    GetClientRect(hw, &mut cr);
    BUSY.set(false);
    let (dc, bmp) = (CreateCompatibleDC(wdc), CreateCompatibleBitmap(wdc, cr.right, cr.bottom));
    let old = SelectObject(dc, bmp);
    fill(dc, cr, if style().light() { mix(pal().bg, pal().dim, 0.3) } else { mix(pal().bg, pal().text, 0.12) }, sc(16));
    fill(dc, inset(cr, 1), pal().bg, sc(16));
    winrust_common::style::backdrop(dc, inset(cr, 1), sc(100) as f32 / 100.0);
    SetBkMode(dc, TRANSPARENT as i32);
    let (face, gface) = (wide(ui_face()), wide(winrust_common::style::icon_face()));
    let font = |f: &[u16], px: i32, wt: i32| CreateFontW(-sc(px), 0, 0, 0, wt, 0, 0, 0, 1, 0, 0, 5, 0, f.as_ptr());
    let (f_txt, f_small, f_glyph, f_title, f_big) = (font(&face, 13, 400), font(&face, 12, 400), font(&gface, 16, 400), font(&face, 15, 600), font(&face, 34, 600));
    let (f_semi, f_head, f_gs, f_label) = (font(&face, 13, 600), font(&face, 18, 700), font(&gface, 11, 400), font(&face, 12, 600));
    let card = card();
    let of = SelectObject(dc, f_txt);
    let (w, h) = size(k);
    let over = OVER.get();
    let hot = |a: Act| over == Some(a);
    // hover highlight that fades; the key is the element's position (flyout keys start at 1_000_000)
    let g = |a: Act, rc: RECT, from: u32, to: u32, rad: i32| glow(dc, rc.top * 4096 + rc.left + 1_000_000, hot(a), rc, from, to, rad);
    let (vol, muted) = VOL.get();

    match k {
        Kind::Quick => {
            let rs = rects(k);
            let q = WIFI.get();
            let ssid = NETS.with_borrow(|n| n.iter().find(|n| n.connected).map(|n| n.ssid.clone()));
            let wsub = match (q >= 0, ssid) {
                (true, Some(s)) => s,
                (true, None) => t!("Подключено", "Connected", ja: "接続済み", pt: "Conectado", zh: "已连接").into(),
                _ if NETS.with_borrow(|n| n.is_empty()) => t!("Выключен", "Off", ja: "オフ", pt: "Desligado", zh: "关").into(),
                _ => t!("Не подключено", "Not connected", ja: "未接続", pt: "Não conectado", zh: "未连接").into(),
            };
            let (bt, air, acc, eco) = (crate::quick::bluetooth(), crate::quick::airplane(), crate::quick::accessibility_on(), crate::quick::energy_saver());
            let onoff = |b: Option<bool>| match b {
                Some(true) => t!("Вкл.", "On", ja: "オン", pt: "Ligado", zh: "开").to_string(),
                Some(false) => t!("Выкл.", "Off", ja: "オフ", pt: "Desligado", zh: "关").to_string(),
                None => t!("Нет", "None", ja: "なし", pt: "Nenhum", zh: "无").to_string(),
            };
            let states = [q >= 0, bt == Some(true), air == Some(true), acc, eco, false];
            let tiles = [
                ("\u{E701}", "Wi‑Fi".to_string(), wsub),
                ("\u{E702}", t!("Bluetooth", "Bluetooth", ja: "Bluetooth", pt: "Bluetooth", zh: "蓝牙").to_string(), onoff(bt)),
                ("\u{E709}", t!("Авиарежим", "Airplane mode", ja: "機内モード", pt: "Modo avião", zh: "飞行模式").to_string(), onoff(air)),
                ("\u{E776}", t!("Спец. возможности", "Accessibility", ja: "アクセシビリティ", pt: "Acessibilidade", zh: "辅助功能").to_string(), onoff(Some(acc))),
                ("\u{E945}", t!("Экономия энергии", "Energy saver", ja: "省エネ", pt: "Economia de energia", zh: "节能模式").to_string(), onoff(Some(eco))),
                ("\u{E7F0}", t!("Субтитры", "Live captions", ja: "ライブ キャプション", pt: "Legendas ao vivo", zh: "实时字幕").to_string(), String::new()),
            ];
            for (i, (glyph, label, sub)) in tiles.iter().enumerate() {
                let (tl, tt) = (16 + 112 * (i as i32 % 3), 16 + 84 * (i as i32 / 3));
                let full = r(tl, tt, tl + 104, tt + 76);
                let on = states[i];
                let base = if on { mix(card, pal().accent, 0.12) } else { card };
                surface(dc, full, base, sc(18));
                g(Act::Tile(i as u8), inset(full, 1), base, mix(base, pal().text, 0.05), sc(16));
                // the round icon badge: lit in the accent when on
                let badge = r(tl + 12, tt + 12, tl + 44, tt + 44);
                fill(dc, badge, if on { pal().accent } else { mix(card, pal().text, 0.08) }, sc(32));
                let ink = if on { on_accent() } else { pal().text };
                if i == 0 {
                    // the same arcs as the bar: lit by the signal, a cross when not connected
                    wifi_glyph(dc, (badge.left + badge.right) / 2, (badge.top + badge.bottom) / 2 - sc(1), sc(15), if q >= 0 { arcs_lit(q) } else { 0 }, ink, mix(ink, if on { pal().accent } else { card }, 0.55), if q < 0 { 1 } else { 0 });
                    let more = rs.iter().find(|(_, a)| *a == Act::WifiMore).unwrap().0;
                    g(Act::WifiMore, more, base, mix(base, pal().text, 0.1), sc(30));
                    SelectObject(dc, f_gs);
                    text(dc, "\u{E76C}", more, pal().dim, DT_CENTER);
                } else {
                    SelectObject(dc, f_glyph);
                    text(dc, glyph, badge, ink, DT_CENTER);
                }
                SelectObject(dc, f_label);
                text(dc, label.as_str(), r(tl + 12, tt + 46, tl + 102, tt + 62), pal().text, DT_LEFT);
                if !sub.is_empty() {
                    SelectObject(dc, f_small);
                    text(dc, sub, r(tl + 12, tt + 61, tl + 100, tt + 75), pal().dim, DT_LEFT);
                }
            }
            // sliders sit together on one card
            let n = sliders();
            surface(dc, r(16, 184, w - 16, 192 + 44 * n), card, sc(18));
            let mut y = 188;
            let b = BRIGHT.load(Relaxed);
            if b >= 0 {
                SelectObject(dc, f_glyph);
                text(dc, "\u{E706}", r(24, y, 60, y + 40), mix(pal().dim, pal().text, 0.5), DT_CENTER);
                track(dc, rs.iter().find(|(_, a)| *a == Act::BrightSlide).unwrap().0, b as f32 / 100.0);
                y += 44;
            }
            g(Act::Mute, r(24, y + 2, 60, y + 38), card, mix(card, pal().text, 0.08), sc(36));
            SelectObject(dc, f_glyph);
            text(dc, if muted || vol == 0 { "\u{E74F}" } else { "\u{E767}" }, r(24, y, 60, y + 40), if muted { BAD } else { mix(pal().dim, pal().text, 0.5) }, DT_CENTER);
            track(dc, rs.iter().find(|(_, a)| *a == Act::VolSlide).unwrap().0, if muted { 0.0 } else { vol.max(0) as f32 / 100.0 });
            // footer: the battery in a pill, the settings gear in a circle
            let (pct, ac) = BAT.get();
            if pct >= 0 {
                let pill = r(16, h - 48, 102, h - 12);
                surface(dc, pill, card, sc(36));
                SelectObject(dc, f_glyph);
                text(dc, if ac { "\u{E83E}" } else { "\u{E83F}" }, r(26, h - 48, 50, h - 12), if ac { GOOD } else if pct <= 15 { BAD } else { pal().text }, DT_CENTER);
                SelectObject(dc, f_semi);
                text(dc, &format!("{pct}%"), r(54, h - 48, 98, h - 12), pal().text, DT_LEFT);
            }
            let gear = r(w - 54, h - 50, w - 14, h - 10);
            surface(dc, gear, card, sc(40));
            g(Act::Settings("ms-settings:"), inset(gear, 1), card, mix(card, pal().text, 0.08), sc(38));
            SelectObject(dc, f_glyph);
            text(dc, "\u{E713}", gear, pal().text, DT_CENTER);
        }
        Kind::Cal => {
            let (y, m) = CAL.get();
            SelectObject(dc, f_title);
            text(dc, &month_title(y, m), r(16, 10, w - 100, 42), pal().text, DT_LEFT);
            SelectObject(dc, f_glyph);
            text(dc, "\u{E70E}", r(w - 84, 10, w - 52, 42), pal().text, DT_CENTER);
            text(dc, "\u{E70D}", r(w - 48, 10, w - 16, 42), pal().text, DT_CENTER);
            SelectObject(dc, f_small);
            for (i, d) in weekday_initials().iter().enumerate() {
                text(dc, d, r(18 + 42 * i as i32, 54, 60 + 42 * i as i32, 78), pal().dim, DT_CENTER);
            }
            let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
            let yy = if m < 3 { y - 1 } else { y };
            let first = ((yy + yy / 4 - yy / 100 + yy / 400 + t[m as usize - 1] + 1) % 7 + 6) % 7; // Monday = 0
            let dim = match m {
                2 => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 },
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            let mut now: SYSTEMTIME = zeroed();
            GetLocalTime(&mut now);
            SelectObject(dc, f_txt);
            for d in 1..=dim {
                let (col, row) = ((first + d - 1) % 7, (first + d - 1) / 7);
                let cell = r(18 + 42 * col, 84 + 40 * row, 60 + 42 * col, 120 + 40 * row);
                let today = now.wYear as i32 == y && now.wMonth as i32 == m && now.wDay as i32 == d;
                if today {
                    fill(dc, cell, pal().accent, sc(18));
                }
                text(dc, &d.to_string(), cell, if today { pal().bg } else { pal().text }, DT_CENTER);
            }
        }
        Kind::Wifi => {
            let on = NETS.with_borrow(|n| !n.is_empty()) || WIFI.get() >= 0;
            let state = net();
            // header: the title, and a status line only when it says something the list does not
            let note = NOTE.with_borrow(|n| n.clone());
            let sub = if !note.is_empty() {
                Some((note, pal().dim))
            } else {
                match state {
                    Link::Wifi(_) => None,
                    Link::NoInternet(_) => Some((t!("Подключено, но интернета нет", "Connected, no internet", ja: "接続済み、インターネットなし", pt: "Conectado, sem internet", zh: "已连接，无 Internet").to_string(), WARN)),
                    Link::Cable => Some((t!("Интернет по кабелю", "Online over a cable", ja: "有線でオンライン", pt: "Online por cabo", zh: "有线联网").into(), pal().dim)),
                    Link::Offline if on => Some((t!("Не подключено", "Not connected", ja: "未接続", pt: "Não conectado", zh: "未连接").into(), pal().dim)),
                    Link::Offline => Some((t!("Выключен", "Turned off", ja: "オフ", pt: "Desligado", zh: "已关闭").into(), pal().dim)),
                }
            };
            SelectObject(dc, f_title);
            match &sub {
                Some((s, c)) => {
                    text(dc, "Wi‑Fi", r(18, 13, w - 76, 33), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, s, r(18, 33, w - 76, 51), *c, DT_LEFT);
                }
                None => text(dc, "Wi‑Fi", r(18, 18, w - 76, 42), pal().text, DT_LEFT),
            }
            // switch: the track fades to the accent, a white knob glides across
            let sw = r(w - 58, 18, w - 16, 42);
            let ts = ease(hv(1_500_000, on));
            let swh = sw.bottom - sw.top;
            fill(dc, sw, mix(mix(card, pal().text, 0.18), pal().accent, ts), swh);
            let kr = swh / 2 - sc(2);
            let kx = sw.left + swh / 2 + ((sw.right - sw.left - swh) as f32 * ts) as i32;
            let ky = (sw.top + sw.bottom) / 2;
            fill(dc, RECT { left: kx - kr, top: ky - kr, right: kx + kr, bottom: ky + kr }, rgb(255, 255, 255), 2 * kr);
            let rs = rects(k);
            let pass = PASS.with_borrow(|p| p.clone());
            if let Some((i, p)) = pass {
                let name = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone()).unwrap_or_default());
                SelectObject(dc, f_txt);
                text(dc, &tf!("Пароль для «{name}»", "Password for “{name}”", ja: "「{name}」のパスワード", pt: "Senha de “{name}”", zh: "“{name}”的密码"), r(18, 62, w - 18, 84), pal().text, DT_LEFT);
                fill(dc, r(16, 90, w - 16, 128), pal().accent, sc(10));
                fill(dc, r(18, 92, w - 18, 126), card, sc(8));
                let dots = "•".repeat(p.chars().count());
                text(dc, &dots, r(30, 90, w - 30, 128), pal().text, DT_LEFT);
                // a blinking caret after the dots
                if now_ms() / 530 % 2 == 0 {
                    let mut tr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    let wd = wide(&dots);
                    DrawTextW(dc, wd.as_ptr(), -1, &mut tr, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
                    let cx = sc(30) + tr.right - tr.left + sc(2);
                    fill(dc, RECT { left: cx, top: sc(100), right: cx + sc(2), bottom: sc(118) }, pal().accent, 0);
                }
                BUSY.set(true);
                SelectObject(dc, f_small);
                text(dc, t!("Enter — подключиться · Esc — отмена", "Enter — connect · Esc — cancel", ja: "Enter — 接続 · Esc — キャンセル", pt: "Enter — conectar · Esc — cancelar", zh: "Enter — 连接 · Esc — 取消"), r(18, 136, w - 18, 156), pal().dim, DT_LEFT);
            } else {
                let (rows, label, _) = wifi_rows();
                let conn = CONN.with_borrow(|c| c.as_ref().map(|c| (c.q, c.phy, c.rx, c.tx)));
                let joining = CONNECTING.with_borrow(|c| c.as_ref().map(|c| c.0.clone()));
                let age = now_ms().saturating_sub(T0.get()) as f32;
                let now = now_ms();
                NETS.with_borrow(|nets| {
                    if nets.is_empty() {
                        SelectObject(dc, f_txt);
                        text(dc, if on { t!("Рядом нет сетей", "No networks nearby", ja: "近くにネットワークがありません", pt: "Nenhuma rede por perto", zh: "附近没有网络") } else { t!("Включите Wi‑Fi, чтобы увидеть сети", "Turn Wi‑Fi on to see networks", ja: "ネットワークを表示するには Wi‑Fi をオンにしてください", pt: "Ative o Wi‑Fi para ver as redes", zh: "开启 Wi‑Fi 以查看网络") }, r(18, 70, w - 18, 110), pal().dim, DT_LEFT);
                    }
                    if let Some(ly) = label {
                        SelectObject(dc, f_small);
                        text(dc, t!("Другие сети", "Other networks", ja: "その他のネットワーク", pt: "Outras redes", zh: "其他网络"), r(20, ly + 2, w - 16, ly + 22), pal().dim, DT_LEFT);
                    }
                    // the connected network on its own card, the others together on one, split by hairlines
                    let others: Vec<usize> = (0..nets.len()).filter(|&i| !nets[i].connected).collect();
                    if let (Some(&a), Some(&b)) = (others.first(), others.last()) {
                        surface(dc, r(12, rows[a].0, w - 12, rows[b].1), card, sc(14));
                    }
                    for (i, n) in nets.iter().enumerate() {
                        let rc = rs[i].0;
                        // rows fade in one after another when the panel opens
                        let t = ease(((age - 40.0 * i as f32) / 220.0).clamp(0.0, 1.0));
                        if t < 1.0 {
                            BUSY.set(true);
                        }
                        let ink = |c: u32| mix(card, c, t);
                        let busy = joining.as_deref() == Some(n.ssid.as_str());
                        if n.connected {
                            surface(dc, rc, card, sc(14));
                        } else if others.last() != Some(&i) {
                            fill(dc, RECT { left: rc.left + sc(48), top: rc.bottom - 1, right: rc.right - sc(14), bottom: rc.bottom }, mix(card, pal().text, 0.08), 0);
                        }
                        let open = hot(Act::Net(i)) || (n.connected && hot(Act::NetOff));
                        g(Act::Net(i), RECT { left: rc.left + sc(4), top: rc.top + sc(4), right: rc.right - sc(4), bottom: rc.bottom - sc(4) }, card, mix(card, pal().text, 0.045), sc(10));
                        // signal arcs; while joining they sweep up one after another
                        let lit = if busy { BUSY.set(true); (now / 230 % 4) as i32 } else { arcs_lit(n.q) };
                        let online = !matches!(state, Link::NoInternet(_));
                        let cy = (rc.top + rc.bottom) / 2;
                        let arc = if n.connected || busy { pal().accent } else { pal().text };
                        wifi_glyph(dc, rc.left + sc(26), cy - sc(1), sc(16), lit, ink(arc), ink(mix(pal().dim, card, 0.55)), if n.connected && !online { 2 } else { 0 });
                        let saved = n.profile.is_some();
                        let (sub, sub_c) = if busy {
                            (format!("{}{}", t!("Подключение", "Connecting", ja: "接続中", pt: "Conectando", zh: "正在连接"), &"...."[..(now / 400 % 4) as usize]), pal().accent)
                        } else if n.connected {
                            let q = conn.map(|c| c.0).unwrap_or(n.q);
                            match conn {
                                _ if !online => (t!("Подключено, без интернета", "Connected, no internet", ja: "接続済み、インターネットなし", pt: "Conectado, sem internet", zh: "已连接，无 Internet").into(), WARN),
                                // under the pointer the line shows the link details instead
                                Some((_, _, rx, tx)) if open => (format!("↓{rx} ↑{tx} {} · {}", t!("Мбит/с", "Mbps", ja: "Mbps", pt: "Mbps", zh: "Mbps"), auth_name(n.auth)), pal().dim),
                                Some((_, phy, ..)) => (format!("{} · {phy} · {q}%", t!("Подключено", "Connected", ja: "接続済み", pt: "Conectado", zh: "已连接")), pal().dim),
                                None => (format!("{} · {q}%", t!("Подключено", "Connected", ja: "接続済み", pt: "Conectado", zh: "已连接")), pal().dim),
                            }
                        } else if !n.secure {
                            (t!("Открытая сеть", "Open network", ja: "オープン ネットワーク", pt: "Rede aberta", zh: "开放网络").into(), WARN)
                        } else if saved {
                            (t!("Сохранена", "Saved", ja: "保存済み", pt: "Salva", zh: "已保存").into(), pal().dim)
                        } else {
                            (String::new(), pal().dim)
                        };
                        let right = rc.right - sc(118);
                        SelectObject(dc, if n.connected { f_semi } else { f_txt });
                        if sub.is_empty() {
                            text(dc, &n.ssid, RECT { left: rc.left + sc(48), right, ..rc }, ink(pal().text), DT_LEFT);
                        } else {
                            text(dc, &n.ssid, RECT { left: rc.left + sc(48), top: cy - sc(19), right, bottom: cy + sc(1) }, ink(pal().text), DT_LEFT);
                            SelectObject(dc, f_small);
                            text(dc, &sub, RECT { left: rc.left + sc(48), top: cy, right, bottom: cy + sc(18) }, ink(sub_c), DT_LEFT);
                        }
                        if busy {
                            // an indeterminate bar sliding along the row's bottom edge
                            let (l, rr) = (rc.left + sc(48), rc.right - sc(16));
                            let span = rr - l;
                            let p = (now % 1300) as f32 / 1300.0;
                            let a = l + ((span as f32) * (ease(p) * 1.3 - 0.3)) as i32;
                            let b = a + span * 3 / 10;
                            fill(dc, RECT { left: l, top: rc.bottom - sc(5), right: rr, bottom: rc.bottom - sc(3) }, mix(card, pal().accent, 0.18), sc(2));
                            fill(dc, RECT { left: a.max(l), top: rc.bottom - sc(5), right: b.min(rr), bottom: rc.bottom - sc(3) }, pal().accent, sc(2));
                        }
                        if n.connected {
                            if open {
                                if let Some(&(ob, _)) = rs.iter().find(|(_, a)| *a == Act::NetOff) {
                                    g(Act::NetOff, ob, card, mix(card, BAD, 0.14), ob.bottom - ob.top);
                                    SelectObject(dc, f_semi);
                                    text(dc, t!("Отключиться", "Disconnect", ja: "切断", pt: "Desconectar", zh: "断开连接"), ob, if hot(Act::NetOff) { BAD } else { pal().dim }, DT_CENTER);
                                }
                            } else {
                                SelectObject(dc, f_glyph);
                                text(dc, "\u{E73E}", RECT { left: rc.right - sc(44), right: rc.right - sc(14), ..rc }, ink(pal().accent), DT_CENTER); // check mark
                            }
                        } else if open && !busy {
                            SelectObject(dc, f_semi);
                            let action = if saved || !n.secure { t!("Подключить", "Connect", ja: "接続", pt: "Conectar", zh: "连接") } else { t!("Ввести пароль", "Enter password", ja: "パスワードを入力", pt: "Digite a senha", zh: "输入密码") };
                            text(dc, action, RECT { left: rc.right - sc(130), right: rc.right - sc(16), ..rc }, pal().accent, DT_RIGHT);
                        } else if n.secure && !busy {
                            SelectObject(dc, f_gs);
                            text(dc, "\u{E72E}", RECT { left: rc.right - sc(42), right: rc.right - sc(14), ..rc }, ink(mix(pal().dim, card, 0.35)), DT_CENTER); // lock
                        }
                    }
                });
            }
            if let Some(&(lr, la)) = rs.iter().find(|(_, a)| matches!(a, Act::Settings(_))) {
                g(la, lr, pal().bg, mix(pal().bg, pal().text, 0.05), sc(10));
                SelectObject(dc, f_txt);
                text(dc, t!("Параметры сети", "Network settings", ja: "ネットワーク設定", pt: "Configurações de rede", zh: "网络设置"), RECT { left: lr.left + sc(8), ..lr }, pal().accent, DT_LEFT);
                SelectObject(dc, f_gs);
                text(dc, "\u{E76C}", RECT { left: lr.right - sc(34), ..lr }, pal().dim, DT_CENTER); // chevron right
            }
            // the right-click menu unfolds downward from where it was opened
            if let Some((ssid, mx, my, t0)) = CTX.with_borrow(|c| c.clone()) {
                let items = ctx_items(&ssid);
                let t = ease((now_ms().saturating_sub(t0) as f32 / 150.0).min(1.0));
                if t < 1.0 {
                    BUSY.set(true);
                }
                let full = sc(12 + CTX_ROW * items.len() as i32);
                let mr = RECT { left: mx, top: my, right: mx + sc(CTX_W), bottom: my + (full as f32 * (0.4 + 0.6 * t)) as i32 };
                fill(dc, RECT { left: mr.left - 1, top: mr.top + sc(2), right: mr.right + 1, bottom: mr.bottom + sc(4) }, mix(pal().bg, 0, if style().light() { 0.12 } else { 0.5 }), sc(14));
                surface(dc, mr, card, sc(12));
                for ((ir, a), (label, what)) in ctx_rects().iter().zip(&items) {
                    if ir.bottom > mr.bottom - sc(4) {
                        break; // still unfolding
                    }
                    g(*a, *ir, card, if *what == 2 { mix(card, BAD, 0.12) } else { mix(card, pal().text, 0.06) }, sc(8));
                    SelectObject(dc, f_txt);
                    text(dc, label, RECT { left: ir.left + sc(12), ..*ir }, if *what == 2 { BAD } else { pal().text }, DT_LEFT);
                }
            }
        }
        Kind::Menu => {
            // header: the app's icon on an accent-tinted tile, its name, and what is open
            MHEAD.with_borrow(|hd| {
                if let Some((ic, glyph, title, sub)) = hd {
                    let tile = r(14, 12, 48, 46);
                    fill(dc, tile, mix(pal().bg, pal().accent, 0.16), sc(10));
                    if !ic.is_null() {
                        DrawIconEx(dc, tile.left + sc(5), tile.top + sc(5), *ic, sc(24), sc(24), 0, null_mut(), DI_NORMAL);
                    } else {
                        SelectObject(dc, f_glyph); // no app icon (Style, Language): the menu's own symbol
                        text(dc, glyph, tile, pal().accent, DT_CENTER);
                    }
                    SelectObject(dc, f_title);
                    text(dc, title, r(60, 10, w - 14, 32), pal().text, DT_LEFT);
                    SelectObject(dc, f_small);
                    text(dc, sub, r(60, 30, w - 14, 48), pal().dim, DT_LEFT);
                    fill(dc, r(12, MHEAD_H + 4, w - 12, MHEAD_H + 5), pal().line, 0);
                }
            });
            let rs = rects(k);
            MITEMS.with_borrow(|items| {
                // section captions have no rect of their own (not clickable): drawn straight from the row layout
                SelectObject(dc, f_small);
                for (it, (t, b)) in items.iter().zip(menu_rows().0) {
                    if it.head {
                        if it.sep {
                            fill(dc, r(12, t - 5, w - 12, t - 4), pal().line, 0);
                        }
                        text(dc, &it.label, r(20, t + 8, w - 14, b), pal().dim, DT_LEFT);
                    }
                }
                for (rc, a) in rs.iter() {
                    let Act::Menu(i) = *a else { continue };
                    let it = &items[i as usize];
                    if it.sep {
                        fill(dc, RECT { left: rc.left + sc(6), top: rc.top - sc(5), right: rc.right - sc(6), bottom: rc.top - sc(5) + 1 }, pal().line, 0);
                    }
                    let col = if it.danger { BAD } else { pal().text };
                    g(*a, *rc, pal().bg, if it.danger { mix(pal().bg, BAD, 0.22) } else { pal().hover }, sc(8));
                    // plain text rows: a window's own icon is the only picture; the current choice gets an accent dot
                    let mut lx = rc.left + sc(14);
                    if !it.icon.is_null() {
                        DrawIconEx(dc, lx, (rc.top + rc.bottom) / 2 - sc(8), it.icon, sc(16), sc(16), 0, null_mut(), DI_NORMAL);
                        lx += sc(26);
                    }
                    if it.swatch != u32::MAX {
                        let (cy, d) = ((rc.top + rc.bottom) / 2, sc(9));
                        fill(dc, RECT { left: lx, top: cy - d, right: lx + 2 * d, bottom: cy + d }, pal().line, 2 * d); // ring: the dot shows on any background
                        fill(dc, RECT { left: lx + 1, top: cy - d + 1, right: lx + 2 * d - 1, bottom: cy + d - 1 }, it.swatch, 2 * d);
                        lx += sc(28);
                    }
                    let mut rx = rc.right - sc(14);
                    if it.on {
                        let (cy, d) = ((rc.top + rc.bottom) / 2, sc(4));
                        fill(dc, RECT { left: rx - 2 * d, top: cy - d, right: rx, bottom: cy + d }, pal().accent, 2 * d);
                        rx -= sc(18);
                    }
                    SelectObject(dc, f_small);
                    if it.beta {
                        text(dc, "BETA", RECT { left: lx, right: rx, ..*rc }, rgb(255, 59, 48), DT_RIGHT);
                    }
                    if !it.hint.is_empty() {
                        text(dc, it.hint, RECT { left: lx, right: rx, ..*rc }, mix(pal().dim, pal().bg, 0.25), DT_RIGHT);
                    }
                    SelectObject(dc, f_txt);
                    text(dc, &it.label, RECT { left: lx, right: rx - match it.hint.chars().count() { 0 => 0, 1..=2 => sc(20), _ => sc(96) }, ..*rc }, if it.on { pal().accent } else { col }, DT_LEFT);
                }
            });
        }
        Kind::Start => {
            surface(dc, r(20, 20, w - 20, 56), card, sc(36));
            SelectObject(dc, f_glyph);
            text(dc, "\u{E721}", r(28, 20, 56, 56), pal().dim, DT_CENTER);
            SelectObject(dc, f_txt);
            let q = QUERY.with_borrow(|q| q.clone());
            if q.is_empty() {
                text(dc, t!("Введите для поиска", "Type to search apps", ja: "アプリを検索", pt: "Digite para pesquisar apps", zh: "输入以搜索应用"), r(64, 20, w - 64, 56), pal().dim, DT_LEFT);
            } else {
                text(dc, &q, r(64, 20, w - 64, 56), if SELALL.get() { pal().accent } else { pal().text }, DT_LEFT);
            }
            if let Some((kr, ka)) = rects(k).into_iter().find(|(_, a)| *a == Act::Keyboard) {
                g(ka, kr, card, mix(card, pal().text, 0.1), sc(14));
                SelectObject(dc, f_glyph);
                text(dc, "\u{E765}", kr, if hot(ka) { pal().text } else { pal().dim }, DT_CENTER); // keyboard
                SelectObject(dc, f_txt);
            }
            let f = filtered();
            if f.is_empty() {
                if q.is_empty() {
                    text(dc, t!("Здесь пока пусто", "Nothing here yet", ja: "まだ何もありません", pt: "Nada aqui ainda", zh: "这里还什么都没有"), r(24, 76, w - 24, 106), pal().text, DT_LEFT);
                    text(dc, t!("Перетащите приложение или файл на кнопку «Пуск», чтобы добавить", "Drag an app or file onto the Start button to add it", ja: "スタート ボタンにアプリやファイルをドラッグして追加します", pt: "Arraste um app ou arquivo para o botão Iniciar para adicionar", zh: "把应用或文件拖到“开始”按钮即可添加"), r(24, 106, w - 24, 134), pal().dim, DT_LEFT);
                    text(dc, t!("Или найдите приложение и добавьте его из меню по правой кнопке", "Or search an app and right-click it → Add to Start", ja: "またはアプリを検索し、右クリックして「スタートに追加」を選びます", pt: "Ou pesquise um app e use o menu do botão direito → Adicionar ao Iniciar", zh: "或者搜索应用，右键点击 → 添加到开始菜单"), r(24, 134, w - 24, 162), pal().dim, DT_LEFT);
                } else {
                    text(dc, t!("Приложения не найдены", "No apps found", ja: "アプリが見つかりません", pt: "Nenhum app encontrado", zh: "未找到应用"), r(24, 76, w - 24, 116), pal().dim, DT_LEFT);
                }
            }
            let rs = rects(k);
            let apps = APPS.lock().unwrap();
            for (rc, a) in &rs {
                match *a {
                    Act::App(i) => {
                        // the list is reloaded on a background thread: an index from a moment ago may be gone
                        let Some(app) = f.get(i).and_then(|&ai| apps.get(ai)) else { continue };
                        // the row that was just pinned pulses in the accent colour for a moment
                        let pulse = JUST.with_borrow(|j| {
                            let age = now_ms().saturating_sub(j.1) as f32 / 1200.0;
                            if age < 1.0 && !j.0.is_empty() && app.1 == wide(&j.0) { (1.0 - age) * (1.0 - age) } else { 0.0 }
                        });
                        let sel = i == SEL.get(); // what Enter opens
                        if pulse > 0.0 || sel {
                            BUSY.set(BUSY.get() || pulse > 0.0);
                            fill(dc, *rc, mix(if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, pal().accent, 0.45 * pulse), sc(8));
                        }
                        g(*a, *rc, if sel { mix(pal().bg, pal().accent, 0.14) } else { pal().bg }, if sel { mix(pal().hover, pal().accent, 0.18) } else { pal().hover }, sc(8));
                        if sel {
                            fill(dc, RECT { left: rc.left, top: rc.top + sc(10), right: rc.left + sc(3), bottom: rc.bottom - sc(10) }, pal().accent, sc(3));
                        }
                        let ic = ICONS.with_borrow_mut(|m| {
                            *m.entry(app.1.clone()).or_insert_with(|| {
                                let t = lnk_target(&app.1);
                                lnk_icon(&app.1, &t)
                            })
                        });
                        if !ic.is_null() {
                            DrawIconEx(dc, rc.left + sc(13), (rc.top + rc.bottom - icon_px()) / 2, ic, icon_px(), icon_px(), 0, null_mut(), DI_NORMAL); // at its loaded size: no stretching
                        }
                        let store = wstr(&app.1).starts_with("shell:");
                        text(dc, &app.0, RECT { left: rc.left + sc(52), top: rc.top, right: rc.right - sc(110), bottom: rc.bottom }, pal().text, DT_LEFT);
                        SelectObject(dc, f_small);
                        let kind = if sel && !q.is_empty() { "Enter ↵" } else if store { t!("Приложение", "App", ja: "アプリ", pt: "App", zh: "应用") } else { "" };
                        text(dc, kind, RECT { left: rc.right - sc(110), top: rc.top, right: rc.right - sc(14), bottom: rc.bottom }, pal().dim, DT_RIGHT);
                        SelectObject(dc, f_txt);
                    }
                    Act::Power(p) => {
                        let armed = CONFIRM.get() == p as i32;
                        glow(dc, rc.top * 4096 + rc.left + 1_000_000, armed || hot(*a), *rc, pal().bg, if armed { BAD } else { pal().hover }, sc(8));
                        SelectObject(dc, f_glyph);
                        text(dc, ["\u{E72E}", "\u{E708}", "\u{E777}", "\u{E7E8}"][p as usize], *rc, pal().text, DT_CENTER);
                    }
                    _ => {}
                }
            }
            SelectObject(dc, f_small);
            let note = NOTE.with_borrow(|n| n.clone());
            let hint = match CONFIRM.get() {
                2 => t!("Нажмите ещё раз для перезагрузки", "Click again to restart", ja: "もう一度クリックで再起動", pt: "Clique de novo para reiniciar", zh: "再次点击以重启").to_string(),
                3 => t!("Нажмите ещё раз для выключения", "Click again to shut down", ja: "もう一度クリックでシャットダウン", pt: "Clique de novo para desligar", zh: "再次点击以关机").to_string(),
                _ if !note.is_empty() => note,
                _ => std::env::var("USERNAME").unwrap_or_default(),
            };
            text(dc, &hint, r(24, h - 52, w - 200, h - 12), pal().dim, DT_LEFT);
        }
    }

    BitBlt(wdc, 0, 0, cr.right, cr.bottom, dc, 0, 0, SRCCOPY);
    SelectObject(dc, of);
    for f in [f_txt, f_small, f_glyph, f_title, f_big, f_semi, f_head, f_gs, f_label] {
        DeleteObject(f);
    }
    SelectObject(dc, old);
    DeleteObject(bmp);
    DeleteDC(dc);
    if BUSY.get() {
        SetTimer(hw, 2, 16, None); // next animation frame
    } else {
        KillTimer(hw, 2);
    }
    EndPaint(hw, &ps);
}

// ---------- window proc ----------

unsafe extern "system" fn proc(hw: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wp == 3 {
        anim_step(hw); // also for the fading-out ghost window
        return 0;
    }
    if hw != FH.get() {
        return DefWindowProcW(hw, msg, wp, lp); // ghost or a window still being created
    }
    let Some(k) = FLY.get() else { return DefWindowProcW(hw, msg, wp, lp) };
    let (x, y) = ((lp & 0xffff) as i16 as i32, (lp >> 16 & 0xffff) as i16 as i32);
    match msg {
        WM_PAINT => paint(hw, k),
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xffff == 0 && !PREVIEW.load(Relaxed) => {
            // A file drag that began elsewhere (desktop icon, folder window) deactivates this panel with the button still
            // down: keep Start open so the drop can land on it, and close once the button is up (timer 6).
            if k == Kind::Start && GetAsyncKeyState(VK_LBUTTON as i32) < 0 {
                SetTimer(hw, 6, 150, None);
            } else {
                DEACT.set((Some(k), now_ms()));
                close();
            }
        }
        WM_TIMER if wp == 6 => {
            if GetAsyncKeyState(VK_LBUTTON as i32) >= 0 {
                KillTimer(hw, 6);
                if GetForegroundWindow() != hw {
                    close();
                }
            }
        }
        WM_DROPFILES => on_files(hw, wp as HDROP),
        WM_TIMER if wp == 2 => {
            InvalidateRect(hw, null(), 0);
        }
        WM_TIMER => {
            if k == Kind::Wifi && PASS.with_borrow(|p| p.is_none()) {
                read_nets();
                let (w, h) = (sc(size(k).0), sc(size(k).1));
                // the panel hangs above the bar: keep its bottom edge where it is and grow / shrink upward
                let (x, _) = FIN.get();
                let y = pop_y(h) + origin().1;
                FIN.set((x, y));
                SetWindowPos(hw, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                SetWindowRgn(hw, CreateRoundRectRgn(0, 0, w + 1, h + 1, rr(16), rr(16)), 1);
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_MOUSEMOVE => {
            if DRAG.get() != 0 {
                let want = if DRAG.get() == 1 { Act::VolSlide } else { Act::BrightSlide };
                if let Some((rc, a)) = rects(k).into_iter().find(|(_, a)| *a == want) {
                    slide(a, x, rc);
                }
            }
            let o = act_at(k, x, y).map(|a| a.1);
            if OVER.replace(o) != o || DRAG.get() != 0 {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_LBUTTONDOWN => {
            let hit = act_at(k, x, y);
            if CTX.with_borrow(|c| c.is_some()) && !matches!(hit, Some((_, Act::Ctx(_)))) {
                CTX.set(None); // a click beside the open menu only closes it
            } else if let Some((rc, a)) = hit {
                run_act(a, rc, x);
            }
            if FLY.get().is_some() {
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_RBUTTONUP => {
            match (k, act_at(k, x, y)) {
                (Kind::Start, Some((_, Act::App(i)))) => app_menu(hw, i, x, y),
                (Kind::Wifi, Some((_, Act::Net(_) | Act::NetOff))) if PASS.with_borrow(|p| p.is_none()) => {
                    let i = if let Some((_, Act::Net(i))) = act_at(k, x, y) { i } else { NETS.with_borrow(|n| n.iter().position(|n| n.connected)).unwrap_or(0) };
                    if let Some(ssid) = NETS.with_borrow(|n| n.get(i).map(|n| n.ssid.clone())) {
                        // keep the menu inside the panel
                        let (pw, ph) = (sc(size(k).0), sc(size(k).1));
                        let mh = sc(12 + CTX_ROW * ctx_items(&ssid).len() as i32);
                        CTX.set(Some((ssid, x.min(pw - sc(CTX_W) - sc(8)).max(sc(8)), y.min(ph - mh - sc(8)).max(sc(8)), now_ms())));
                    }
                }
                _ => {}
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_LBUTTONUP => {
            let d = DRAG.replace(0);
            if d != 0 {
                ReleaseCapture();
                if d == 2 {
                    set_brightness(BRIGHT.load(Relaxed));
                }
            }
        }
        WM_MOUSEWHEEL => {
            let up = (wp >> 16) as i16 > 0;
            match k {
                Kind::Quick => add_volume(if up { 2 } else { -2 }),
                Kind::Start => SCROLL.set(if up { SCROLL.get().saturating_sub(2) } else { (SCROLL.get() + 2).min(filtered().len().saturating_sub(ROWS)) }),
                _ => {}
            }
            InvalidateRect(hw, null(), 0);
        }
        WM_KEYDOWN if k == Kind::Start && (wp == VK_DOWN as usize || wp == VK_UP as usize) => {
            // arrows move the row Enter opens; the list scrolls to keep it in view
            let n = filtered().len();
            if n > 0 {
                let s = if wp == VK_DOWN as usize { (SEL.get() + 1).min(n - 1) } else { SEL.get().saturating_sub(1) };
                SEL.set(s);
                SCROLL.set(SCROLL.get().min(s).max((s + 1).saturating_sub(ROWS)));
                InvalidateRect(hw, null(), 0);
            }
        }
        WM_CHAR => {
            let c = wp as u32;
            if c == 27 {
                if CTX.with_borrow(|c| c.is_some()) {
                    CTX.set(None);
                } else if PASS.with_borrow(|p| p.is_some()) {
                    PASS.set(None);
                } else if k == Kind::Start && QUERY.with_borrow(|q| !q.is_empty()) {
                    QUERY.set(String::new()); // Esc clears the search first, the next one closes
                    SCROLL.set(0);
                    SEL.set(0);
                } else {
                    close();
                    return 0;
                }
            } else if k == Kind::Wifi && PASS.with_borrow(|p| p.is_some()) {
                if c == 13 {
                    if let Some((i, p)) = PASS.take() {
                        NETS.with_borrow(|n| wifi_join(&n[i], &p));
                    }
                } else if let Some(p) = PASS.with_borrow_mut(|p| p.as_mut().map(|p| &mut p.1 as *mut String)) {
                    let p = &mut *p;
                    if c == 8 {
                        p.pop();
                    } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                        p.push(ch);
                    }
                }
            } else if k == Kind::Start {
                if c == 13 {
                    if launch(SEL.get()) {
                        close();
                        return 0;
                    }
                } else {
                    QUERY.with_borrow_mut(|q| {
                        let all = SELALL.replace(false);
                        if c == 1 {
                            SELALL.set(!q.is_empty()); // Ctrl+A
                        } else if c == 127 {
                            // Ctrl+Backspace: the last word
                            if all { q.clear() } else { let t = q.trim_end().len(); q.truncate(q[..t].rfind(' ').map_or(0, |i| i + 1)); }
                        } else if c == 8 {
                            if all { q.clear() } else { q.pop(); }
                        } else if let Some(ch) = char::from_u32(c).filter(|_| c >= 32) {
                            if all { q.clear() }
                            q.push(ch);
                        }
                    });
                    SCROLL.set(0);
                    SEL.set(0);
                    find_loose_exes(hw);
                }
            }
            InvalidateRect(hw, null(), 0);
        }
        _ => return DefWindowProcW(hw, msg, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{flip_layout, score, init, is_open, toggle, close_now, Kind, FLY, FH, GetModuleHandleW, SetProcessDPIAware, null, null_mut, DestroyWindow, IsWindow};

    #[test]
    fn layout_flip_both_ways() {
        assert_eq!(flip_layout("сркщьу"), "chrome");
        assert_eq!(flip_layout("ntktuhfv"), "телеграм");
        assert_eq!(flip_layout("vs code"), "мы сщву");
    }

    /// The Win button always opens Start: a press right after a close is not dropped, and a state left saying "open" with no
    /// window behind it (CreateWindowExW failed) does not turn the button into a close-only no-op.
    #[test]
    fn win_button_always_opens_start() {
        unsafe {
            SetProcessDPIAware();
            init(GetModuleHandleW(null()));
            toggle(Kind::Start);
            assert!(is_open(Kind::Start), "the first press opens Start");
            toggle(Kind::Start);
            assert!(!is_open(Kind::Start), "the second press closes it");
            toggle(Kind::Start);
            assert!(is_open(Kind::Start), "a press right after the close opens it again");
            let live = FH.get();
            assert!(!live.is_null() && IsWindow(live) != 0);
            DestroyWindow(live);
            close_now();
            // the window is gone but the state still says "open" (what a failed creation or a kill behind our back leaves)
            FLY.set(Some(Kind::Start));
            FH.set(null_mut());
            toggle(Kind::Start);
            assert!(is_open(Kind::Start) && IsWindow(FH.get()) != 0, "the press opens a real panel");
            close_now();
        }
    }

    #[test]
    fn ranking() {
        assert_eq!(score("visual studio code", "vis"), Some(0));
        assert_eq!(score("visual studio code", "cod"), Some(1));
        assert_eq!(score("visual studio code", "vsc"), Some(2));
        assert_eq!(score("calculator", "cul"), Some(3));
        assert_eq!(score("calculator", "xyz"), None);
        assert_eq!(score("calculator", ""), None);
        assert_eq!(score("v", "v"), Some(0)); // one-letter initials do not count, the prefix does
    }
}
