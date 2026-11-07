//! Notices held during focus (ORB_SPEC 6.4): normal ones wait for the break, urgent ones pass.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Urgent,
    Normal,
    Quiet,
}

/// What happened to an offered notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate<T> {
    /// Show it now.
    Show(T),
    /// Queued until [`HeldNotices::release`].
    Held,
    /// A quiet notice during focus is dropped, not queued: a stale offer is noise (docs/design/downloads.md).
    Dropped,
}

/// A FIFO of notices waiting for the break. `count` feeds "2 notices wait for your break".
#[derive(Clone, Debug)]
pub struct HeldNotices<T> {
    items: VecDeque<T>,
}

impl<T> Default for HeldNotices<T> {
    fn default() -> Self {
        HeldNotices { items: VecDeque::new() }
    }
}

impl<T> HeldNotices<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `holding` is [`crate::focus::FocusTimer::is_holding`] at the time of the offer.
    pub fn offer(&mut self, notice: T, priority: Priority, holding: bool) -> Gate<T> {
        match (holding, priority) {
            (false, _) | (true, Priority::Urgent) => Gate::Show(notice),
            (true, Priority::Normal) => {
                self.items.push_back(notice);
                Gate::Held
            }
            (true, Priority::Quiet) => Gate::Dropped,
        }
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Drops the held notices that went stale while waiting, such as "10 menit lagi Subuh" once
    /// Subuh has come.
    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.items.retain(keep);
    }

    /// Everything held, oldest first. Call it when the timer leaves focus (break or stop).
    pub fn release(&mut self) -> Vec<T> {
        self.items.drain(..).collect()
    }
}
