//! What the Disk state machine (disk.rs) asks of the system it runs on: the log, timers, waking
//! the UI thread from a worker, the notice queue and the pill, the window, the drive list, the
//! file viewer, the folder picker, Undo out of the bin, and the local clock. disk.rs calls only
//! these, so the same scans, levels, Safe to clean, Duplicates, confirm sheet and Undo run on
//! Windows (here, over the existing Win32 modules) and on macOS (`mac::disk`).

#[cfg(target_os = "macos")]
pub(crate) use crate::mac::disk::*;

#[cfg(windows)]
pub(crate) use imp::*;

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    use crate::disk_model::Stamp;
    use crate::notices::{self, Notice, Source};
    use crate::ui::Pill;
    use crate::{App, TIMER_DISK, TIMER_DISK_RELEASE, WM_APP_DISK};

    pub(crate) use crate::disk_restore::restore;
    pub(crate) use crate::disk_window::{self as window, Win};

    pub fn log(app: &mut App, line: &str) {
        crate::log_line(app, line);
    }

    pub fn data_dir() -> PathBuf {
        crate::store::data_dir()
    }

    pub fn reduced_motion() -> bool {
        crate::downloads::reduced_motion()
    }

    /// The drop shows the scan's ring (and the ember badge).
    pub fn render_orb(app: &mut App) {
        crate::orb::render_orb(app);
    }

    /// Another ring (focus) owns the drop.
    pub fn other_ring(app: &App) -> bool {
        crate::habits::ring(app).is_some()
    }

    pub fn start_anim(app: &mut App) {
        crate::start_anim(app);
    }

    pub fn memory_mb() -> (f64, f64) {
        crate::memory_mb()
    }

    pub fn post(app: &mut App, n: Notice) {
        notices::post(app, n);
    }

    pub fn notify(app: &mut App, p: Pill) {
        notices::notify(app, Source::Disk, p);
    }

    pub fn reply(app: &mut App, p: Pill) {
        notices::reply(app, p);
    }

    /// New words for the pill on screen, in place.
    pub fn live(app: &mut App, p: Pill) {
        notices::live(app, p);
    }

    pub fn dismiss_pill(app: &mut App) {
        crate::pill::dismiss_pill(app);
    }

    /// A quiet time holds normal notices (hidden, full screen, focus, a pause).
    pub fn quiet_holds(app: &App) -> bool {
        notices::quiet(app).holds()
    }

    pub fn forget_cooldown(app: &mut App, kind: &str) {
        notices::forget_cooldown(app, kind);
    }

    /// The low-space check's timer, every `ms`.
    pub fn arm_check(app: &mut App, ms: u32) {
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_DISK, ms, None);
        }
    }

    /// Lets the scan's tree go `ms` after the window closed.
    pub fn arm_release(app: &mut App, ms: u32) {
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_DISK_RELEASE, ms, None);
        }
    }

    pub fn disarm_release(app: &mut App) {
        unsafe {
            let _ = KillTimer(Some(app.hwnd), TIMER_DISK_RELEASE);
        }
    }

    /// Wakes the UI thread from a worker: one WM_APP_DISK on the orb window.
    #[derive(Clone, Copy)]
    pub struct Waker(isize);

    impl Waker {
        pub fn wake(&self) {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.0 as *mut _)),
                    WM_APP_DISK,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    pub fn waker(app: &App) -> Waker {
        Waker(app.hwnd.0 as isize)
    }

    const DRIVE_FIXED: u32 = 3;

    /// The fixed drives, `C:\` and so on.
    pub fn fixed_drives() -> Vec<PathBuf> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
            .filter(|p| unsafe { GetDriveTypeW(&HSTRING::from(p.as_os_str())) } == DRIVE_FIXED)
            .collect()
    }

    /// "Windows", or "Local Disk" when the volume has no name.
    pub fn volume_label(root: &Path) -> String {
        let mut name = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                &HSTRING::from(root.as_os_str()),
                Some(&mut name),
                None,
                None,
                None,
                None,
            )
        }
        .is_ok();
        let n = name.iter().position(|&c| c == 0).unwrap_or(0);
        let s = String::from_utf16_lossy(&name[..n]);
        if ok && !s.is_empty() {
            s
        } else {
            "Local Disk".into()
        }
    }

    /// Where Scan again goes with nothing scanned yet.
    pub fn default_root() -> PathBuf {
        PathBuf::from("C:\\")
    }

    /// The drive the dial's Disk child and the tray test read.
    pub fn main_drive() -> PathBuf {
        PathBuf::from("C:\\")
    }

    fn shell(file: &str, params: Option<&str>) {
        let params = params.map(HSTRING::from);
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(file),
                params
                    .as_ref()
                    .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// A folder opens; a file is shown selected in its folder.
    pub fn show_in_folder(p: &Path) {
        if p.is_dir() {
            shell(&p.to_string_lossy(), None);
        } else {
            shell(
                "explorer.exe",
                Some(&format!("/select,\"{}\"", p.display())),
            );
        }
    }

    pub fn open_bin() {
        shell("shell:RecycleBinFolder", None);
    }

    pub fn open_storage_settings() {
        shell("ms-settings:storagesense", None);
    }

    /// Windows asks no permission to read folders; nothing to open.
    pub fn open_privacy_settings() {}

    /// The folder picker belongs to the Disk window, so it opens over it (after this click).
    pub fn pick_folder(app: &mut App) {
        unsafe {
            let _ = PostMessageW(
                Some(app.disk.win.hwnd),
                WM_APP_DISK,
                WPARAM(crate::disk::PICK_FOLDER),
                LPARAM(0),
            );
        }
    }

    pub fn toggle_max(app: &mut App) {
        crate::frame::toggle(app.disk.win.hwnd);
    }

    /// A local date and minute for a moment.
    pub fn stamp(t: SystemTime) -> Stamp {
        let secs = t
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let mo = crate::store::local(secs);
        let d = mo.date();
        Stamp {
            year: d.year(),
            month: d.month(),
            day: d.day(),
            minute: mo.minute_of_day(),
        }
    }
}
