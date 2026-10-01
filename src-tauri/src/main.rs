// Atlas runs without a console window: Atlas is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    choose_backend();
    atlas_lib::run()
}

/// Which backend the island runs on:
///
/// - Native Wayland when gtk-layer-shell can be loaded — the compositor then
///   pins the island to the top edge itself (see layershell.rs), and no
///   self-positioning or global cursor is needed. Eye tracking is the one
///   casualty: a Wayland client cannot ask for the cursor outside its windows.
/// - XWayland otherwise: a Wayland client cannot place itself, stay above, or
///   query the global cursor, but an X11 client can do all three, so GTK is
///   pointed at the X11 backend — before anything initialises it.
///
/// ATLAS_ISLAND=wayland|x11 overrides the choice. The automatic pick overrides
/// an inherited GDK_BACKEND: desktop apps often export GDK_BACKEND=wayland,
/// and respecting it blindly would strand the island mid-screen with no way to
/// move it.
#[cfg(target_os = "linux")]
fn choose_backend() {
    let forced = std::env::var("ATLAS_ISLAND").unwrap_or_default();
    let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let native = forced == "wayland" || (wayland_session && forced.is_empty() && atlas_lib::layershell::library_present());
    if native {
        std::env::set_var("GDK_BACKEND", "wayland");
    } else {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    // webkit2gtk's accelerated compositing breaks transparent windows on many
    // drivers; this is the standard workaround for overlay apps.
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
}
