//! What Settings and the first run (settings.rs) ask of the system they run on, like disk_sys
//! for Disk: the window (show, hide, size, draw, take the keyboard), the global shortcut
//! (register, let go, probe), start at login, a location fix, the system's light or dark, the
//! other windows' redraw on a theme change, and opening the system's own settings pages.
//!
//! Windows: `settings_window.rs` (the Win32 window, RegisterHotKey, the HKCU Run entry, the WinRT
//! Geolocator, the adzan file picker). macOS: `mac/settings_win.rs` (an NSWindow, a Carbon hot
//! key, SMAppService; no location fix and no adzan file yet).
//!
//! Both provide:
//! - `Win` (the window's own state, in `app.settings.win`), `HAS_LOCATION`
//! - `place`, `show(app, activate)`, `hide`, `take_foreground`, `prepare`, `paint(app, &View)`
//! - `register(app, s) -> bool`, `unregister`, `is_free(app, s) -> bool`
//! - `read_startup() -> Startup`, `write_startup(app, on) -> bool`
//! - `find_location(app, tx)` (sends one `Locate`, then wakes `settings::on_located`)
//! - `open_startup_settings`, `open_location_settings`, `open_folder(path)`, `pick_adzan(app, subuh)`
//! - `system_dark()`, `redraw_others(app)`, `no_first_run()`

#[cfg(target_os = "macos")]
pub(crate) use crate::mac::settings_win::*;

#[cfg(windows)]
pub(crate) use crate::settings_window::*;
