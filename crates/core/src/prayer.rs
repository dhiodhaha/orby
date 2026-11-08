//! Prayer times from the sun's position, with Kemenag RI settings.
//!
//! Standard astronomical approximations (sun declination and the equation of time from the
//! low-precision solar formulas, accurate to about a minute for prayer times), written from the
//! formulas, not from any library. Times are local minutes after midnight.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Location {
    pub lat: f64,
    pub lng: f64,
    /// Hours from UTC (WIB = 7).
    pub tz: f64,
    pub elevation_m: f64,
}

impl Location {
    pub const YOGYAKARTA: Location = Location {
        lat: -7.7956,
        lng: 110.3695,
        tz: 7.0,
        elevation_m: 0.0,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Method {
    pub fajr_angle: f64,
    pub isha_angle: f64,
    /// Shadow factor for Ashar: 1 (Shafi'i, used in Indonesia) or 2 (Hanafi).
    pub asr_factor: f64,
    /// Minutes added to each computed time before rounding: [subuh, dzuhur, ashar, maghrib, isya].
    /// For Kemenag this is the 2 min ihtiyat plus the small difference between this sun model and the
    /// one behind the official Bimas Islam schedules. Dzuhur is 1 min more, as in their tables.
    pub offsets_min: [f64; 5],
    /// Minutes taken off Terbit (sunrise) before rounding down.
    pub terbit_ihtiyat_min: f64,
    /// Imsak is this many minutes before Subuh.
    pub imsak_before_min: f64,
}

impl Method {
    /// Calibrated against the Bimas Islam (Kemenag RI) schedule for Kota Yogyakarta on
    /// 1, 6, 15 and 30 Nov 2026: all 20 published times match to the minute (see tests).
    pub const KEMENAG: Method = Method {
        fajr_angle: 20.0,
        isha_angle: 18.0,
        asr_factor: 1.0,
        offsets_min: [2.3, 3.0, 2.0, 2.5, 1.9],
        terbit_ihtiyat_min: 2.0,
        imsak_before_min: 10.0,
    };

    /// Majelis Tarjih dan Tajdid PP Muhammadiyah: Subuh at the sun 18° below the horizon (Munas
    /// Tarjih XXXI, 2020, announced March 2021; Subuh comes about 8 minutes later than Kemenag's
    /// 20°), Isya 18°, Ashar shadow factor 1. Their tables round up with a smaller ihtiyat than
    /// Kemenag's. The offsets were fitted on the Imsakiyah 1447 H for Yogyakarta by Oman
    /// Fathurohman SW, Majelis Tarjih dan Tajdid (18 Feb to 19 Mar 2026): 178 of 180 published
    /// times match to the minute, the other two are Ashar ±1 (see tests). No published
    /// Muhammadiyah table for another season was found, so other months are not checked.
    pub const MUHAMMADIYAH: Method = Method {
        fajr_angle: 18.0,
        isha_angle: 18.0,
        asr_factor: 1.0,
        offsets_min: [0.95, 1.0, 1.0, 2.44, 1.0],
        terbit_ihtiyat_min: 2.45,
        imsak_before_min: 10.0,
    };
}

/// The most a manual adjustment moves one prayer, in minutes either way.
pub const MAX_ADJUST_MIN: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prayer {
    Subuh,
    Dzuhur,
    Ashar,
    Maghrib,
    Isya,
}

impl Prayer {
    pub const ALL: [Prayer; 5] = [
        Prayer::Subuh,
        Prayer::Dzuhur,
        Prayer::Ashar,
        Prayer::Maghrib,
        Prayer::Isya,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Prayer::Subuh => "Subuh",
            Prayer::Dzuhur => "Dzuhur",
            Prayer::Ashar => "Ashar",
            Prayer::Maghrib => "Maghrib",
            Prayer::Isya => "Isya",
        }
    }
}

/// Local times in whole minutes after midnight, rounded the way published schedules are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Times {
    pub imsak: u32,
    pub subuh: u32,
    pub terbit: u32,
    pub dzuhur: u32,
    pub ashar: u32,
    pub maghrib: u32,
    pub isya: u32,
}

impl Times {
    pub fn of(&self, p: Prayer) -> u32 {
        match p {
            Prayer::Subuh => self.subuh,
            Prayer::Dzuhur => self.dzuhur,
            Prayer::Ashar => self.ashar,
            Prayer::Maghrib => self.maghrib,
            Prayer::Isya => self.isya,
        }
    }

    /// The times moved by whole minutes per prayer ([subuh, dzuhur, ashar, maghrib, isya]), for
    /// someone who follows a local mosque's schedule. Each is clamped to ±`MAX_ADJUST_MIN`.
    /// Imsak moves with Subuh; Terbit stays (it is not a prayer time).
    pub fn adjusted(&self, adj: [i32; 5]) -> Times {
        let a = adj.map(|m| m.clamp(-MAX_ADJUST_MIN, MAX_ADJUST_MIN));
        let mv = |t: u32, m: i32| (t as i32 + m).max(0) as u32;
        Times {
            imsak: mv(self.imsak, a[0]),
            subuh: mv(self.subuh, a[0]),
            terbit: self.terbit,
            dzuhur: mv(self.dzuhur, a[1]),
            ashar: mv(self.ashar, a[2]),
            maghrib: mv(self.maghrib, a[3]),
            isya: mv(self.isya, a[4]),
        }
    }
}

pub fn hhmm(m: u32) -> String {
    format!("{:02}:{:02}", (m / 60) % 24, m % 60)
}

fn rad(d: f64) -> f64 {
    d * PI / 180.0
}
fn deg(r: f64) -> f64 {
    r * 180.0 / PI
}
fn fix(a: f64, b: f64) -> f64 {
    let r = a - b * (a / b).floor();
    if r < 0.0 {
        r + b
    } else {
        r
    }
}

/// Julian day at 0h UT of a Gregorian date.
pub fn julian(y: i32, m: u32, d: u32) -> f64 {
    let (mut y, mut m) = (y as f64, m as f64);
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + d as f64 + b - 1524.5
}

/// Sun declination (degrees) and equation of time (hours) at a Julian day.
fn sun(jd: f64) -> (f64, f64) {
    let d = jd - 2451545.0;
    let g = fix(357.529 + 0.98560028 * d, 360.0);
    let q = fix(280.459 + 0.98564736 * d, 360.0);
    let l = fix(q + 1.915 * rad(g).sin() + 0.020 * rad(2.0 * g).sin(), 360.0);
    let e = 23.439 - 0.00000036 * d;
    let ra = fix(
        deg((rad(e).cos() * rad(l).sin()).atan2(rad(l).cos())) / 15.0,
        24.0,
    );
    let decl = deg((rad(e).sin() * rad(l).sin()).asin());
    let mut eqt = q / 15.0 - ra;
    eqt -= 24.0 * (eqt / 24.0).round();
    (decl, eqt)
}

struct Calc {
    loc: Location,
    jd: f64,
}

impl Calc {
    fn noon(&self, t: f64) -> f64 {
        let (_, eqt) = sun(self.jd + t / 24.0);
        fix(12.0 - eqt, 24.0)
    }
    /// Time (hours, local solar) when the sun is `angle` degrees below the horizon.
    fn angle_time(&self, angle: f64, t: f64, before_noon: bool) -> f64 {
        let (decl, _) = sun(self.jd + t / 24.0);
        let noon = self.noon(t);
        let (lat, dc) = (rad(self.loc.lat), rad(decl));
        let cos_h = (-rad(angle).sin() - dc.sin() * lat.sin()) / (dc.cos() * lat.cos());
        let h = deg(cos_h.clamp(-1.0, 1.0).acos()) / 15.0;
        if before_noon {
            noon - h
        } else {
            noon + h
        }
    }
    fn asr_time(&self, factor: f64, t: f64) -> f64 {
        let (decl, _) = sun(self.jd + t / 24.0);
        let x = factor + (rad(self.loc.lat) - rad(decl)).abs().tan();
        let angle = -deg((1.0 / x).atan());
        self.angle_time(angle, t, false)
    }
}

/// Prayer times for one day, rounded like Kemenag's schedules: offsets added, then always rounded
/// up to the next minute (Terbit: ihtiyat taken off, rounded down).
pub fn times(loc: Location, m: Method, y: i32, mo: u32, d: u32) -> Times {
    let c = Calc {
        loc,
        jd: julian(y, mo, d) - loc.lng / (15.0 * 24.0),
    };
    let rise_angle = 0.833 + 0.0347 * loc.elevation_m.max(0.0).sqrt();
    let mut g = [5.0, 6.0, 12.0, 13.0, 18.0, 18.0];
    for _ in 0..2 {
        g = [
            c.angle_time(m.fajr_angle, g[0], true),
            c.angle_time(rise_angle, g[1], true),
            c.noon(g[2]),
            c.asr_time(m.asr_factor, g[3]),
            c.angle_time(rise_angle, g[4], false),
            c.angle_time(m.isha_angle, g[5], false),
        ];
    }
    let shift = loc.tz - loc.lng / 15.0;
    let mins = g.map(|v| (v + shift) * 60.0);
    let up = |x: f64, k: usize| (x + m.offsets_min[k]).floor() as u32 + 1;
    let subuh = up(mins[0], 0);
    Times {
        imsak: subuh - m.imsak_before_min as u32,
        subuh,
        terbit: (mins[1] - m.terbit_ihtiyat_min).floor() as u32,
        dzuhur: up(mins[2], 1),
        ashar: up(mins[3], 2),
        maghrib: up(mins[4], 3),
        isya: up(mins[5], 4),
    }
}

/// The prayer that comes next after `now_min` (minutes after local midnight) and the minutes until it.
/// After Isya the next one is tomorrow's Subuh, passed in as `tomorrow_subuh`.
pub fn next(t: &Times, now_min: f64, tomorrow_subuh: u32) -> (Prayer, u32, f64) {
    for p in Prayer::ALL {
        let at = t.of(p) as f64;
        if at > now_min {
            return (p, t.of(p), at - now_min);
        }
    }
    (
        Prayer::Subuh,
        tomorrow_subuh,
        tomorrow_subuh as f64 + 1440.0 - now_min,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bimas Islam (Kemenag RI) schedule for Kota Yogyakarta, Nov 2026, as published on liputan6.com.
    /// Columns: imsak, subuh, dzuhur, ashar, maghrib, isya.
    const OFFICIAL: [(u32, [&str; 6]); 4] = [
        (1, ["03:43", "03:53", "11:26", "14:39", "17:37", "18:48"]),
        (6, ["03:41", "03:51", "11:26", "14:42", "17:38", "18:50"]),
        (15, ["03:39", "03:49", "11:27", "14:47", "17:40", "18:54"]),
        (30, ["03:39", "03:49", "11:31", "14:55", "17:46", "19:01"]),
    ];

    #[test]
    fn matches_the_official_kemenag_schedule() {
        for (d, want) in OFFICIAL {
            let t = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, 11, d);
            let got = [t.imsak, t.subuh, t.dzuhur, t.ashar, t.maghrib, t.isya].map(hhmm);
            assert_eq!(got, want, "2026-11-{d:02}");
        }
    }

    #[test]
    fn order_is_always_right_through_the_year() {
        for mo in 1..=12 {
            for d in [1, 15, 28] {
                let t = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, mo, d);
                assert!(t.imsak < t.subuh && t.subuh < t.terbit && t.terbit < t.dzuhur);
                assert!(
                    t.dzuhur < t.ashar && t.ashar < t.maghrib && t.maghrib < t.isya,
                    "{mo}/{d}: {t:?}"
                );
            }
        }
    }

    #[test]
    fn next_prayer_rolls_over_after_isya() {
        let t = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, 11, 6);
        let (p, at, _) = next(&t, 16.0 * 60.0, 231);
        assert_eq!((p, hhmm(at).as_str()), (Prayer::Maghrib, "17:38"));
        let (p, _, mins) = next(&t, 20.0 * 60.0, 231);
        assert_eq!(p, Prayer::Subuh);
        assert!((mins - (231.0 + 240.0)).abs() < 1e-6);
    }
}

#[cfg(test)]
mod held_out {
    use super::*;

    /// Days the offsets were NOT tuned on (June and March 2026), same official source.
    /// Columns: subuh, dzuhur, ashar, maghrib, isya. Allowed: at most 1 minute off.
    const OFFICIAL: [((u32, u32), [&str; 5]); 5] = [
        ((6, 1), ["04:25", "11:40", "15:00", "17:30", "18:44"]),
        ((6, 15), ["04:27", "11:42", "15:02", "17:32", "18:47"]),
        ((6, 30), ["04:31", "11:46", "15:05", "17:36", "18:50"]),
        ((3, 1), ["04:27", "11:54", "14:55", "18:02", "19:11"]),
        ((3, 20), ["04:28", "11:50", "15:01", "17:53", "19:01"]),
    ];

    fn mins(s: &str) -> i32 {
        let (h, m) = s.split_once(':').unwrap();
        h.parse::<i32>().unwrap() * 60 + m.parse::<i32>().unwrap()
    }

    #[test]
    fn other_seasons_within_one_minute() {
        let mut exact = 0;
        let mut report = String::new();
        for ((mo, d), want) in OFFICIAL {
            let t = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, mo, d);
            let got = [t.subuh, t.dzuhur, t.ashar, t.maghrib, t.isya];
            for (g, w) in got.iter().zip(want) {
                let diff = *g as i32 - mins(w);
                if diff == 0 {
                    exact += 1;
                }
                report += &format!("{mo:02}-{d:02} got {} want {w} ({diff:+})\n", hhmm(*g));
                assert!(diff.abs() <= 1, "{report}");
            }
        }
        println!("{report}exact {exact} of 25");
    }
}

#[cfg(test)]
mod muhammadiyah {
    use super::*;

    /// "Jadwal Imsakiyah 1447 H, Yogyakarta (DIY)", dihisab oleh Oman Fathurohman SW, Majelis
    /// Tarjih dan Tajdid PP Muhammadiyah, published by Suara Muhammadiyah:
    /// https://web.suaramuhammadiyah.id/wp-content/uploads/2026/01/Imsakiyah-1447-H_yogyakarta.pdf
    /// Its place: 7° 47' 48" S, 110° 21' 50" E, WIB.
    const PLACE: Location = Location {
        lat: -(7.0 + 47.0 / 60.0 + 48.0 / 3600.0),
        lng: 110.0 + 21.0 / 60.0 + 50.0 / 3600.0,
        tz: 7.0,
        elevation_m: 0.0,
    };

    /// Columns: imsak, subuh, terbit, dzuhur, ashar, maghrib, isya (2026).
    const PUBLISHED: [((u32, u32), [&str; 7]); 30] = [
        (
            (2, 18),
            [
                "04:23", "04:33", "05:40", "11:54", "15:04", "18:05", "19:15",
            ],
        ),
        (
            (2, 19),
            [
                "04:23", "04:33", "05:40", "11:54", "15:03", "18:05", "19:15",
            ],
        ),
        (
            (2, 20),
            [
                "04:23", "04:33", "05:40", "11:54", "15:02", "18:05", "19:14",
            ],
        ),
        (
            (2, 21),
            [
                "04:23", "04:33", "05:40", "11:54", "15:01", "18:04", "19:14",
            ],
        ),
        (
            (2, 22),
            [
                "04:24", "04:34", "05:40", "11:54", "15:00", "18:04", "19:13",
            ],
        ),
        (
            (2, 23),
            [
                "04:24", "04:34", "05:40", "11:53", "15:00", "18:04", "19:13",
            ],
        ),
        (
            (2, 24),
            [
                "04:24", "04:34", "05:40", "11:53", "14:59", "18:03", "19:12",
            ],
        ),
        (
            (2, 25),
            [
                "04:24", "04:34", "05:40", "11:53", "14:58", "18:03", "19:12",
            ],
        ),
        (
            (2, 26),
            [
                "04:24", "04:34", "05:40", "11:53", "14:57", "18:03", "19:11",
            ],
        ),
        (
            (2, 27),
            [
                "04:24", "04:34", "05:40", "11:53", "14:56", "18:02", "19:11",
            ],
        ),
        (
            (2, 28),
            [
                "04:24", "04:34", "05:40", "11:53", "14:55", "18:02", "19:11",
            ],
        ),
        (
            (3, 1),
            [
                "04:25", "04:35", "05:40", "11:52", "14:55", "18:01", "19:10",
            ],
        ),
        (
            (3, 2),
            [
                "04:25", "04:35", "05:40", "11:52", "14:55", "18:01", "19:10",
            ],
        ),
        (
            (3, 3),
            [
                "04:25", "04:35", "05:40", "11:52", "14:55", "18:00", "19:09",
            ],
        ),
        (
            (3, 4),
            [
                "04:25", "04:35", "05:40", "11:52", "14:56", "18:00", "19:09",
            ],
        ),
        (
            (3, 5),
            [
                "04:25", "04:35", "05:40", "11:52", "14:56", "18:00", "19:08",
            ],
        ),
        (
            (3, 6),
            [
                "04:25", "04:35", "05:40", "11:51", "14:57", "17:59", "19:08",
            ],
        ),
        (
            (3, 7),
            [
                "04:25", "04:35", "05:40", "11:51", "14:57", "17:59", "19:07",
            ],
        ),
        (
            (3, 8),
            [
                "04:25", "04:35", "05:40", "11:51", "14:57", "17:58", "19:07",
            ],
        ),
        (
            (3, 9),
            [
                "04:25", "04:35", "05:40", "11:51", "14:58", "17:58", "19:06",
            ],
        ),
        (
            (3, 10),
            [
                "04:25", "04:35", "05:40", "11:50", "14:58", "17:57", "19:05",
            ],
        ),
        (
            (3, 11),
            [
                "04:25", "04:35", "05:40", "11:50", "14:58", "17:57", "19:05",
            ],
        ),
        (
            (3, 12),
            [
                "04:25", "04:35", "05:40", "11:50", "14:59", "17:56", "19:04",
            ],
        ),
        (
            (3, 13),
            [
                "04:25", "04:35", "05:40", "11:50", "14:59", "17:56", "19:04",
            ],
        ),
        (
            (3, 14),
            [
                "04:25", "04:35", "05:40", "11:49", "14:59", "17:55", "19:03",
            ],
        ),
        (
            (3, 15),
            [
                "04:25", "04:35", "05:40", "11:49", "14:59", "17:55", "19:03",
            ],
        ),
        (
            (3, 16),
            [
                "04:25", "04:35", "05:40", "11:49", "15:00", "17:54", "19:02",
            ],
        ),
        (
            (3, 17),
            [
                "04:25", "04:35", "05:40", "11:48", "15:00", "17:54", "19:02",
            ],
        ),
        (
            (3, 18),
            [
                "04:25", "04:35", "05:40", "11:48", "15:00", "17:53", "19:01",
            ],
        ),
        (
            (3, 19),
            [
                "04:25", "04:35", "05:40", "11:48", "15:00", "17:53", "19:01",
            ],
        ),
    ];

    fn mins(s: &str) -> i32 {
        let (h, m) = s.split_once(':').unwrap();
        h.parse::<i32>().unwrap() * 60 + m.parse::<i32>().unwrap()
    }

    #[test]
    fn matches_the_published_muhammadiyah_imsakiyah_for_yogyakarta() {
        let mut exact = 0;
        let mut report = String::new();
        for ((mo, d), want) in PUBLISHED {
            let t = times(PLACE, Method::MUHAMMADIYAH, 2026, mo, d);
            let got = [
                t.imsak, t.subuh, t.terbit, t.dzuhur, t.ashar, t.maghrib, t.isya,
            ];
            for (g, w) in got.iter().zip(want) {
                let diff = *g as i32 - mins(w);
                exact += (diff == 0) as u32;
                report += &format!("{mo:02}-{d:02} got {} want {w} ({diff:+})\n", hhmm(*g));
                assert!(diff.abs() <= 1, "{report}");
            }
        }
        assert!(exact >= 208, "{report}exact {exact} of 210");
    }

    #[test]
    fn subuh_18_degrees_is_about_8_minutes_after_kemenag() {
        // Muhammadiyah's own figure for the change from 20° to 18°: about 8 minutes.
        for (mo, d) in [(3, 1), (6, 15), (11, 6)] {
            let k = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, mo, d);
            let m = times(Location::YOGYAKARTA, Method::MUHAMMADIYAH, 2026, mo, d);
            let diff = m.subuh as i32 - k.subuh as i32;
            assert!((7..=9).contains(&diff), "{mo}/{d}: {diff}");
        }
    }

    #[test]
    fn manual_adjustments_move_each_prayer_and_are_clamped() {
        let t = times(Location::YOGYAKARTA, Method::KEMENAG, 2026, 11, 6);
        let a = t.adjusted([2, 0, -1, 0, 25]);
        assert_eq!((a.subuh, a.imsak), (t.subuh + 2, t.imsak + 2));
        assert_eq!(a.terbit, t.terbit);
        assert_eq!(a.dzuhur, t.dzuhur);
        assert_eq!(a.ashar, t.ashar - 1);
        assert_eq!(a.isya, t.isya + MAX_ADJUST_MIN as u32);
        assert_eq!(t.adjusted([0; 5]), t);
    }
}
