//! Hybrid logical clock: wall milliseconds plus a counter, tie-broken by device id.

use serde::{Deserialize, Serialize};

use crate::model::DeviceId;

/// A timestamp. The derived order (wall, then counter, then device) is total, so two stamps
/// from different devices never compare equal.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    pub device: DeviceId,
}

/// One device's clock. Every stamp it hands out is greater than every stamp it has handed out
/// or observed before, even if the wall clock goes backwards.
#[derive(Clone, Debug)]
pub struct HlcClock {
    last: Hlc,
}

impl HlcClock {
    pub fn new(device: DeviceId) -> Self {
        Self::resume(device, 0, 0)
    }

    /// Continue from a saved (wall, counter), e.g. after reopening the database.
    pub fn resume(device: DeviceId, wall_ms: u64, counter: u32) -> Self {
        HlcClock { last: Hlc { wall_ms, counter, device } }
    }

    pub fn last(&self) -> &Hlc {
        &self.last
    }

    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        if now_ms > self.last.wall_ms {
            self.last.wall_ms = now_ms;
            self.last.counter = 0;
        } else {
            self.last.counter += 1;
        }
        self.last.clone()
    }

    /// Fold in a stamp seen from another device so the next tick sorts after it.
    pub fn observe(&mut self, remote: &Hlc) {
        if (remote.wall_ms, remote.counter) > (self.last.wall_ms, self.last.counter) {
            self.last.wall_ms = remote.wall_ms;
            self.last.counter = remote.counter;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(s: &str) -> DeviceId {
        DeviceId(s.into())
    }

    #[test]
    fn monotonic_when_wall_clock_goes_back() {
        let mut c = HlcClock::new(dev("pc"));
        let a = c.tick(1000);
        let b = c.tick(1000);
        let d = c.tick(900);
        let e = c.tick(2000);
        assert!(a < b && b < d && d < e);
        assert_eq!((d.wall_ms, d.counter), (1000, 2));
        assert_eq!((e.wall_ms, e.counter), (2000, 0));
    }

    #[test]
    fn tick_after_observe_sorts_after_remote() {
        let mut c = HlcClock::new(dev("pc"));
        c.tick(1000);
        let remote = Hlc { wall_ms: 5000, counter: 7, device: dev("phone") };
        c.observe(&remote);
        let next = c.tick(1001);
        assert!(next > remote);
        assert_eq!((next.wall_ms, next.counter), (5000, 8));
    }

    #[test]
    fn device_breaks_ties() {
        let a = Hlc { wall_ms: 1, counter: 0, device: dev("a") };
        let b = Hlc { wall_ms: 1, counter: 0, device: dev("b") };
        assert!(a < b);
        assert_ne!(a, b);
    }
}
