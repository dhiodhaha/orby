//! The macOS half of `disk_sys`: what Disk asks of the system, on AppKit. The window itself is
//! `disk_win.rs`. Nothing here deletes: the only move is app_disk's Move to Trash, and Undo is
//! app_disk's Put Back (a rename out of the Trash that never overwrites).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, sel};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL};

use super::clock;
use crate::disk_model::Stamp;
use crate::notices_model::{Notice, Priority, Source};
use crate::ui::Pill;
use crate::App;

pub(crate) use super::disk_win::{self as window, Win};

pub fn log(app: &mut App, line: &str) {
    app.log(line);
}

pub fn data_dir() -> PathBuf {
    clock::data_dir()
}

pub fn reduced_motion() -> bool {
    super::reduced_motion()
}

/// The drop has no ring on macOS yet: nothing to redraw for a scan.
pub fn render_orb(_app: &mut App) {}

pub fn other_ring(_app: &App) -> bool {
    false
}

/// The run loop's one timer already follows `disk::animating`.
pub fn start_anim(_app: &mut App) {}

/// (resident MB, the same) from the Mach task info.
pub fn memory_mb() -> (f64, f64) {
    let mb = resident_bytes() as f64 / 1_048_576.0;
    (mb, mb)
}

/// The process's resident size, for the log.
pub fn resident_bytes() -> u64 {
    // SAFETY: getrusage fills a plain struct; ru_maxrss is in bytes on macOS (the peak).
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let peak = ru.ru_maxrss as u64;
    current_resident().unwrap_or(peak)
}

fn current_resident() -> Option<u64> {
    // proc_pid_rusage(getpid(), RUSAGE_INFO_V2): ri_resident_size is the current resident size.
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV2 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
        child_user_time: u64,
        child_system_time: u64,
        child_pkg_idle_wkups: u64,
        child_interrupt_wkups: u64,
        child_pageins: u64,
        child_elapsed_abstime: u64,
        diskio_bytesread: u64,
        diskio_byteswritten: u64,
    }
    extern "C" {
        fn proc_pid_rusage(
            pid: libc::c_int,
            flavor: libc::c_int,
            buffer: *mut RusageInfoV2,
        ) -> libc::c_int;
    }
    let mut info = RusageInfoV2::default();
    // SAFETY: flavor 2 is RUSAGE_INFO_V2, which fills exactly this struct.
    let r = unsafe { proc_pid_rusage(libc::getpid(), 2, &mut info) };
    (r == 0).then_some(info.phys_footprint.max(info.resident_size))
}

pub fn post(app: &mut App, n: Notice) {
    app.post(n);
}

pub fn notify(app: &mut App, p: Pill) {
    app.post(Notice::new(Source::Disk, Priority::Normal, p));
}

pub fn reply(app: &mut App, p: Pill) {
    app.reply(p);
}

/// New words for the pill on screen, in place (it does not grow from the drop again).
pub fn live(app: &mut App, p: Pill) {
    if app.pill.is_some() && !app.pill_closing {
        let queued = app.pill_queued;
        app.show_pill(p, queued);
    } else {
        app.reply(p);
    }
}

pub fn dismiss_pill(app: &mut App) {
    app.dismiss_pill(false);
}

pub fn quiet_holds(app: &App) -> bool {
    app.quiet().holds()
}

pub fn forget_cooldown(app: &mut App, kind: &str) {
    app.queue.cooldowns.remove(kind);
}

pub fn arm_check(app: &mut App, ms: u32) {
    app.disk_check_at = Some(Instant::now() + Duration::from_millis(ms as u64));
    app.disk_check_every = Duration::from_millis(ms as u64);
}

pub fn arm_release(app: &mut App, ms: u32) {
    app.disk_release_at = Some(Instant::now() + Duration::from_millis(ms as u64));
}

pub fn disarm_release(app: &mut App) {
    app.disk_release_at = None;
}

/// Wakes the main thread from a worker: `performSelectorOnMainThread:` on the menu target,
/// which lives as long as the app.
#[derive(Clone, Copy)]
pub struct Waker(usize);

// SAFETY: the pointer is only used for performSelectorOnMainThread, which may be sent from any
// thread, and the object it names is never released while the app runs.
unsafe impl Send for Waker {}

impl Waker {
    pub fn wake(&self) {
        let obj = self.0 as *const AnyObject;
        // SAFETY: see above; the selector is defined on the target's class.
        unsafe {
            let obj: &AnyObject = &*obj;
            let _: () = msg_send![obj, performSelectorOnMainThread: sel!(diskWake:), withObject: None::<&AnyObject>, waitUntilDone: false];
        }
    }
}

pub fn waker(app: &App) -> Waker {
    Waker(app.menu_target_ptr())
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The start screen's rows: your home folder (the default) and the startup disk.
pub fn fixed_drives() -> Vec<PathBuf> {
    vec![home(), PathBuf::from("/")]
}

/// "Macintosh HD" for the startup disk (its name in the Finder), "dhafin · home folder" for home.
pub fn volume_label(root: &Path) -> String {
    if root == home() {
        let user = root
            .file_name()
            .map_or_else(|| "Home".into(), |n| n.to_string_lossy().into_owned());
        return format!("{user} · home folder");
    }
    let fm = NSFileManager::defaultManager();
    let Some(s) = root.to_str() else {
        return crate::disk_model::drive_label(root);
    };
    let name = fm.displayNameAtPath(&NSString::from_str(s)).to_string();
    if name.is_empty() || name == "/" {
        crate::disk_model::drive_label(root)
    } else {
        name
    }
}

/// Scan again with nothing scanned yet: the home folder.
pub fn default_root() -> PathBuf {
    home()
}

/// The disk the dial's Disk child and the low-space pill read.
pub fn main_drive() -> PathBuf {
    PathBuf::from("/")
}

/// The low-space pill's Find big folders: your own folders, where the room usually went.
pub fn big_folders_root(_drive: char) -> PathBuf {
    home()
}

fn open_url(s: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(s)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

fn file_url(p: &Path) -> Option<Retained<NSURL>> {
    p.to_str()
        .map(|s| NSURL::fileURLWithPath(&NSString::from_str(s)))
}

/// Show in Finder: a Finder window with the item selected.
pub fn show_in_folder(p: &Path) {
    if let Some(url) = file_url(p) {
        let urls = NSArray::from_retained_slice(&[url]);
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
    }
}

/// The Trash in the Finder (opening it never empties it).
pub fn open_bin() {
    if let Some(url) = file_url(&home().join(".Trash")) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// System Settings, General, Storage.
pub fn open_storage_settings() {
    open_url("x-apple.systempreferences:com.apple.settings.Storage");
}

/// System Settings, Privacy & Security, Full Disk Access. Orby never grants anything itself.
pub fn open_privacy_settings() {
    open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles");
}

/// The folder picker runs after this click, with the app not borrowed (its modal loop runs
/// the app's events): `pickFolder:` on the menu target, next time round the run loop.
pub fn pick_folder(app: &mut App) {
    let obj = app.menu_target_ptr() as *const AnyObject;
    // SAFETY: the target lives as long as the app; the selector is defined on its class.
    unsafe {
        let obj: &AnyObject = &*obj;
        let _: () = msg_send![obj, performSelectorOnMainThread: sel!(pickFolder:), withObject: None::<&AnyObject>, waitUntilDone: false];
    }
}

pub fn toggle_max(app: &mut App) {
    window::toggle_zoom(app);
}

/// A local date and minute for a moment.
pub fn stamp(t: SystemTime) -> Stamp {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: localtime_r fills a plain struct from a plain number.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    Stamp {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        minute: (tm.tm_hour * 60 + tm.tm_min) as u32,
    }
}

/// Undo: each item this run moved to the Trash goes back where it was (never over something
/// new there).
pub fn restore(
    paths: &[PathBuf],
    _since: SystemTime,
) -> Vec<(PathBuf, Result<(), app_disk::PutBackError>)> {
    paths
        .iter()
        .map(|p| (p.clone(), app_disk::put_back(p)))
        .collect()
}
