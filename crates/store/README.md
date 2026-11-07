# app-store

The local-first task store (ORB_SPEC 6.2 Tasks, 9 Sync; DESIGN.md "Tasks panel and sharing (v6.2)").
Every device keeps a full SQLite copy of its spaces, folders, lists and tasks. Each write is split
into per-field changes stamped with a hybrid logical clock (HLC). The changes go into a change log
and an outbox for the sync client to push. Changes from other devices are merged with
last-writer-wins per field, so editing a title on the PC and ticking the task done on the phone
both survive. No UI, no network, no system clock: every write takes `now` (Unix ms), and the
agenda takes `today`.

## Storage
One row per field: `field(kind, entity_id, field, value, wall_ms, counter, device)`. A change can
arrive before the rest of its entity, so a field row has to make sense on its own. The merge is one
conditional upsert (`WHERE incoming_hlc > stored_hlc`). Typed entities are rebuilt on read, and an
entity whose required fields have not all arrived stays hidden until they do. Other tables:
`change_log` (every change seen, keyed by `change_id`, which also makes redelivery a no-op), `outbox`
(local changes not yet acked), `purged` (ids whose tombstones were purged, so late changes for them
are dropped), and `meta` (the saved clock).

## Public API
- `Store::open(path, DeviceId, UserId)`, `Store::open_in_memory(..)`.
- Writes (each takes `now: u64`): `create_space`, `edit_space(SpaceEdit)`, `set_member(space, user, Option<Role>)`,
  `create_folder(space, parent, name, order)`, `edit_folder(FolderEdit)`, `create_list(ListParent, name, order)`,
  `edit_list(ListEdit)`, `create_task(list, NewTask)`, `edit_task(TaskEdit)`, `set_done(task, bool)`,
  `delete(&EntityRef)`.
- Sync: `outbox() -> Vec<Change>`, `ack(&[ChangeId])`, `apply_remote(&[Change]) -> usize` (count of
  changes new to this store), `purge_tombstones(now) -> usize` (`TOMBSTONE_TTL_MS` = 30 days).
- Queries: `snapshot() -> Snapshot`, `spaces()`, `task(id)`, `folder_tree(space) -> Vec<TreeNode>`,
  `tasks_of_list(list) -> ListTasks {todo, done}`, `smart_view(folder) -> ListTasks`,
  `agenda(today: Date) -> Agenda {overdue, today, upcoming}`.
- Types: `Space {id, name, shared, members: Vec<Member {user, role: Owner|Editor|Viewer}>}`,
  `Folder {id, space_id, parent_id: Option, name, order}`, `List {id, parent: ListParent::Folder|Space, name, order}`,
  `Task {id, list_id, title, notes, due: Option<Due {date, time: Option}>, repeat: None|Daily|Weekly, assignee,
  done, created_by, completed_by, created_at, completed_at, order}`, `Date` (YYYY-MM-DD, valid only),
  `TimeOfDay` (HH:MM), `Hlc {wall_ms, counter, device}` with a total order, `HlcClock {tick, observe}`,
  `Change {change_id, space_id, entity_kind, entity_id, field, value: serde_json::Value, hlc, device}`,
  `StoreError {Sqlite, Json, NotFound, PermissionDenied, Invalid}`.

## Rules it enforces
- A delete is a `deleted` field, so a concurrent edit never undoes it; everything under a deleted
  space, folder or list is hidden with it.
- Local writes check the user's role: viewers cannot write, editors add, edit and finish,
  only the owner changes the space itself (name, shared, members, delete).
- A folder cannot move inside itself or its own subtree; tasks and lists cannot change space.
- Siblings sort by `order`, ties by id, so every device shows the same order.
- A tombstone is purged only when its whole subtree has nothing left in the outbox.

## Verified
```
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\store\target"; cd C:\Users\dopamine\dev\clippy\crates\store; cargo test; cargo clippy --all-targets -- -D warnings
```
21 tests pass (5 unit, 10 in `tests/crud.rs`, 6 in `tests/sync.rs`), and clippy is clean with
`-D warnings`. The suite was run 5 times in a row with the same result. The sync tests cover:
rename vs tick done, same-field LWW in both orders, delete vs edit in both orders, a fresh device
replaying the full history reversed and shuffled in two batches, duplicate and echoed delivery,
and late changes for a purged task. As a mutation check, making the merge ignore the HLC makes 2
sync tests fail.
