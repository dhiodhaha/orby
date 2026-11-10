//! Orby: the mercury drop, its pill, dial and panels.
//!
//! One crate for both systems. The drawing of the pill, the dial and the Prayer panel
//! (canvas.rs, ui.rs, panel.rs, motion.rs) and the plain models are shared; Windows keeps its
//! window plumbing in the modules below (app_windows.rs holds its `App` and loop), and macOS
//! lives in mac/ (AppKit, Core Graphics, Core Text, Metal).
//!
//! On Windows:
//!
//! This file owns the state (`App`), the shared ids, the orb window's message dispatch and
//! the loop. Each window's behaviour lives in its own module:
//! - orb.rs: the drop (shader, springs, docking, tuck, drag)
//! - pill.rs: the notification pill
//! - dial.rs: the half dial and its picks
//! - prayer.rs: prayer times and reminders (panel.rs draws the Prayer panel)
//! - habits.rs: focus and drink on the app_habits engines (focus_panel.rs, drink_panel.rs draw)
//! - panels.rs: the one panel window that shows Prayer, Focus or Drink
//! - store.rs: the drink log and focus sessions on disk
//! - tasks.rs: the Tasks panel, due pills and the overdue badge (tasks_model.rs decides,
//!   tasks_panel.rs draws, textfield.rs edits the quick add line); tasks_window.rs is the Tasks
//!   window on the same store (tasks_window_panel.rs draws it)
//! - calendar.rs: the calendar link, its 10 min reads, event pills and the Today panel
//!   (today_model.rs decides, today_panel.rs draws)
//! - downloads.rs: the clipboard offer, the download queue on yt-dlp, its ring, pills, panel and
//!   format menu (downloads_model.rs decides, downloads_panel.rs draws)
//! - disk.rs: the low-space pill, scans, results, Safe to clean, Move to Recycle Bin and Undo
//!   in its own window (disk_window.rs; disk_model.rs decides, disk_panel.rs draws,
//!   disk_restore.rs puts items back)
//! - history.rs: the History panel, drink and focus by day (history_model.rs decides,
//!   history_panel.rs draws)
//! - notices.rs: the notice queue every pill goes through, quiet times, the badge count and the
//!   Notices panel (notices_model.rs decides, notices_panel.rs draws)
//! - settings.rs: the Settings window and the first run, settings.json, the shortcut, start with
//!   Windows, Windows location (settings_model.rs decides, settings_panel.rs draws)
//! - hook.rs: the low-level mouse hook
//! - tray.rs: the tray icon and menu commands
//!
//! Renders only while something moves. At rest the process sleeps in GetMessage.

#![cfg_attr(windows, windows_subsystem = "windows")]
// Shared models carry what the Windows panels need; the macOS app uses part of them so far.
#![cfg_attr(not(windows), allow(dead_code))]

// Shared by both systems.
mod canvas;
mod motion;
mod notices_model;
mod panel;
mod png;
mod settings_model;
mod snapshot;
mod ui;

// Windows: Direct2D, DirectComposition and Win32 windows.
#[cfg(windows)]
mod calendar;
#[cfg(windows)]
mod depth;
#[cfg(windows)]
mod dial;
#[cfg(windows)]
mod disk;
#[cfg(windows)]
mod disk_model;
#[cfg(windows)]
mod disk_panel;
#[cfg(windows)]
mod disk_restore;
#[cfg(windows)]
mod disk_window;
#[cfg(windows)]
mod downloads;
#[cfg(windows)]
mod downloads_model;
#[cfg(windows)]
mod downloads_panel;
#[cfg(windows)]
mod drink_panel;
#[cfg(windows)]
mod focus_panel;
#[cfg(windows)]
mod frame;
#[cfg(windows)]
mod gfx;
#[cfg(windows)]
mod habits;
#[cfg(windows)]
mod history;
#[cfg(windows)]
mod history_model;
#[cfg(windows)]
mod history_panel;
#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod khgt;
#[cfg(windows)]
mod morph;
#[cfg(windows)]
mod notices;
#[cfg(windows)]
mod notices_panel;
#[cfg(windows)]
mod orb;
#[cfg(windows)]
mod panels;
#[cfg(windows)]
mod pill;
#[cfg(windows)]
mod prayer;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod settings_panel;
#[cfg(windows)]
mod store;
#[cfg(windows)]
mod tasks;
#[cfg(windows)]
mod tasks_model;
#[cfg(windows)]
mod tasks_panel;
#[cfg(windows)]
mod tasks_window;
#[cfg(windows)]
mod tasks_window_panel;
#[cfg(windows)]
mod textfield;
#[cfg(windows)]
mod today_model;
#[cfg(windows)]
mod today_panel;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
include!("app_windows.rs");

// macOS: AppKit panels, Core Graphics and Core Text, the orb on Metal.
#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::run();
}
