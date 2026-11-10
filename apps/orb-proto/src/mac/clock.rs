//! The local clock and the settings folder on macOS.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_habits::{Date, Moment};

pub struct Local {
    pub y: i32,
    pub m: u32,
    pub d: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub ms: u32,
    /// Seconds east of UTC.
    pub offset: i64,
}

pub fn local() -> Local {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let t = now.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    Local {
        y: tm.tm_year + 1900,
        m: (tm.tm_mon + 1) as u32,
        d: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        min: tm.tm_min as u32,
        sec: tm.tm_sec as u32,
        ms: now.subsec_millis(),
        offset: tm.tm_gmtoff,
    }
}

/// Minutes since local midnight, with seconds.
pub fn now_minutes() -> f64 {
    let l = local();
    l.hour as f64 * 60.0 + l.min as f64 + l.sec as f64 / 60.0
}

pub fn today() -> (i32, u32, u32) {
    let l = local();
    (l.y, l.m, l.d)
}

pub fn now() -> Moment {
    let l = local();
    Date::new(l.y, l.m, l.d)
        .map(|d| d.at(l.hour, l.min) + Duration::from_secs(l.sec as u64))
        .unwrap_or(Moment::from_unix(0, 0))
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Milliseconds until the next whole wall-clock second, landing just after it begins.
pub fn ms_to_next_second() -> u64 {
    1000 - local().ms as u64 + 5
}

/// `~/Library/Application Support/orb-proto`, created on first use; `ORB_PROTO_DATA` points it
/// elsewhere (the same override as Windows).
pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("ORB_PROTO_DATA") {
        Some(d) => PathBuf::from(d),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Library/Application Support/orb-proto"),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}
