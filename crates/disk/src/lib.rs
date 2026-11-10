//! Arbor, the disk scanner. Scans a folder or drive into an arena [`Tree`] with a
//! multi-threaded walk, answers the panel queries (biggest children, largest files, type
//! breakdown, drive space), lists safe-to-clean candidates and cleaning-rule caches, and
//! moves items to the Recycle Bin. It never deletes anything by itself and has no permanent delete.

mod category;
mod clean;
mod dupes;
mod recycle;
mod rules;
mod scan;
mod space;
mod sys;
mod tree;

pub use category::{Category, TypeShare};
pub use clean::{safe_to_clean, Candidate, CleanPlaces, Suggestion, SuggestionKind, Tag};
pub use dupes::{
    candidates, check_duplicate_removal, drive_kind, find_duplicates, recycle_duplicates, Candidates, DriveKind, DupeFile, DupeGroup,
    DupeOptions, DupeProgress, DupeRemoval, DupeReport, DupeStage, DUPE_EDGE, DUPE_MIN_SIZE, DUPE_SKIPPED_NAMES,
};
pub use recycle::{has_recycle_bin, move_to_recycle_bin, recycle_bin_size, RecycleError, RecycleReport};
pub use rules::{
    check_removal, is_unfinished_download, measure_rules, never_touch, recycle_rule_items, Group, Measured, Refusal, Rule, RuleEnv,
    RuleFound, RuleMeasure, Tier, NEVER_NAMES, NEVER_REMOVE_ROOTS, NEVER_TOUCH, RULES, UNFINISHED_DOWNLOAD,
};
pub use scan::{scan, Progress, ScanError, ScanOptions};
pub use space::{drive_space, DriveSpace, LowSpaceWatch, LOW_SPACE_PERCENT, LOW_SPACE_STEP};
pub use tree::{ChildShare, Node, NodeId, NodeKind, ScanStatus, SkipReason, Skipped, Tree};
