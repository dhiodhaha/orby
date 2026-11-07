//! Arbor, the disk scanner. Scans a folder or drive into an arena [`Tree`] with a
//! multi-threaded walk, answers the panel queries (biggest children, largest files, type
//! breakdown, drive space), lists safe-to-clean candidates, and moves items to the
//! Recycle Bin. It never deletes anything by itself and has no permanent delete.

mod category;
mod clean;
mod recycle;
mod scan;
mod space;
mod sys;
mod tree;

pub use category::{Category, TypeShare};
pub use clean::{safe_to_clean, Candidate, CleanPlaces, Suggestion, SuggestionKind, Tag};
pub use recycle::{has_recycle_bin, move_to_recycle_bin, recycle_bin_size, RecycleError, RecycleReport};
pub use scan::{scan, Progress, ScanError, ScanOptions};
pub use space::{drive_space, DriveSpace, LowSpaceWatch, LOW_SPACE_PERCENT, LOW_SPACE_STEP};
pub use tree::{ChildShare, Node, NodeId, NodeKind, ScanStatus, SkipReason, Skipped, Tree};
