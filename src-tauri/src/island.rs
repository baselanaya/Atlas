// Island window: placement on the chosen display, the two window sizes
// (full panel / invisible wake strip), click-through and the cursor poll.
//
// There is no notch on a PC, so the island is a black shape drawn at the top
// centre of the main display inside a borderless, transparent, always-on-top
// window that never takes focus. On Linux this needs the X11 backend (see
// main.rs): a Wayland client cannot place itself, cannot stay above, and cannot
// ask for the cursor position outside its own windows — an X11 client can do
// all three, so the app runs through XWayland.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(windows)]
use windows::core::BOOL;
#[cfg(windows)]
use windows::Win32::Foundation::LPARAM;
#[cfg(windows)]
use windows::Win32::System::Ole::RevokeDragDrop;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

/// Logical size of the full window — the largest island view, like the macOS panel.
pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;
/// Logical size of the invisible strip that wakes the island when it is hidden.
pub const STRIP_W: f64 = 240.0;
pub const STRIP_H: f64 = 6.0;

pub const WINDOW_LABEL: &str = "island";

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// Where the mouse is and whether the left button is down, in global screen
/// coordinates. One query per poll tick gives the island everything: eyes,
/// click-through and drag detection.
#[derive(Clone, Copy)]
pub struct PointerState {
    pub x: f64,
    pub y: f64,
    pub button1: bool,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read — an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// Mirrors the window flag so we only call into the platform when it changes.
    ignoring: AtomicBool,
    /// The last input shape applied on Linux (rect, whole-panel-during-drag);
    /// the X server, not a flag race, routes the clicks.
    #[cfg_attr(windows, allow(dead_code))]
    applied_shape: Mutex<Option<(IslandRect, bool)>>,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            ignoring: AtomicBool::new(false),
            applied_shape: Mutex::new(None),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
        #[cfg(unix)]
        {
            *self.applied_shape.lock().unwrap() = None;
        }
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn wait_until_active(&self) {
        let mut guard = self.active.lock().unwrap();
        while !*guard {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

#[cfg(windows)]
fn pointer_state() -> Option<PointerState> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    let button1 = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 };
    Some(PointerState { x: p.x as f64, y: p.y as f64, button1 })
}

/// XQueryPointer on the root window. The display is opened once per thread and
/// kept for the app's lifetime: opening per tick would be the expensive way to
/// do a 60 Hz poll.
#[cfg(unix)]
fn pointer_state() -> Option<PointerState> {
    use std::cell::RefCell;

    use x11::xlib::{XDefaultRootWindow, XOpenDisplay, XQueryPointer};

    thread_local! {
        static DISPLAY: RefCell<Option<*mut x11::xlib::Display>> = const { RefCell::new(None) };
    }

    DISPLAY.with(|slot| {
        let mut guard = slot.borrow_mut();
        let display = match *guard {
            Some(d) => d,
            None => {
                let d = unsafe { XOpenDisplay(std::ptr::null()) };
                if d.is_null() {
                    return None;
                }
                *guard = Some(d);
                d
            }
        };

        let root = unsafe { XDefaultRootWindow(display) };
        let mut root_return = 0u64;
        let mut child_return = 0u64;
        let mut root_x = 0i32;
        let mut root_y = 0i32;
        let mut win_x = 0i32;
        let mut win_y = 0i32;
        let mut mask = 0u32;
        let ok = unsafe {
            XQueryPointer(
                display,
                root,
                &mut root_return,
                &mut child_return,
                &mut root_x,
                &mut root_y,
                &mut win_x,
                &mut win_y,
                &mut mask,
            )
        };
        if ok == 0 {
            return None;
        }
        // Button1Mask in X.h is 1 << 8.
        Some(PointerState {
            x: root_x as f64,
            y: root_y as f64,
            button1: mask & 0x100 != 0,
        })
    })
}

/// The clickable island, on Linux: an X11 input shape over the island rect.
///
/// tao's set_ignore_cursor_events clamps the GTK input shape to a 1×1 region
/// and restores it through a different layer, which can leave the window dead
/// to clicks after the first toggle — so on Linux the shape is set here,
/// directly and deterministically: clicks land on the island (plus the hit
/// margin), pass through the transparent panel everywhere else, and the whole
/// panel takes the mouse while a drag (a file being dropped) is in flight.
/// XShape lives in libXext and has no bindings in the x11 crate, so the one
/// call the island needs is declared here.
#[cfg(unix)]
mod shape {
    use std::os::raw::c_int;
    use x11::xlib::{Display, Window, XRectangle};

    #[link(name = "Xext")]
    extern "C" {
        pub fn XShapeCombineRectangles(
            display: *mut Display,
            dest: Window,
            dest_kind: c_int,
            x_offset: c_int,
            y_offset: c_int,
            rectangles: *mut XRectangle,
            n: c_int,
            ordering: c_int,
        );
    }
}

#[cfg(unix)]
fn apply_input_shape(xid: u64, rect: &IslandRect, scale: f64, full_panel: bool) {
    use x11::xlib::{XCloseDisplay, XFlush, XOpenDisplay, XRectangle};

    // X protocol constants (Xutil.h): the input shape, replaced wholesale.
    const SHAPE_INPUT: i32 = 2;
    const SHAPE_SET: i32 = 0;

    let (x, y, w, h) = if full_panel {
        (0.0f64, 0.0f64, PANEL_W * scale, PANEL_H * scale)
    } else if rect.w > 0.0 {
        (
            (rect.x - HIT_MARGIN) * scale,
            (rect.y - HIT_MARGIN) * scale,
            (rect.w + 2.0 * HIT_MARGIN) * scale,
            (rect.h + 2.0 * HIT_MARGIN) * scale,
        )
    } else {
        // Nothing laid out yet: the server default — the whole window — will
        // do until the frontend pushes the first rect.
        return;
    };
    let mut rects = [XRectangle {
        x: x.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16,
        y: y.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16,
        width: w.round().clamp(1.0, u16::MAX as f64) as u16,
        height: h.round().clamp(1.0, u16::MAX as f64) as u16,
    }];
    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return;
        }
        shape::XShapeCombineRectangles(
            display,
            xid,
            SHAPE_INPUT,
            0,
            0,
            rects.as_mut_ptr(),
            1,
            SHAPE_SET,
        );
        XFlush(display);
        XCloseDisplay(display);
    }
}

/// The X window id of a webview window, for the input shape above.
#[cfg(unix)]
fn x11_window_id(win: &WebviewWindow) -> Option<u64> {
    use raw_window_handle::HasWindowHandle;
    let handle = win.window_handle().ok()?;
    match handle.as_raw() {
        raw_window_handle::RawWindowHandle::Xlib(x) => Some(x.window),
        raw_window_handle::RawWindowHandle::Xcb(x) => Some(x.window.get() as u64),
        _ => None,
    }
}

/// Lets dropped files reach the app again.
///
/// wry installs its drop target by walking the webview's child windows **once**,
/// when the webview is created. WebView2 creates `Chrome_RenderWidgetHostHWND`
/// later and registers its own target on it; being the innermost window, that one
/// wins, and since the page has no HTML5 drop handler it refuses everything — the
/// "no drop" cursor, with nothing reaching Tauri. Revoking it makes OLE fall
/// through to the target wry registered on the parent widget, which is the one
/// that feeds Tauri's drag events.
///
/// Cheap and idempotent, so it is simply re-run whenever a drag might be starting.
#[cfg(windows)]
pub fn unblock_webview_drops(app: &AppHandle) {
    for label in [WINDOW_LABEL, "settings"] {
        let Some(win) = app.get_webview_window(label) else { continue };
        let Some(hwnd) = hwnd_of(&win) else { continue };
        unsafe {
            let _ = EnumChildWindows(Some(hwnd), Some(revoke_render_widget), LPARAM(0));
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn revoke_render_widget(hwnd: HWND, _: LPARAM) -> BOOL {
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut name) };
    if len > 0 {
        let class = String::from_utf16_lossy(&name[..len as usize]);
        if class == "Chrome_RenderWidgetHostHWND" {
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
    }
    true.into()
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// The display the island lives on: the primary one, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some(pointer) = pointer_state() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, pointer.x, pointer.y)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// Places and sizes the window. `collapsed` picks the wake strip instead of the panel.
pub fn apply_geometry(app: &AppHandle, pref: &str, collapsed: bool) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let (lw, lh) = if collapsed { (STRIP_W, STRIP_H) } else { (PANEL_W, PANEL_H) };
    let pw = (lw * scale).round().max(1.0) as u32;
    let ph = (lh * scale).round().max(1.0) as u32;
    let x = mp.x + (ms.width as i32 - pw as i32) / 2;
    let y = mp.y;

    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
}

#[cfg(windows)]
fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut _))
}

/// WS_EX_NOACTIVATE keeps clicks from stealing focus; WS_EX_TOOLWINDOW keeps the
/// island out of Alt-Tab.
#[cfg(windows)]
pub fn make_non_activating(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Temporarily allow activation so a text field inside the island can be typed in.
#[cfg(windows)]
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if activating {
            ex & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// GTK has no WS_EX_NOACTIVATE equivalent exposed through Tauri; skip_taskbar in
/// the window config keeps the island out of the taskbar, and focus handling is
/// left to the window manager for now.
#[cfg(unix)]
pub fn make_non_activating(_win: &WebviewWindow) {}

#[cfg(unix)]
pub fn set_activating(_win: &WebviewWindow, _activating: bool) {}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

/// Emits `cursor` (window-logical coordinates) at ~60 Hz while the island is
/// visible. Parked on a condvar the rest of the time.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        let mut was_down = false;
        // Remembered across wakes so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        loop {
            gate.wait_until_active();
            let mut last = (f64::MIN, f64::MIN);
            let mut ticks: u32 = 0;
            while gate.is_active() {
                std::thread::sleep(Duration::from_millis(16));

                // Monitors get plugged in, unplugged, rearranged and rescaled, and
                // an island pinned to coordinates that no longer exist is an island
                // nobody can reach. Checked about twice a second — the cursor poll
                // is already running, so this costs one monitor query.
                ticks = ticks.wrapping_add(1);
                if ticks % 30 == 0 {
                    let now = current_screen_key(&app);
                    if now.is_some() && now != last_screen {
                        let first = last_screen.is_none();
                        last_screen = now;
                        if !first {
                            crate::log::line("display layout changed — repositioning".to_string());
                            let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                        }
                    }
                }

                let Some(win) = window(&app) else { continue };
                let Ok(origin) = win.outer_position() else { continue };
                let scale = win.scale_factor().unwrap_or(1.0);
                let Some(pointer) = pointer_state() else { continue };
                let x = (pointer.x - origin.x as f64) / scale;
                let y = (pointer.y - origin.y as f64) / scale;
                let size = match win.inner_size() {
                    Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                    Err(_) => (PANEL_W, PANEL_H),
                };
                if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                    continue;
                }
                last = (x, y);

                // Click-through: the window only takes the mouse over the island
                // shape. A small entry margin means the flag is already off by the
                // time a moving cursor reaches a button.
                let r = *gate.rect.lock().unwrap();
                #[cfg(windows)]
                let on_island = r.w > 0.0
                    && x >= r.x - HIT_MARGIN
                    && x <= r.x + r.w + HIT_MARGIN
                    && y >= r.y - HIT_MARGIN
                    && y <= r.y + r.h + HIT_MARGIN;

                // A file being dragged has to be able to find us. On Windows,
                // WS_EX_TRANSPARENT — what click-through is there — hides the
                // window from WindowFromPoint, so OLE finds no drop target and
                // shows the "no drop" cursor. macOS has no such problem: AppKit
                // delivers drags to registered destinations whatever
                // ignoresMouseEvents says. So while a button is held anywhere over
                // the panel, the whole panel takes the mouse, which also makes the
                // drop zone as forgiving as the Mac's. A press may be the start of
                // a drag: make sure the drop target is ours before the file
                // arrives. webkit2gtk needs none of this — HTML5 drops reach the
                // page without a fight.
                let down = pointer.button1;
                #[cfg(windows)]
                {
                    if down && !was_down {
                        let handle = app.clone();
                        let _ = app.run_on_main_thread(move || unblock_webview_drops(&handle));
                    }
                    was_down = down;
                }

                let dragging = down
                    && x >= 0.0
                    && x <= size.0
                    && y >= 0.0
                    && y <= size.1;

                #[cfg(windows)]
                {
                    let accept = on_island || dragging;
                    if gate.ignoring.load(Ordering::Relaxed) == accept {
                        gate.ignoring.store(!accept, Ordering::Relaxed);
                        let _ = win.set_ignore_cursor_events(!accept);
                    }
                }
                #[cfg(unix)]
                {
                    // The X server routes by the input shape, so there is no
                    // flag to race a click against — only a shape to keep
                    // current as the island grows and shrinks.
                    let key = (r, dragging);
                    let mut applied = gate.applied_shape.lock().unwrap();
                    if applied.map_or(true, |prev| prev != key) {
                        if let Some(xid) = x11_window_id(&win) {
                            apply_input_shape(xid, &r, scale, dragging);
                            *applied = Some(key);
                        }
                    }
                }

                let _ = win.emit("cursor", CursorPayload { x, y });
            }
        }
    });
}

/// KWin (and other Linux WMs) can drop position requests made while the window
/// is still being mapped for the first time, so the island is placed once more
/// after the WM has certainly had its say. On Windows the first placement is
/// always honored and this is never needed.
pub fn pin_after_show(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(350));
        let pref = app
            .try_state::<crate::Shared>()
            .map(|s| s.settings.lock().unwrap().screen.clone())
            .unwrap_or_else(|| "primary".into());
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || apply_geometry(&handle, &pref, false));
    });
}

pub fn set_ignore_cursor(app: &AppHandle, ignore: bool) {
    #[cfg(windows)]
    if let Some(win) = window(app) {
        let _ = win.set_ignore_cursor_events(ignore);
    }
    #[cfg(unix)]
    {
        // The input shape owns this on Linux; after a resize the poll re-applies
        // it within a tick (see forget_ignore_state).
        let _ = (app, ignore);
    }
}
