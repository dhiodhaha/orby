//! Local-first task store for spaces, folders, lists and tasks (ORB_SPEC 6.2 and 9).
//!
//! Every device keeps a full SQLite copy. Each write is split into per-field changes stamped
//! with a hybrid logical clock; they land in a change log and an outbox. `Store::apply_remote`
//! merges changes from other devices by last-writer-wins per field, so concurrent edits to
//! different fields of one task both survive, and delivery order and duplicates do not matter.
//!
//! Time is always passed in (`now` in Unix milliseconds, `today` as a `Date`); the store never
//! reads the system clock.

mod error;
mod hlc;
mod model;
mod store;

pub use error::{Result, StoreError};
pub use hlc::{Hlc, HlcClock};
pub use model::{
    Agenda, Change, ChangeId, Date, DeviceId, Due, EntityKind, EntityRef, Folder, FolderEdit, FolderId, List, ListEdit,
    ListId, ListParent, ListTasks, Member, NewTask, Repeat, Role, Snapshot, Space, SpaceEdit, SpaceId, Task, TaskEdit,
    TaskId, TimeOfDay, TreeNode, UserId,
};
pub use store::{Store, TOMBSTONE_TTL_MS};
