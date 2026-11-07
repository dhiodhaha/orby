//! The SQLite store: per-field rows with their winning HLC, a change log, an outbox.
//!
//! Entities are kept as one row per field rather than one row per entity. A change can arrive
//! before the rest of its entity (any delivery order), and a field row needs no other column to
//! be meaningful, so merging is a single conditional upsert. Typed entities are rebuilt on read;
//! one whose required fields have not all arrived is simply not visible yet.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{Result, StoreError};
use crate::hlc::{Hlc, HlcClock};
use crate::model::*;

/// Tombstones older than this are purged.
pub const TOMBSTONE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS field (
    kind TEXT NOT NULL, entity_id TEXT NOT NULL, field TEXT NOT NULL, value TEXT NOT NULL,
    wall_ms INTEGER NOT NULL, counter INTEGER NOT NULL, device TEXT NOT NULL,
    PRIMARY KEY (kind, entity_id, field)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS change_log (
    change_id TEXT PRIMARY KEY, kind TEXT NOT NULL, entity_id TEXT NOT NULL, body TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS change_log_entity ON change_log (entity_id);
CREATE TABLE IF NOT EXISTS outbox (seq INTEGER PRIMARY KEY AUTOINCREMENT, change_id TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS purged (kind TEXT NOT NULL, entity_id TEXT NOT NULL, PRIMARY KEY (kind, entity_id)) WITHOUT ROWID;
";

const DELETED: &str = "deleted";
const MEMBER_PREFIX: &str = "member:";

type Fields = Map<String, Value>;
type FieldList = Vec<(String, Value)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Edit,
    Own,
}

pub struct Store {
    conn: Connection,
    user: UserId,
    clock: HlcClock,
}

impl Store {
    /// Open or create a database file. `device` must be stable for this install; `user` is who
    /// is signed in, and becomes `created_by` / `completed_by` on writes.
    pub fn open(path: impl AsRef<Path>, device: DeviceId, user: UserId) -> Result<Store> {
        Self::init(Connection::open(path)?, device, user)
    }

    pub fn open_in_memory(device: DeviceId, user: UserId) -> Result<Store> {
        Self::init(Connection::open_in_memory()?, device, user)
    }

    fn init(conn: Connection, device: DeviceId, user: UserId) -> Result<Store> {
        conn.execute_batch(SCHEMA)?;
        let saved: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'clock'", [], |r| r.get(0)).optional()?;
        let clock = match saved {
            Some(s) => {
                let (wall, counter): (u64, u32) = serde_json::from_str(&s)?;
                HlcClock::resume(device, wall, counter)
            }
            None => HlcClock::new(device),
        };
        Ok(Store { conn, user, clock })
    }

    pub fn user(&self) -> &UserId {
        &self.user
    }

    pub fn device(&self) -> &DeviceId {
        &self.clock.last().device
    }

    // ---- writes ----

    /// A new private space with the local user as its owner.
    pub fn create_space(&mut self, name: &str, now: u64) -> Result<SpaceId> {
        let id = SpaceId(new_id(now));
        let space = Space { id: id.clone(), name: name.into(), shared: false, members: vec![] };
        let mut fields = entity_fields(&space)?;
        fields.push((member_field(&self.user), serde_json::to_value(Role::Owner)?));
        self.write(&id, EntityKind::Space, id.as_str(), fields, now)?;
        Ok(id)
    }

    pub fn edit_space(&mut self, id: &SpaceId, edit: SpaceEdit, now: u64) -> Result<()> {
        self.require(id, Access::Own)?;
        let field = match edit {
            SpaceEdit::Name(n) => ("name".to_owned(), Value::from(n)),
            SpaceEdit::Shared(s) => ("shared".to_owned(), Value::from(s)),
        };
        self.write(id, EntityKind::Space, id.as_str(), vec![field], now)
    }

    /// Add, change or (with `None`) remove a member. Owner only.
    pub fn set_member(&mut self, space: &SpaceId, user: &UserId, role: Option<Role>, now: u64) -> Result<()> {
        self.require(space, Access::Own)?;
        let value = serde_json::to_value(role)?;
        self.write(space, EntityKind::Space, space.as_str(), vec![(member_field(user), value)], now)
    }

    pub fn create_folder(
        &mut self,
        space: &SpaceId,
        parent: Option<&FolderId>,
        name: &str,
        order: f64,
        now: u64,
    ) -> Result<FolderId> {
        self.require(space, Access::Edit)?;
        if let Some(p) = parent {
            self.check_folder_parent(space, p, None)?;
        }
        let id = FolderId(new_id(now));
        let folder =
            Folder { id: id.clone(), space_id: space.clone(), parent_id: parent.cloned(), name: name.into(), order };
        self.write(space, EntityKind::Folder, id.as_str(), entity_fields(&folder)?, now)?;
        Ok(id)
    }

    pub fn edit_folder(&mut self, id: &FolderId, edit: FolderEdit, now: u64) -> Result<()> {
        let folder: Folder = self.record(EntityKind::Folder, id.as_str())?;
        self.require(&folder.space_id, Access::Edit)?;
        let field = match edit {
            FolderEdit::Name(n) => ("name", Value::from(n)),
            FolderEdit::Order(o) => ("order", Value::from(o)),
            FolderEdit::Parent(p) => {
                if let Some(p) = &p {
                    self.check_folder_parent(&folder.space_id, p, Some(id))?;
                }
                ("parent_id", serde_json::to_value(p)?)
            }
        };
        self.write(&folder.space_id, EntityKind::Folder, id.as_str(), vec![(field.0.into(), field.1)], now)
    }

    pub fn create_list(&mut self, parent: ListParent, name: &str, order: f64, now: u64) -> Result<ListId> {
        let space = self.space_of_parent(&parent)?;
        self.require(&space, Access::Edit)?;
        let id = ListId(new_id(now));
        let list = List { id: id.clone(), parent, name: name.into(), order };
        self.write(&space, EntityKind::List, id.as_str(), entity_fields(&list)?, now)?;
        Ok(id)
    }

    pub fn edit_list(&mut self, id: &ListId, edit: ListEdit, now: u64) -> Result<()> {
        let list: List = self.record(EntityKind::List, id.as_str())?;
        let space = self.space_of_parent(&list.parent)?;
        self.require(&space, Access::Edit)?;
        let field = match edit {
            ListEdit::Name(n) => ("name", Value::from(n)),
            ListEdit::Order(o) => ("order", Value::from(o)),
            ListEdit::Parent(p) => {
                if self.space_of_parent(&p)? != space {
                    return Err(StoreError::Invalid("a list cannot move to another space"));
                }
                ("parent", serde_json::to_value(p)?)
            }
        };
        self.write(&space, EntityKind::List, id.as_str(), vec![(field.0.into(), field.1)], now)
    }

    pub fn create_task(&mut self, list: &ListId, new: NewTask, now: u64) -> Result<TaskId> {
        let space = self.space_of_list(list)?;
        self.require(&space, Access::Edit)?;
        let id = TaskId(new_id(now));
        let task = Task {
            id: id.clone(),
            list_id: list.clone(),
            title: new.title,
            notes: new.notes,
            due: new.due,
            repeat: new.repeat,
            assignee: new.assignee,
            done: false,
            created_by: self.user.clone(),
            completed_by: None,
            created_at: now,
            completed_at: None,
            order: new.order,
        };
        self.write(&space, EntityKind::Task, id.as_str(), entity_fields(&task)?, now)?;
        Ok(id)
    }

    pub fn edit_task(&mut self, id: &TaskId, edit: TaskEdit, now: u64) -> Result<()> {
        let task: Task = self.record(EntityKind::Task, id.as_str())?;
        let space = self.space_of_list(&task.list_id)?;
        self.require(&space, Access::Edit)?;
        let field = match edit {
            TaskEdit::Title(t) => ("title", Value::from(t)),
            TaskEdit::Notes(n) => ("notes", Value::from(n)),
            TaskEdit::Due(d) => ("due", serde_json::to_value(d)?),
            TaskEdit::Repeat(r) => ("repeat", serde_json::to_value(r)?),
            TaskEdit::Assignee(a) => ("assignee", serde_json::to_value(a)?),
            TaskEdit::Order(o) => ("order", Value::from(o)),
            TaskEdit::List(l) => {
                if self.space_of_list(&l)? != space {
                    return Err(StoreError::Invalid("a task cannot move to another space"));
                }
                ("list_id", serde_json::to_value(l)?)
            }
        };
        self.write(&space, EntityKind::Task, id.as_str(), vec![(field.0.into(), field.1)], now)
    }

    /// Tick or untick. Records who completed it and when.
    pub fn set_done(&mut self, id: &TaskId, done: bool, now: u64) -> Result<()> {
        let task: Task = self.record(EntityKind::Task, id.as_str())?;
        let space = self.space_of_list(&task.list_id)?;
        self.require(&space, Access::Edit)?;
        let (by, at) = if done { (Some(self.user.clone()), Some(now)) } else { (None, None) };
        let fields = vec![
            ("done".to_owned(), Value::from(done)),
            ("completed_by".to_owned(), serde_json::to_value(by)?),
            ("completed_at".to_owned(), serde_json::to_value(at)?),
        ];
        self.write(&space, EntityKind::Task, id.as_str(), fields, now)
    }

    /// Leave a tombstone. Everything under a deleted space, folder or list disappears with it.
    pub fn delete(&mut self, target: &EntityRef, now: u64) -> Result<()> {
        let (space, access) = match target {
            EntityRef::Space(id) => {
                self.record::<Space>(EntityKind::Space, id.as_str())?;
                (id.clone(), Access::Own)
            }
            EntityRef::Folder(id) => (self.record::<Folder>(EntityKind::Folder, id.as_str())?.space_id, Access::Edit),
            EntityRef::List(id) => {
                let list: List = self.record(EntityKind::List, id.as_str())?;
                (self.space_of_parent(&list.parent)?, Access::Edit)
            }
            EntityRef::Task(id) => {
                let task: Task = self.record(EntityKind::Task, id.as_str())?;
                (self.space_of_list(&task.list_id)?, Access::Edit)
            }
        };
        self.require(&space, access)?;
        self.write(&space, target.kind(), target.id(), vec![(DELETED.into(), Value::Bool(true))], now)
    }

    fn write(&mut self, space: &SpaceId, kind: EntityKind, id: &str, fields: FieldList, now: u64) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (field, value) in fields {
            let hlc = self.clock.tick(now);
            let change = Change {
                change_id: ChangeId(new_id(now)),
                space_id: space.clone(),
                entity_kind: kind,
                entity_id: id.to_owned(),
                field,
                value,
                device: hlc.device.clone(),
                hlc,
            };
            log_change(&tx, &change)?;
            tx.execute("INSERT INTO outbox (change_id) VALUES (?1)", [change.change_id.as_str()])?;
            merge_field(&tx, &change)?;
        }
        save_clock(&tx, &self.clock)?;
        tx.commit()?;
        Ok(())
    }

    // ---- sync ----

    /// Merge changes from other devices. Order does not matter and duplicates are skipped.
    /// Returns how many changes were new to this store.
    pub fn apply_remote(&mut self, changes: &[Change]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut fresh = 0;
        for c in changes {
            let purged = tx
                .query_row(
                    "SELECT 1 FROM purged WHERE kind = ?1 AND entity_id = ?2",
                    params![c.entity_kind.as_str(), c.entity_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if purged || !log_change(&tx, c)? {
                continue;
            }
            self.clock.observe(&c.hlc);
            merge_field(&tx, c)?;
            fresh += 1;
        }
        save_clock(&tx, &self.clock)?;
        tx.commit()?;
        Ok(fresh)
    }

    /// Local changes not yet acknowledged by the server, oldest first.
    pub fn outbox(&self) -> Result<Vec<Change>> {
        let mut stmt =
            self.conn.prepare("SELECT c.body FROM outbox o JOIN change_log c USING (change_id) ORDER BY o.seq")?;
        let bodies = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        bodies.iter().map(|b| serde_json::from_str(b).map_err(StoreError::from)).collect()
    }

    /// The server has stored these; drop them from the outbox.
    pub fn ack(&mut self, ids: &[ChangeId]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM outbox WHERE change_id = ?1", [id.as_str()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove entities deleted at least `TOMBSTONE_TTL_MS` before `now`, with everything under
    /// them. A tombstone whose subtree still has unsent changes waits until they are acked.
    /// Later changes to a purged entity are ignored. Returns how many entities were removed.
    pub fn purge_tombstones(&mut self, now: u64) -> Result<usize> {
        let cutoff = now.saturating_sub(TOMBSTONE_TTL_MS) as i64;
        let roots: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT kind, entity_id FROM field WHERE field = 'deleted' AND value = 'true' AND wall_ms <= ?1",
            )?;
            let rows = stmt.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if roots.is_empty() {
            return Ok(0);
        }
        let pending: HashSet<String> = {
            let mut stmt = self.conn.prepare("SELECT c.entity_id FROM outbox o JOIN change_log c USING (change_id)")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let all = self.load_all()?;
        let mut children: HashMap<&str, Vec<(EntityKind, &str)>> = HashMap::new();
        for ((kind, id), fields) in &all {
            if let Some(parent) = parent_key(*kind, fields) {
                children.entry(parent).or_default().push((*kind, id.as_str()));
            }
        }
        let mut doomed: BTreeSet<(EntityKind, String)> = BTreeSet::new();
        for (kind, id) in &roots {
            let Some(kind) = EntityKind::parse(kind) else { continue };
            let mut subtree = vec![(kind, id.as_str())];
            let mut i = 0;
            while i < subtree.len() {
                if let Some(kids) = children.get(subtree[i].1) {
                    subtree.extend(kids.iter().copied());
                }
                i += 1;
            }
            if subtree.iter().any(|(_, id)| pending.contains(*id)) {
                continue;
            }
            doomed.extend(subtree.into_iter().map(|(k, id)| (k, id.to_owned())));
        }
        let tx = self.conn.transaction()?;
        for (kind, id) in &doomed {
            tx.execute("DELETE FROM field WHERE kind = ?1 AND entity_id = ?2", params![kind.as_str(), id])?;
            tx.execute("DELETE FROM change_log WHERE entity_id = ?1", [id])?;
            tx.execute("INSERT OR IGNORE INTO purged (kind, entity_id) VALUES (?1, ?2)", params![kind.as_str(), id])?;
        }
        tx.commit()?;
        Ok(doomed.len())
    }

    // ---- queries ----

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut snap = Snapshot::default();
        let mut folders = BTreeMap::new();
        let mut lists = BTreeMap::new();
        let mut tasks = BTreeMap::new();
        for ((kind, id), fields) in self.load_all()? {
            match kind {
                EntityKind::Space => {
                    let members = members_of(&fields);
                    if let Some(mut s) = decode::<Space>(&id, fields) {
                        s.members = members;
                        snap.spaces.insert(s.id.clone(), s);
                    }
                }
                EntityKind::Folder => {
                    if let Some(f) = decode::<Folder>(&id, fields) {
                        folders.insert(f.id.clone(), f);
                    }
                }
                EntityKind::List => {
                    if let Some(l) = decode::<List>(&id, fields) {
                        lists.insert(l.id.clone(), l);
                    }
                }
                EntityKind::Task => {
                    if let Some(t) = decode::<Task>(&id, fields) {
                        tasks.insert(t.id.clone(), t);
                    }
                }
            }
        }
        snap.folders = folders
            .iter()
            .filter(|(id, _)| folder_reachable(id, &folders, &snap.spaces))
            .map(|(id, f)| (id.clone(), f.clone()))
            .collect();
        snap.lists = lists
            .into_iter()
            .filter(|(_, l)| match &l.parent {
                ListParent::Space(s) => snap.spaces.contains_key(s),
                ListParent::Folder(f) => snap.folders.contains_key(f),
            })
            .collect();
        snap.tasks = tasks.into_iter().filter(|(_, t)| snap.lists.contains_key(&t.list_id)).collect();
        Ok(snap)
    }

    pub fn spaces(&self) -> Result<Vec<Space>> {
        Ok(self.snapshot()?.spaces.into_values().collect())
    }

    pub fn task(&self, id: &TaskId) -> Result<Option<Task>> {
        Ok(self.snapshot()?.tasks.remove(id))
    }

    /// The folders and lists of a space, nested, each level sorted by `order`.
    pub fn folder_tree(&self, space: &SpaceId) -> Result<Vec<TreeNode>> {
        let snap = self.snapshot()?;
        if !snap.spaces.contains_key(space) {
            return Err(StoreError::NotFound(EntityKind::Space, space.0.clone()));
        }
        Ok(build_level(&snap, &ListParent::Space(space.clone())))
    }

    pub fn tasks_of_list(&self, list: &ListId) -> Result<ListTasks> {
        let snap = self.snapshot()?;
        if !snap.lists.contains_key(list) {
            return Err(StoreError::NotFound(EntityKind::List, list.0.clone()));
        }
        Ok(split_done(snap.tasks.into_values().filter(|t| &t.list_id == list)))
    }

    /// The smart view: tasks from every list anywhere under the folder.
    pub fn smart_view(&self, folder: &FolderId) -> Result<ListTasks> {
        let snap = self.snapshot()?;
        if !snap.folders.contains_key(folder) {
            return Err(StoreError::NotFound(EntityKind::Folder, folder.0.clone()));
        }
        let mut inside: HashSet<&FolderId> = HashSet::from([folder]);
        let mut grew = true;
        while grew {
            grew = false;
            for f in snap.folders.values() {
                if f.parent_id.as_ref().is_some_and(|p| inside.contains(p)) && inside.insert(&f.id) {
                    grew = true;
                }
            }
        }
        let lists: HashSet<&ListId> = snap
            .lists
            .values()
            .filter(|l| matches!(&l.parent, ListParent::Folder(f) if inside.contains(f)))
            .map(|l| &l.id)
            .collect();
        Ok(split_done(snap.tasks.values().filter(|t| lists.contains(&t.list_id)).cloned()))
    }

    /// Open tasks with a due date across every space: before, on, and after `today`.
    pub fn agenda(&self, today: Date) -> Result<Agenda> {
        let mut open: Vec<(Due, Task)> =
            self.snapshot()?.tasks.into_values().filter(|t| !t.done).filter_map(|t| Some((t.due?, t))).collect();
        open.sort_by(|(da, a), (db, b)| da.cmp(db).then(a.order.total_cmp(&b.order)).then(a.id.cmp(&b.id)));
        let mut agenda = Agenda::default();
        for (due, task) in open {
            let bucket = match due.date.cmp(&today) {
                std::cmp::Ordering::Less => &mut agenda.overdue,
                std::cmp::Ordering::Equal => &mut agenda.today,
                std::cmp::Ordering::Greater => &mut agenda.upcoming,
            };
            bucket.push(task);
        }
        Ok(agenda)
    }

    // ---- internals ----

    fn load_all(&self) -> Result<BTreeMap<(EntityKind, String), Fields>> {
        let mut stmt = self.conn.prepare("SELECT kind, entity_id, field, value FROM field")?;
        let mut rows = stmt.query([])?;
        let mut all: BTreeMap<(EntityKind, String), Fields> = BTreeMap::new();
        while let Some(r) = rows.next()? {
            let Some(kind) = EntityKind::parse(&r.get::<_, String>(0)?) else { continue };
            let value: Value = serde_json::from_str(&r.get::<_, String>(3)?)?;
            all.entry((kind, r.get(1)?)).or_default().insert(r.get(2)?, value);
        }
        Ok(all)
    }

    fn fields_of(&self, kind: EntityKind, id: &str) -> Result<Fields> {
        let mut stmt = self.conn.prepare("SELECT field, value FROM field WHERE kind = ?1 AND entity_id = ?2")?;
        let mut rows = stmt.query(params![kind.as_str(), id])?;
        let mut fields = Fields::new();
        while let Some(r) = rows.next()? {
            fields.insert(r.get(0)?, serde_json::from_str(&r.get::<_, String>(1)?)?);
        }
        Ok(fields)
    }

    /// One entity by itself (parents are not checked), or NotFound if absent or deleted.
    fn record<T: DeserializeOwned>(&self, kind: EntityKind, id: &str) -> Result<T> {
        decode(id, self.fields_of(kind, id)?).ok_or_else(|| StoreError::NotFound(kind, id.to_owned()))
    }

    fn require(&self, space: &SpaceId, access: Access) -> Result<()> {
        let fields = self.fields_of(EntityKind::Space, space.as_str())?;
        if fields.is_empty() || fields.get(DELETED) == Some(&Value::Bool(true)) {
            return Err(StoreError::NotFound(EntityKind::Space, space.0.clone()));
        }
        let role = fields.get(&member_field(&self.user)).and_then(|v| serde_json::from_value::<Role>(v.clone()).ok());
        match (role, access) {
            (Some(Role::Owner), _) | (Some(Role::Editor), Access::Edit) => Ok(()),
            _ => Err(StoreError::PermissionDenied),
        }
    }

    fn space_of_parent(&self, parent: &ListParent) -> Result<SpaceId> {
        match parent {
            ListParent::Space(s) => {
                self.record::<Space>(EntityKind::Space, s.as_str())?;
                Ok(s.clone())
            }
            ListParent::Folder(f) => Ok(self.record::<Folder>(EntityKind::Folder, f.as_str())?.space_id),
        }
    }

    fn space_of_list(&self, list: &ListId) -> Result<SpaceId> {
        let list: List = self.record(EntityKind::List, list.as_str())?;
        self.space_of_parent(&list.parent)
    }

    /// `parent` must be a live folder of `space`, and not `moving` or anything under it.
    fn check_folder_parent(&self, space: &SpaceId, parent: &FolderId, moving: Option<&FolderId>) -> Result<()> {
        let mut seen = HashSet::new();
        let mut cur = Some(parent.clone());
        while let Some(id) = cur {
            if Some(&id) == moving {
                return Err(StoreError::Invalid("a folder cannot move inside itself"));
            }
            let f: Folder = self.record(EntityKind::Folder, id.as_str())?;
            if &f.space_id != space {
                return Err(StoreError::Invalid("the parent folder is in another space"));
            }
            if !seen.insert(id) {
                break;
            }
            cur = f.parent_id;
        }
        Ok(())
    }
}

/// Store the change in the log. False if it was already there.
fn log_change(tx: &Transaction, c: &Change) -> Result<bool> {
    let n = tx.execute(
        "INSERT OR IGNORE INTO change_log (change_id, kind, entity_id, body) VALUES (?1, ?2, ?3, ?4)",
        params![c.change_id.as_str(), c.entity_kind.as_str(), c.entity_id, serde_json::to_string(c)?],
    )?;
    Ok(n == 1)
}

/// Last writer wins: the row only changes when the incoming HLC is greater. SQLite compares
/// the row values left to right, the same order as `Hlc`'s derived `Ord`.
fn merge_field(tx: &Transaction, c: &Change) -> Result<()> {
    tx.execute(
        "INSERT INTO field (kind, entity_id, field, value, wall_ms, counter, device)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (kind, entity_id, field) DO UPDATE SET
             value = excluded.value, wall_ms = excluded.wall_ms,
             counter = excluded.counter, device = excluded.device
         WHERE (excluded.wall_ms, excluded.counter, excluded.device)
             > (field.wall_ms, field.counter, field.device)",
        params![
            c.entity_kind.as_str(),
            c.entity_id,
            c.field,
            serde_json::to_string(&c.value)?,
            c.hlc.wall_ms as i64,
            c.hlc.counter,
            c.hlc.device.as_str(),
        ],
    )?;
    Ok(())
}

fn save_clock(tx: &Transaction, clock: &HlcClock) -> Result<()> {
    let last: &Hlc = clock.last();
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('clock', ?1)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [serde_json::to_string(&(last.wall_ms, last.counter))?],
    )?;
    Ok(())
}

/// A UUIDv7 from the injected time, so ids sort roughly by creation without a hidden clock read.
fn new_id(now_ms: u64) -> String {
    let ts = uuid::Timestamp::from_unix(uuid::NoContext, now_ms / 1000, ((now_ms % 1000) * 1_000_000) as u32);
    uuid::Uuid::new_v7(ts).to_string()
}

fn member_field(user: &UserId) -> String {
    format!("{MEMBER_PREFIX}{user}")
}

/// Every field of a new entity except its id, which travels as `entity_id`.
fn entity_fields<T: Serialize>(entity: &T) -> Result<FieldList> {
    let Value::Object(mut map) = serde_json::to_value(entity)? else {
        return Err(StoreError::Invalid("entity did not serialize to an object"));
    };
    map.remove("id");
    Ok(map.into_iter().collect())
}

/// None when deleted or when a required field has not arrived yet.
fn decode<T: DeserializeOwned>(id: &str, mut fields: Fields) -> Option<T> {
    if fields.get(DELETED) == Some(&Value::Bool(true)) {
        return None;
    }
    fields.insert("id".into(), Value::String(id.into()));
    serde_json::from_value(Value::Object(fields)).ok()
}

fn members_of(fields: &Fields) -> Vec<Member> {
    fields
        .iter()
        .filter_map(|(k, v)| {
            let user = k.strip_prefix(MEMBER_PREFIX)?;
            let role = serde_json::from_value(v.clone()).ok()?;
            Some(Member { user: UserId(user.into()), role })
        })
        .collect()
}

/// The id of the entity this one sits directly under, read from raw fields.
fn parent_key(kind: EntityKind, fields: &Fields) -> Option<&str> {
    match kind {
        EntityKind::Space => None,
        EntityKind::Folder => {
            fields.get("parent_id").and_then(Value::as_str).or_else(|| fields.get("space_id")?.as_str())
        }
        EntityKind::List => fields.get("parent")?.as_object()?.values().next()?.as_str(),
        EntityKind::Task => fields.get("list_id")?.as_str(),
    }
}

/// A folder is visible when its whole parent chain exists, stays in its space, and reaches a
/// live space. Two devices moving folders into each other at once can form a cycle; those
/// folders stay hidden until someone moves one out.
fn folder_reachable(id: &FolderId, folders: &BTreeMap<FolderId, Folder>, spaces: &BTreeMap<SpaceId, Space>) -> bool {
    let mut seen = HashSet::new();
    let mut cur = id;
    loop {
        let Some(f) = folders.get(cur) else { return false };
        if !seen.insert(cur) {
            return false;
        }
        match &f.parent_id {
            None => return spaces.contains_key(&f.space_id),
            Some(p) => {
                if folders.get(p).is_some_and(|pf| pf.space_id != f.space_id) {
                    return false;
                }
                cur = p;
            }
        }
    }
}

fn build_level(snap: &Snapshot, at: &ListParent) -> Vec<TreeNode> {
    let mut nodes: Vec<(f64, String, TreeNode)> = Vec::new();
    for f in snap.folders.values() {
        let here = match at {
            ListParent::Space(s) => f.parent_id.is_none() && &f.space_id == s,
            ListParent::Folder(p) => f.parent_id.as_ref() == Some(p),
        };
        if here {
            let children = build_level(snap, &ListParent::Folder(f.id.clone()));
            nodes.push((f.order, f.id.0.clone(), TreeNode::Folder { folder: f.clone(), children }));
        }
    }
    for l in snap.lists.values().filter(|l| &l.parent == at) {
        nodes.push((l.order, l.id.0.clone(), TreeNode::List(l.clone())));
    }
    nodes.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    nodes.into_iter().map(|(_, _, n)| n).collect()
}

fn split_done(tasks: impl Iterator<Item = Task>) -> ListTasks {
    let (mut done, mut todo): (Vec<Task>, Vec<Task>) = tasks.partition(|t| t.done);
    todo.sort_by(|a, b| a.order.total_cmp(&b.order).then(a.created_at.cmp(&b.created_at)).then(a.id.cmp(&b.id)));
    done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then(a.id.cmp(&b.id)));
    ListTasks { todo, done }
}
