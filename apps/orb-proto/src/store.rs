//! The drink log, focus sessions and focus setup on disk, as one small JSON file:
//! `habits.json` in the data folder (`data_dir`). Written whole after each change (write,
//! then rename).
//!
//! Times on disk are real Unix seconds (UTC), so the file means the same thing in any time zone
//! and can be synced later. The engines work in local wall-clock `Moment`s; [`unix`] and
//! [`local`] convert at this edge, with the zone's rule for that date (DST included).

use std::path::{Path, PathBuf};

use app_habits::drink::Source;
use app_habits::Moment;
use serde::{Deserialize, Serialize};

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

#[cfg(test)]
fn epoch_secs(m: Moment) -> i64 {
    let e = Moment::from_unix(0, 0);
    if m >= e {
        m.since(e).as_secs() as i64
    } else {
        -(e.since(m).as_secs() as i64)
    }
}

/// A local wall-clock moment as Unix seconds, by the zone's rule for that date (DST included).
pub fn unix(m: Moment) -> i64 {
    crate::sys::unix_of_local(m)
}

/// Unix seconds as a local wall-clock moment.
pub fn local(unix_secs: i64) -> Moment {
    crate::sys::local_of_unix(unix_secs)
}

/// The data folder (`sys::data_dir`): `%APPDATA%\orb-proto` on Windows, `~/Library/Application
/// Support/orb-proto` on macOS, created on first use. `ORB_PROTO_DATA` points it elsewhere, so a
/// test drive of the exe never writes into the real log.
pub fn data_dir() -> PathBuf {
    crate::sys::data_dir()
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
    use app_habits::Date;

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
        let now_local = crate::sys::now();
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
