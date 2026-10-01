// Atlas runs without a console window: Atlas is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    prefer_x11();
    atlas_lib::run()
}

/// The island needs three things a Wayland client cannot have: to place itself
/// at the top of the screen, to stay above everything, and to ask for the
/// cursor position outside its own windows (XQueryPointer, the counterpart of
/// the Windows build's GetCursorPos poll). Through XWayland all three work, so
/// GTK is pointed at the X11 backend — before anything initialises it.
///
/// This deliberately overrides an inherited GDK_BACKEND: desktop applications
/// often export GDK_BACKEND=wayland, and respecting it would strand the island
/// in the middle of the screen with no way to move it.
#[cfg(target_os = "linux")]
fn prefer_x11() {
    std::env::set_var("GDK_BACKEND", "x11");
    // webkit2gtk's accelerated compositing breaks transparent windows on many
    // drivers; this is the standard workaround for overlay apps.
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
}
