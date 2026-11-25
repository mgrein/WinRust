//! topbar: a bar on every other monitor (shares main.rs's items through `super::*`). Like the Windows 11 taskbar on a
//! second screen: Start, search, Task View, the app buttons and the clock, no tray or status icons. It paints with the
//! main bar's painter and runs the same clicks; flyouts and menus open on its monitor.
use super::*;

thread_local! {
    /// The second-monitor bar whose message is being handled (null: the main bar). layout() leaves the tray out for it.
    static ON2: Cell<HWND> = const { Cell::new(null_mut()) };
    static BARS: RefCell<Vec<(HWND, RECT)>> = const { RefCell::new(Vec::new()) }; // each bar and its monitor
    static HOT2: Cell<i32> = const { Cell::new(NONE) };
    static HV2: RefCell<HashMap<i32, (f32, u64)>> = RefCell::new(HashMap::new()); // their hover fades, apart from the main bar's
}

const CLASS: &str = "topbar_second";

pub(crate) fn on2() -> HWND {
    ON2.get()
}

fn same(a: RECT, b: RECT) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

pub(crate) unsafe fn register() {
    let c = wide(CLASS);
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc2), hInstance: GetModuleHandleW(null()), hCursor: LoadCursorW(null_mut(), IDC_ARROW), lpszClassName: c.as_ptr(), ..zeroed() });
}

unsafe extern "system" fn add_mon(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> i32 {
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(m, &mut mi) != 0 && mi.dwFlags & MONITORINFOF_PRIMARY == 0 {
        (*(lp as *mut Vec<RECT>)).push(mi.rcMonitor);
    }
    1
}

/// One bar per non-primary monitor, placed for the current size and style: made, moved or closed as monitors come and go.
/// Side bars (left / right) stay on the main monitor only.
pub(crate) unsafe fn sync() {
    let mut want: Vec<RECT> = Vec::new();
    if !vert() {
        EnumDisplayMonitors(null_mut(), null(), Some(add_mon), &mut want as *mut Vec<RECT> as LPARAM);
    }
    fly::ORIGIN.set(None);
    let gone: Vec<HWND> = BARS.with_borrow(|b| b.iter().filter(|(_, m)| !want.iter().any(|&w| same(w, *m))).map(|b| b.0).collect());
    for h in gone {
        DestroyWindow(h);
    }
    for m in want {
        let h = BARS.with_borrow(|b| b.iter().find(|(_, x)| same(*x, m)).map(|b| b.0)).unwrap_or_else(|| create(m));
        if !h.is_null() {
            place2(h, m);
        }
    }
}

pub(crate) unsafe fn invalidate_all() {
    BARS.with_borrow(|b| b.iter().for_each(|x| _ = InvalidateRect(x.0, null(), 0)));
}

pub(crate) unsafe fn destroy_all() {
    let all: Vec<HWND> = BARS.with_borrow(|b| b.iter().map(|x| x.0).collect());
    for h in all {
        DestroyWindow(h);
    }
}

unsafe fn create(m: RECT) -> HWND {
    let c = wide(CLASS);
    let h = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_LAYERED, c.as_ptr(), c.as_ptr(), WS_POPUP, m.left, m.bottom - bar_h(), m.right - m.left, bar_h(), null_mut(), null_mut(), GetModuleHandleW(null()), null());
    if h.is_null() {
        return h;
    }
    BARS.with_borrow_mut(|b| b.push((h, m)));
    ChangeWindowMessageFilterEx(h, CALLBACK, MSGFLT_ALLOW, null_mut());
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = h;
    ab.uCallbackMessage = CALLBACK;
    SHAppBarMessage(ABM_NEW, &mut ab);
    SetTimer(h, 1, 1000, None); // the clock
    SetTimer(h, 3, 300, None); // fullscreen watch
    h
}

/// The bar along the bottom of monitor `m`, with that monitor's work area ending above it.
unsafe fn place2(h: HWND, m: RECT) {
    apply_opacity(h);
    let strip = if dock() || i18n::style::autohide() { 0 } else { bar_h() };
    let mut ab: APPBARDATA = zeroed();
    ab.cbSize = size_of::<APPBARDATA>() as u32;
    ab.hWnd = h;
    ab.uEdge = ABE_BOTTOM;
    ab.rc = RECT { left: m.left, top: m.bottom - strip, right: m.right, bottom: m.bottom };
    SHAppBarMessage(ABM_SETPOS, &mut ab);
    set_monitor_work_area(m, RECT { left: m.left, top: m.top, right: m.right, bottom: m.bottom - strip });
    SetWindowPos(h, HWND_TOPMOST, m.left, m.bottom - bar_h(), m.right - m.left, bar_h(), SWP_NOACTIVATE | SWP_SHOWWINDOW);
    InvalidateRect(h, null(), 0);
}

/// SPI_SETWORKAREA with a rectangle inside a monitor sets that monitor's work area (Explorer would, but it may be absent).
unsafe fn set_monitor_work_area(m: RECT, mut want: RECT) {
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromRect(&m, MONITOR_DEFAULTTONEAREST), &mut mi);
    if same(mi.rcWork, want) {
        return;
    }
    SystemParametersInfoW(SPI_SETWORKAREA, 0, &mut want as *mut _ as *mut _, 0);
    PostMessageW(HWND_BROADCAST, WM_SETTINGCHANGE, SPI_SETWORKAREA as usize, 0);
    EnumWindows(Some(fit_maximized), &want as *const RECT as isize);
}

unsafe extern "system" fn proc2(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let prev = ON2.replace(hwnd);
    let r = handle(hwnd, msg, wp, lp);
    ON2.set(prev);
    r
}

unsafe fn handle(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let mon = || BARS.with_borrow(|b| b.iter().find(|x| x.0 == hwnd).map(|x| x.1));
    match msg {
        WM_PAINT => {
            // the main bar's painter, with this bar's hover state swapped in
            let hot = HOT.replace(HOT2.get());
            HV.with_borrow_mut(|a| HV2.with_borrow_mut(|b| std::mem::swap(a, b)));
            paint(hwnd);
            HV.with_borrow_mut(|a| HV2.with_borrow_mut(|b| std::mem::swap(a, b)));
            HOT.set(hot);
        }
        WM_ERASEBKGND => return 1,
        WM_TIMER if wp == 3 => {
            // out of the way of a fullscreen game or video on its monitor, like the main bar
            let fs = mon().is_some_and(|m| fg_fullscreen(hwnd).is_some_and(|f| same(f, m)));
            let shown = IsWindowVisible(hwnd) != 0;
            if fs && shown {
                ShowWindow(hwnd, SW_HIDE);
            } else if !fs && !shown {
                ShowWindow(hwnd, SW_SHOWNA);
            }
        }
        WM_TIMER => _ = InvalidateRect(hwnd, null(), 0), // the clock, and the next frame of a hover fade
        WM_MOUSEMOVE => {
            let h = hit(hwnd, (lp & 0xffff) as i16 as i32, (lp >> 16) as i16 as i32);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            if HOT2.replace(h) != h {
                InvalidateRect(hwnd, null(), 0);
            }
        }
        WM_MOUSELEAVE => {
            HOT2.set(NONE);
            InvalidateRect(hwnd, null(), 0);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            let x = (lp & 0xffff) as i16 as i32;
            let h = hit(hwnd, x, (lp >> 16) as i16 as i32);
            fly::ORIGIN.set(mon());
            thumbs::hide_preview();
            if h != NONE {
                press(h);
            }
            if h == NONE && msg == WM_RBUTTONDOWN {
                GEAR_AT.store(x, Relaxed);
                fly::context_menu(x);
            } else if h >= 1 && msg == WM_RBUTTONDOWN {
                app_menu(hwnd, h);
            } else {
                click(h, msg == WM_LBUTTONDOWN, msg == WM_MBUTTONDOWN);
            }
            InvalidateRect(hwnd, null(), 0);
        }
        CALLBACK if wp == ABN_POSCHANGED as usize => {
            if let Some(m) = mon() {
                place2(hwnd, m);
            }
        }
        WM_DESTROY => {
            let mut ab: APPBARDATA = zeroed();
            ab.cbSize = size_of::<APPBARDATA>() as u32;
            ab.hWnd = hwnd;
            SHAppBarMessage(ABM_REMOVE, &mut ab);
            if let Some(m) = mon() {
                set_monitor_work_area(m, m); // the strip goes back to the monitor
            }
            BARS.with_borrow_mut(|b| b.retain(|x| x.0 != hwnd));
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}
