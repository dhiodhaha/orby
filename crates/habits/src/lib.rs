//! Habit engines for the orb: the focus timer, the held-notices gate and drink pacing.
//!
//! Pure logic with no OS calls and no system clock. Every function that depends on time takes
//! `now` as a [`time::Moment`] (local wall-clock time), so the app reads the clock once at the top.

pub mod drink;
pub mod focus;
pub mod held;
pub mod time;

pub use time::{mins, Date, Moment};
