//! Orby on macOS (phase 1): the orb on Metal, its dial, pills and the Prayer panel in
//! borderless non-activating panels, a menu bar item and Ctrl+Option+Space.
//!
//! - app.rs: the state, the run loop glue, the orb's docking and tuck, the dial, pills and panel
//! - cg.rs: the shared `Canvas` on Core Graphics
//! - text.rs: Inter through Core Text
//! - metal.rs / metal.msl: the liquid-metal drop
//! - hotkey.rs: the global shortcut through Carbon (no Accessibility permission)
//! - clock.rs: local time and the settings folder
//! - disk.rs / disk_win.rs: the Disk window and what Disk asks of the system (disk_sys)
//! - login.rs: start at login (SMAppService)

pub mod app;
pub mod cg;
pub mod clock;
pub mod disk;
pub mod disk_win;
pub mod hotkey;
pub mod login;
pub mod metal;
pub mod text;

use objc2_app_kit::NSWorkspace;

/// System Settings, Accessibility, Display, Reduce motion.
pub fn reduced_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let snap = std::env::var_os("ORB_SNAPSHOT")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            args.iter()
                .position(|a| a == "--snapshot")
                .and_then(|i| args.get(i + 1))
                .map(std::path::PathBuf::from)
        });
    if let Some(dir) = snap {
        crate::snapshot::run_mac(&dir);
        return;
    }
    app::run();
}
