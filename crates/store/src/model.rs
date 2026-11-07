//! The data shapes: ids, entities, edits, the wire change, and query results.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::hlc::Hlc;

macro_rules! id_type {
    ($($name:ident),* $(,)?) => {$(
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_owned())
            }
        }
    )*};
}

id_type!(SpaceId, FolderId, ListId, TaskId, UserId, DeviceId, ChangeId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Space,
    Folder,
    List,
    Task,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Space => "space",
            EntityKind::Folder => "folder",
            EntityKind::List => "list",
            EntityKind::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<EntityKind> {
        Some(match s {
            "space" => EntityKind::Space,
            "folder" => EntityKind::Folder,
            "list" => EntityKind::List,
            "task" => EntityKind::Task,
            _ => return None,
        })
    }
}

/// Names one entity of any kind, e.g. for `Store::delete`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityRef {
    Space(SpaceId),
    Folder(FolderId),
    List(ListId),
    Task(TaskId),
}

impl EntityRef {
    pub fn kind(&self) -> EntityKind {
        match self {
            EntityRef::Space(_) => EntityKind::Space,
            EntityRef::Folder(_) => EntityKind::Folder,
            EntityRef::List(_) => EntityKind::List,
            EntityRef::Task(_) => EntityKind::Task,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            EntityRef::Space(id) => id.as_str(),
            EntityRef::Folder(id) => id.as_str(),
            EntityRef::List(id) => id.as_str(),
            EntityRef::Task(id) => id.as_str(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user: UserId,
    pub role: Role,
}

/// A top-level folder. Private when only its owner is a member.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub shared: bool,
    /// Stored as one field per member so two people inviting at once both land.
    #[serde(skip)]
    pub members: Vec<Member>,
}

impl Space {
    pub fn role_of(&self, user: &UserId) -> Option<Role> {
        self.members.iter().find(|m| &m.user == user).map(|m| m.role)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub space_id: SpaceId,
    /// None means the folder sits directly in the space.
    pub parent_id: Option<FolderId>,
    pub name: String,
    pub order: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListParent {
    Folder(FolderId),
    Space(SpaceId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct List {
    pub id: ListId,
    pub parent: ListParent,
    pub name: String,
    pub order: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekly,
}

/// A calendar date, written `YYYY-MM-DD`. Only valid dates can be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (1..=days).contains(&day).then_some(Date { year, month, day })
    }

    pub fn year(self) -> i32 {
        self.year
    }
    pub fn month(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl FromStr for Date {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, '-');
        let mut next = || parts.next().ok_or("expected YYYY-MM-DD");
        let (y, m, d) = (next()?, next()?, next()?);
        let bad = "expected YYYY-MM-DD";
        Date::new(y.parse().map_err(|_| bad)?, m.parse().map_err(|_| bad)?, d.parse().map_err(|_| bad)?)
            .ok_or("no such date")
    }
}

/// A wall-clock time of day, written `HH:MM`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    hour: u8,
    minute: u8,
}

impl TimeOfDay {
    pub fn new(hour: u8, minute: u8) -> Option<TimeOfDay> {
        (hour < 24 && minute < 60).then_some(TimeOfDay { hour, minute })
    }
    pub fn hour(self) -> u8 {
        self.hour
    }
    pub fn minute(self) -> u8 {
        self.minute
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl FromStr for TimeOfDay {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = "expected HH:MM";
        let (h, m) = s.split_once(':').ok_or(bad)?;
        TimeOfDay::new(h.parse().map_err(|_| bad)?, m.parse().map_err(|_| bad)?).ok_or("no such time")
    }
}

macro_rules! serde_via_str {
    ($($t:ty),*) => {$(
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?.parse().map_err(de::Error::custom)
            }
        }
    )*};
}

serde_via_str!(Date, TimeOfDay);

/// A due date with an optional time. Sorts all-day before timed on the same date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Due {
    pub date: Date,
    #[serde(default)]
    pub time: Option<TimeOfDay>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub list_id: ListId,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub due: Option<Due>,
    #[serde(default)]
    pub repeat: Repeat,
    pub assignee: Option<UserId>,
    #[serde(default)]
    pub done: bool,
    pub created_by: UserId,
    pub completed_by: Option<UserId>,
    /// Unix milliseconds.
    pub created_at: u64,
    pub completed_at: Option<u64>,
    #[serde(default)]
    pub order: f64,
}

/// What the caller supplies for a new task; the store fills in id, author and timestamps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub due: Option<Due>,
    pub repeat: Repeat,
    pub assignee: Option<UserId>,
    pub order: f64,
}

impl NewTask {
    pub fn titled(title: &str) -> NewTask {
        NewTask { title: title.into(), ..NewTask::default() }
    }
}

/// One field edit each, so each becomes exactly one change.
#[derive(Clone, Debug, PartialEq)]
pub enum SpaceEdit {
    Name(String),
    Shared(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FolderEdit {
    Name(String),
    Parent(Option<FolderId>),
    Order(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListEdit {
    Name(String),
    Parent(ListParent),
    Order(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TaskEdit {
    Title(String),
    Notes(String),
    Due(Option<Due>),
    Repeat(Repeat),
    Assignee(Option<UserId>),
    List(ListId),
    Order(f64),
}

/// One field of one entity set to one value at one HLC. This is what goes over the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub change_id: ChangeId,
    /// Which space's relay the change belongs to.
    pub space_id: SpaceId,
    pub entity_kind: EntityKind,
    pub entity_id: String,
    pub field: String,
    pub value: serde_json::Value,
    pub hlc: Hlc,
    pub device: DeviceId,
}

/// Every live entity. Deleted ones, ones under a deleted parent, and ones whose fields have
/// not all arrived yet are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub spaces: BTreeMap<SpaceId, Space>,
    pub folders: BTreeMap<FolderId, Folder>,
    pub lists: BTreeMap<ListId, List>,
    pub tasks: BTreeMap<TaskId, Task>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TreeNode {
    Folder { folder: Folder, children: Vec<TreeNode> },
    List(List),
}

/// Tasks split the way the panel's To do and Done tabs show them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListTasks {
    /// By `order`.
    pub todo: Vec<Task>,
    /// Most recently completed first.
    pub done: Vec<Task>,
}

/// Open tasks with a due date, relative to a given day. Each bucket is sorted by due.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agenda {
    pub overdue: Vec<Task>,
    pub today: Vec<Task>,
    pub upcoming: Vec<Task>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_validates_and_round_trips() {
        assert!(Date::new(2026, 2, 29).is_none());
        assert!(Date::new(2028, 2, 29).is_some());
        assert!(Date::new(2026, 13, 1).is_none());
        let d: Date = "2026-11-06".parse().unwrap();
        assert_eq!(d, Date::new(2026, 11, 6).unwrap());
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2026-11-06\"");
        assert!("2026-11-31".parse::<Date>().is_err());
        assert!(serde_json::from_str::<Date>("\"06/11/2026\"").is_err());
    }

    #[test]
    fn due_orders_all_day_first() {
        let date = Date::new(2026, 11, 6).unwrap();
        let all_day = Due { date, time: None };
        let nine = Due { date, time: TimeOfDay::new(9, 0) };
        assert!(all_day < nine);
        assert_eq!(serde_json::to_string(&nine).unwrap(), r#"{"date":"2026-11-06","time":"09:00"}"#);
        assert!(TimeOfDay::new(24, 0).is_none());
    }
}
