//! Drive capacity, scan percentage, and the low-space pill rule.

use crate::sys;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriveSpace {
    pub total: u64,
    /// Free bytes this user can write.
    pub free: u64,
}

impl DriveSpace {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    pub fn used_percent(&self) -> f64 {
        percent(self.used(), self.total)
    }

    pub fn free_percent(&self) -> f64 {
        percent(self.free, self.total)
    }

    /// Scan progress: bytes counted over bytes in use, 0 to 100. Honest without knowing
    /// the file count up front; capped because hard links can count bytes twice.
    pub fn scan_percent(&self, counted: u64) -> f64 {
        percent(counted, self.used()).min(100.0)
    }
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// Total and free space of the drive holding `path`.
pub fn drive_space(path: &Path) -> io::Result<DriveSpace> {
    let (total, free) = sys::space(path)?;
    Ok(DriveSpace { total, free })
}

/// Free space below this share of the drive raises the pill.
pub const LOW_SPACE_PERCENT: f64 = 10.0;
/// After a pill, another one needs free space to fall by this many more points.
pub const LOW_SPACE_STEP: f64 = 5.0;

/// The "C: has 38 GB left" rule. Feed it a reading on each check (the app checks every
/// 10 minutes); it answers whether to show the pill now. Hiding the pill while the orb is
/// hidden or in full screen is the app's call.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LowSpaceWatch {
    /// Free space is fine, or has not been read yet.
    #[default]
    Armed,
    /// A pill was shown when free space was at this percentage.
    Shown { at_free_percent: f64 },
}

impl LowSpaceWatch {
    pub fn check(&mut self, space: DriveSpace) -> bool {
        let free = space.free_percent();
        let (next, show) = match *self {
            _ if free >= LOW_SPACE_PERCENT => (LowSpaceWatch::Armed, false),
            LowSpaceWatch::Armed => (LowSpaceWatch::Shown { at_free_percent: free }, true),
            LowSpaceWatch::Shown { at_free_percent } if free <= at_free_percent - LOW_SPACE_STEP => {
                (LowSpaceWatch::Shown { at_free_percent: free }, true)
            }
            same @ LowSpaceWatch::Shown { .. } => (same, false),
        };
        *self = next;
        show
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    fn free(gb: u64) -> DriveSpace {
        DriveSpace { total: 476 * GB, free: gb * GB }
    }

    #[test]
    fn spec_capacity_example() {
        let s = DriveSpace { total: 476 * GB, free: 38 * GB };
        assert_eq!(s.used(), 438 * GB);
        assert_eq!(s.used_percent().round(), 92.0);
        assert_eq!(s.scan_percent(180 * GB).round(), 41.0);
        assert_eq!(s.scan_percent(500 * GB), 100.0);
        assert_eq!(DriveSpace { total: 0, free: 0 }.scan_percent(5), 0.0);
    }

    #[test]
    fn low_space_shows_once_then_after_five_more_points() {
        let mut w = LowSpaceWatch::default();
        assert!(!w.check(free(100)), "21% free is fine");
        assert!(w.check(free(38)), "8% free fires");
        assert!(!w.check(free(36)), "7.6%: not five points lower yet");
        assert!(!w.check(free(38)));
        assert!(w.check(free(14)), "2.9%: five points below 8% fires again");
        assert!(!w.check(free(14)));
    }

    #[test]
    fn freeing_space_rearms_the_pill() {
        let mut w = LowSpaceWatch::default();
        assert!(w.check(free(38)));
        assert!(!w.check(free(60)), "12.6% free resets");
        assert_eq!(w, LowSpaceWatch::Armed);
        assert!(w.check(free(40)));
    }
}
