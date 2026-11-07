//! Downloads engine around a yt-dlp sidecar (ORB_SPEC §6, docs/design/downloads.md).
//!
//! Pure parts: link detection ([`MediaLink`]), format choice ([`Format`]), the yt-dlp argument
//! builder ([`build_args`]), the output parser ([`Tracker`], [`classify_error`]) and the queue
//! state machine ([`Queue`]). The only I/O lives in [`runner`], which spawns the process.

pub mod args;
pub mod format;
pub mod link;
pub mod parse;
pub mod queue;
pub mod runner;

pub use args::{build_args, Request};
pub use format::{AudioCodec, Format, FormatPrefs, MediaKind};
pub use link::{is_media_link, MediaLink, Site};
pub use parse::{classify_error, parse_line, ErrorKind, Line, Outcome, Progress, Tracker, Update};
pub use queue::{Cancelled, Counts, Job, JobId, JobState, Overall, Queue, QueueError, Ring, Tone, MAX_RUNNING};
pub use runner::{delete_partial, start, Download, Sidecar};
