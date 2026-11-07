//! The drink log, focus sessions and focus setup on disk, as one small JSON file:
//! `%APPDATA%\orb-proto\habits.json`. Written whole after each change (write, then rename).
//!
//! Times on disk are real Unix seconds (UTC), so the file means the same thing in any time zone
//! and can be synced later. The engines work in local wall-clock `Moment`s; [`unix`] and
//! [`local`] convert at this edge, with the zone's rule for that date (DST included).

use std::path::{Path, PathBuf};

use app_habits::drink::Source;
use app_habits::{Date, Moment};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::Time::{
    SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime,
};

/// 1: times are Unix seconds. 0 (files written before the field existed): local wall-clock seconds.
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Saved {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub drinks: Vec<SavedDrink>,
    #[serde(default)]
    pub sessions: Vec<SavedSession>,
    #[serde(default)]
    pub focus: FocusPrefs,
}

/// `at` is Unix seconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedDrink {
    pub ml: u32,
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

/// `ended` is Unix seconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedSession {
    pub ended: i64,
    pub minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FocusPrefs {
    pub length_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub auto_start: bool,
}

impl Default for FocusPrefs {
    fn default() -> Self {
        FocusPrefs {
            length_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            auto_start: false,
        }
    }
}

impl SavedDrink {
    pub fn source(&self) -> Source {
        match &self.device {
            Some(d) => Source::Device(d.clone()),
            None => Source::Local,
        }
    }
}

fn epoch_secs(m: Moment) -> i64 {
    let e = Moment::from_unix(0, 0);
    if m >= e {
        m.since(e).as_secs() as i64
    } else {
        -(e.since(m).as_secs() as i64)
    }
}

fn to_systemtime(m: Moment) -> SYSTEMTIME {
    let (d, s) = (m.date(), epoch_secs(m).rem_euclid(60));
    let t = m.minute_of_day();
    SYSTEMTIME {
        wYear: d.year() as u16,
        wMonth: d.month() as u16,
        wDayOfWeek: 0,
        wDay: d.day() as u16,
        wHour: (t / 60) as u16,
        wMinute: (t % 60) as u16,
        wSecond: s as u16,
        wMilliseconds: 0,
    }
}

fn from_systemtime(st: &SYSTEMTIME) -> Option<Moment> {
    let d = Date::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)?;
    Some(
        d.at(st.wHour as u32, st.wMinute as u32)
            + std::time::Duration::from_secs(st.wSecond as u64),
    )
}

/// A local wall-clock moment as Unix seconds, by the zone's rule for that date.
pub fn unix(m: Moment) -> i64 {
    let mut utc = SYSTEMTIME::default();
    match unsafe { TzSpecificLocalTimeToSystemTime(None, &to_systemtime(m), &mut utc) }
        .ok()
        .and_then(|_| from_systemtime(&utc))
    {
        Some(u) => epoch_secs(u),
        None => epoch_secs(m),
    }
}

/// Unix seconds as a local wall-clock moment.
pub fn local(unix_secs: i64) -> Moment {
    let u = Moment::from_unix(unix_secs, 0);
    let mut lt = SYSTEMTIME::default();
    unsafe { SystemTimeToTzSpecificLocalTime(None, &to_systemtime(u), &mut lt) }
        .ok()
        .and_then(|_| from_systemtime(&lt))
        .unwrap_or(u)
}

/// `%APPDATA%\orb-proto`, created on first use. `ORB_PROTO_DATA` points it elsewhere, so a test
/// drive of the exe never writes into the real log.
pub fn data_dir() -> PathBuf {
    let dir = match std::env::var_os("ORB_PROTO_DATA") {
        Some(d) => PathBuf::from(d),
        None => std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(crate::exe_dir)
            .join("orb-proto"),
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A missing or unreadable file starts empty; a corrupt one is kept aside, never overwritten.
/// An older file's local times come back as Unix seconds.
pub fn load(path: &Path) -> Saved {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Saved {
            format: FORMAT,
            ..Saved::default()
        };
    };
    match serde_json::from_str::<Saved>(&text) {
        Ok(mut s) => {
            if s.format == 0 {
                let from_local = |secs: i64| unix(Moment::from_unix(secs, 0));
                s.drinks.iter_mut().for_each(|d| d.at = from_local(d.at));
                s.sessions
                    .iter_mut()
                    .for_each(|x| x.ended = from_local(x.ended));
            }
            s.format = FORMAT;
            s
        }
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
            Saved {
                format: FORMAT,
                ..Saved::default()
            }
        }
    }
}

pub fn save(path: &Path, s: &Saved) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(&Saved {
            format: FORMAT,
            ..s.clone()
        })?,
    )?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orb-proto-store-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("habits.json")
    }

    #[test]
    fn saved_habits_round_trip_through_the_file() {
        let path = temp_file("roundtrip");
        let at = Date::new(2026, 11, 6).unwrap().at(10, 42);
        let s = Saved {
            format: FORMAT,
            drinks: vec![
                SavedDrink {
                    ml: 250,
                    at: unix(at),
                    device: None,
                },
                SavedDrink {
                    ml: 150,
                    at: unix(at) + 60,
                    device: Some("phone".into()),
                },
            ],
            sessions: vec![SavedSession {
                ended: unix(at),
                minutes: 25,
                task: None,
            }],
            focus: FocusPrefs {
                length_min: 50,
                short_break_min: 10,
                long_break_min: 20,
                auto_start: true,
            },
        };
        save(&path, &s).unwrap();
        let back = load(&path);
        assert_eq!(back, s);
        assert_eq!(local(back.drinks[0].at), at);
        assert_eq!(back.drinks[1].source(), Source::Device("phone".into()));
    }

    #[test]
    fn times_on_disk_are_real_unix_seconds() {
        let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let now_local = from_systemtime(&st).unwrap();
        let sys = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(
            (unix(now_local) - sys).abs() <= 2,
            "local now {now_local:?} gave {} for Unix {sys}",
            unix(now_local)
        );
        assert_eq!(local(unix(now_local)), now_local);
    }

    #[test]
    fn an_older_file_with_local_times_is_read_as_unix() {
        let path = temp_file("format0");
        let at = Date::new(2026, 11, 6).unwrap().at(22, 39);
        let local_secs = epoch_secs(at);
        std::fs::write(&path, format!(r#"{{"drinks":[{{"ml":250,"at":{local_secs}}}],"sessions":[{{"ended":{local_secs},"minutes":1}}]}}"#)).unwrap();
        let back = load(&path);
        assert_eq!(back.format, FORMAT);
        assert_eq!(back.drinks[0].at, unix(at));
        assert_eq!(local(back.drinks[0].at), at);
        assert_eq!(local(back.sessions[0].ended), at);
    }

    #[test]
    fn a_missing_file_starts_empty_and_a_corrupt_one_is_kept_aside() {
        let path = temp_file("corrupt");
        let _ = std::fs::remove_file(&path);
        let empty = Saved {
            format: FORMAT,
            ..Saved::default()
        };
        assert_eq!(load(&path), empty);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), empty);
        assert!(path.with_extension("json.corrupt").exists());
    }
}
