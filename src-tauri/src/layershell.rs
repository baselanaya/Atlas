//! Layer-shell for the island on native Wayland — the waybar route to a
//! surface that lives at the top edge, above everything, positioned by the
//! compositor instead of fighting it.
//!
//! gtk-layer-shell is loaded at runtime (dlopen) so the binary runs fine
//! wherever the library is missing — the island simply falls back to XWayland,
//! which main.rs decides before anything initializes GTK. Nothing here runs
//! unless the Wayland backend was actually chosen.

#![cfg(unix)]

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use libc::{dlopen, dlsym, RTLD_LAZY, RTLD_LOCAL};

// GtkLayerShellLayer
const LAYER_OVERLAY: i32 = 3;
// GtkLayerShellEdge
const EDGE_LEFT: i32 = 0;
const EDGE_TOP: i32 = 2;

type CWindow = *mut std::ffi::c_void;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// dlsym, keeping the raw pointer so a missing symbol stays detectable.
unsafe fn symbol(handle: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
    let c = CString::new(name).unwrap();
    dlsym(handle, c.as_ptr())
}

/// Can the library be loaded at all? Cheap, GTK-free — safe to call from main
/// before anything initializes.
pub fn library_present() -> bool {
    open_library().is_some()
}

fn open_library() -> Option<*mut std::ffi::c_void> {
    for name in ["libgtk-layer-shell.so.0", "libgtk-layer-shell.so"] {
        let c = CString::new(name).unwrap();
        let handle = unsafe { dlopen(c.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
        if !handle.is_null() {
            return Some(handle);
        }
    }
    None
}

/// Turns the island window into a layer surface pinned to the top edge,
/// horizontally centered by a left margin, on the overlay layer. Must run
/// before the window is mapped. Returns false when anything is missing, in
/// which case the caller logs and the island behaves like a plain window.
pub fn try_init(window: CWindow, center_margin: i32, width: i32, height: i32) -> bool {
    let Some(lib) = open_library() else {
        crate::log::line("layer-shell: library not found — island runs as a plain window".to_string());
        return false;
    };

    unsafe {
        let supported = symbol(lib, "gtk_layer_is_supported");
        if supported.is_null()
            || std::mem::transmute::<*mut std::ffi::c_void, extern "C" fn() -> i32>(supported)() == 0
        {
            crate::log::line("layer-shell: compositor does not support the protocol".to_string());
            return false;
        }

        let init = symbol(lib, "gtk_layer_init_for_window");
        let set_layer = symbol(lib, "gtk_layer_set_layer");
        let set_anchor = symbol(lib, "gtk_layer_set_anchor");
        let set_margin = symbol(lib, "gtk_layer_set_margin");
        let set_zone = symbol(lib, "gtk_layer_set_exclusive_zone");
        let set_size = symbol(lib, "gtk_layer_set_size");
        if init.is_null() || set_layer.is_null() || set_anchor.is_null() || set_margin.is_null() || set_zone.is_null() {
            crate::log::line("layer-shell: missing symbols in the loaded library".to_string());
            return false;
        }

        let init: extern "C" fn(CWindow) = std::mem::transmute(init);
        let set_layer: extern "C" fn(CWindow, i32) = std::mem::transmute(set_layer);
        let set_anchor: extern "C" fn(CWindow, i32, i32) = std::mem::transmute(set_anchor);
        let set_margin: extern "C" fn(CWindow, i32, i32) = std::mem::transmute(set_margin);
        let set_zone: extern "C" fn(CWindow, i32) = std::mem::transmute(set_zone);

        init(window);
        set_layer(window, LAYER_OVERLAY);
        set_anchor(window, EDGE_TOP, 1);
        set_anchor(window, EDGE_LEFT, 1);
        set_margin(window, EDGE_TOP, 0);
        set_margin(window, EDGE_LEFT, center_margin.max(0));
        // Not a panel: never reserve screen space.
        set_zone(window, -1);
        if !set_size.is_null() {
            let set_size: extern "C" fn(CWindow, i32, i32) = std::mem::transmute(set_size);
            set_size(window, width, height);
        }
    }
    ACTIVE.store(true, Ordering::Release);
    true
}

/// True once the island runs as a layer surface (native Wayland mode).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

// ── the GtkWindow hunt ────────────────────────────────────────────────────────
// tao keeps its GtkWindow to itself, but the process links GTK anyway, so the
// island is found by title through plain C symbols. The island is "Atlas"; the
// settings window is "Settings — Atlas" and never matches.

extern "C" {
    fn gtk_window_list_toplevels() -> *mut std::ffi::c_void; // GList*
    fn gtk_window_get_title(window: CWindow) -> *const std::ffi::c_char;
}

/// The island's GtkWindow, by title. Null when not found.
pub fn island_window() -> Option<CWindow> {
    unsafe {
        let list = gtk_window_list_toplevels();
        if list.is_null() {
            return None;
        }
        let mut node = list;
        let mut found = None;
        while !node.is_null() {
            // GList: { gpointer data; GList *next; GList *prev; }
            let data = *(node as *mut *mut std::ffi::c_void);
            if !data.is_null() {
                let title = gtk_window_get_title(data);
                if !title.is_null() {
                    let title = std::ffi::CStr::from_ptr(title);
                    if title.to_bytes() == b"Atlas" {
                        found = Some(data);
                        break;
                    }
                }
            }
            node = *(node.add(std::mem::size_of::<usize>() * 2) as *mut *mut std::ffi::c_void);
        }
        // gtk_window_list_toplevels owns the list; free it with g_list_free.
        let raw = libc::dlsym(libc::RTLD_DEFAULT, b"g_list_free\0".as_ptr() as *const _);
        if !raw.is_null() {
            let g_list_free: extern "C" fn(*mut std::ffi::c_void) = std::mem::transmute(raw);
            g_list_free(list);
        }
        found
    }
}
