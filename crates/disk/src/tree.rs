//! The scan result: an arena of nodes with totals kept up to date while the scan runs,
//! plus the read-only queries the panels need.

use crate::category::{Category, TypeShare};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Index of a node in its [`Tree`]. Only meaningful for the tree that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    fn ix(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    /// A junction, symbolic link or mount point. Listed, never entered, counts zero bytes.
    Link,
}

#[derive(Debug)]
pub struct Node {
    name: Box<OsStr>,
    parent: Option<NodeId>,
    kind: NodeKind,
    size: u64,
    allocated: u64,
    files: u64,
    dirs: u64,
    modified: Option<SystemTime>,
    hidden: bool,
    flags: u8,
    children: Vec<NodeId>,
}

/// [`Node`] flags from the listing: a system file, data not on this disk (cloud placeholder or
/// offline), any reparse point.
pub(crate) const F_SYSTEM: u8 = 1;
pub(crate) const F_NOT_LOCAL: u8 = 2;
pub(crate) const F_REPARSE: u8 = 4;

impl Node {
    pub fn name(&self) -> &OsStr {
        &self.name
    }
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }
    pub fn kind(&self) -> NodeKind {
        self.kind
    }
    pub fn is_dir(&self) -> bool {
        self.kind == NodeKind::Dir
    }
    /// Logical bytes; for a folder, the whole subtree.
    pub fn size(&self) -> u64 {
        self.size
    }
    /// Bytes on disk: cluster rounded, compressed size for compressed or sparse files, and
    /// zero for cloud placeholders whose data is not local.
    pub fn allocated(&self) -> u64 {
        self.allocated
    }
    /// Files in the subtree (1 for a file).
    pub fn files(&self) -> u64 {
        self.files
    }
    /// Folders below this one (not counting itself).
    pub fn dirs(&self) -> u64 {
        self.dirs
    }
    /// For a folder, the newest modified time anywhere in its subtree.
    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
    pub fn hidden(&self) -> bool {
        self.hidden
    }
    /// Windows marks it as a system file.
    pub fn system(&self) -> bool {
        self.flags & F_SYSTEM != 0
    }
    /// A cloud placeholder or offline file: its data is not on this disk.
    pub fn not_local(&self) -> bool {
        self.flags & F_NOT_LOCAL != 0
    }
    /// Any reparse point, including the ones the scan treats as plain files or folders.
    pub fn reparse(&self) -> bool {
        self.flags & F_REPARSE != 0
    }
}

/// One row of a folder listing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChildShare {
    pub id: NodeId,
    pub size: u64,
    /// 0 to 100. Zero when the parent is empty.
    pub percent_of_parent: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanStatus {
    Running,
    Complete,
    /// Stop was pressed: some folders were not read, so totals are a lower bound.
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    AccessDenied,
    /// Deleted between being listed and being read.
    Vanished,
    Other(std::io::ErrorKind),
}

impl SkipReason {
    pub(crate) fn of(e: &std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::PermissionDenied => Self::AccessDenied,
            std::io::ErrorKind::NotFound => Self::Vanished,
            k => Self::Other(k),
        }
    }
}

/// A path the scan could not read and therefore did not count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// What one listed entry adds to the tree.
pub(crate) struct NewEntry {
    pub name: Box<OsStr>,
    pub kind: NodeKind,
    pub size: u64,
    pub allocated: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    pub flags: u8,
}

#[derive(Debug)]
pub struct Tree {
    root_path: PathBuf,
    nodes: Vec<Node>,
    skipped: Vec<Skipped>,
    status: ScanStatus,
}

impl Tree {
    pub(crate) fn new(root_path: PathBuf, modified: Option<SystemTime>) -> Self {
        let root = Node {
            name: root_path.as_os_str().into(),
            parent: None,
            kind: NodeKind::Dir,
            size: 0,
            allocated: 0,
            files: 0,
            dirs: 0,
            modified,
            hidden: false,
            flags: 0,
            children: Vec::new(),
        };
        Tree { root_path, nodes: vec![root], skipped: Vec::new(), status: ScanStatus::Running }
    }

    /// Adds the entries of one folder and pushes their totals up to every ancestor, so
    /// the tree is consistent after each call. Returns the ids of the new subfolders.
    pub(crate) fn add_listing(&mut self, dir: NodeId, entries: Vec<NewEntry>) -> Vec<NodeId> {
        let (mut size, mut alloc, mut files, mut dirs) = (0u64, 0u64, 0u64, 0u64);
        let mut newest = None;
        let mut subdirs = Vec::new();
        for e in entries {
            let id = NodeId(self.nodes.len() as u32);
            match e.kind {
                NodeKind::File => files += 1,
                NodeKind::Dir => {
                    dirs += 1;
                    subdirs.push(id);
                }
                NodeKind::Link => {}
            }
            let counted = e.kind == NodeKind::File;
            if counted {
                size += e.size;
                alloc += e.allocated;
            }
            newest = newest.max(e.modified);
            self.nodes.push(Node {
                name: e.name,
                parent: Some(dir),
                kind: e.kind,
                size: if counted { e.size } else { 0 },
                allocated: if counted { e.allocated } else { 0 },
                files: u64::from(counted),
                dirs: 0,
                modified: e.modified,
                hidden: e.hidden,
                flags: e.flags,
                children: Vec::new(),
            });
            self.nodes[dir.ix()].children.push(id);
        }
        let mut at = Some(dir);
        while let Some(id) = at {
            let n = &mut self.nodes[id.ix()];
            n.size += size;
            n.allocated += alloc;
            n.files += files;
            n.dirs += dirs;
            n.modified = n.modified.max(newest);
            at = n.parent;
        }
        subdirs
    }

    pub(crate) fn skip(&mut self, s: Skipped) {
        self.skipped.push(s);
    }

    pub(crate) fn finish(&mut self, status: ScanStatus) {
        self.status = status;
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// The scanned folder, as an absolute path.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.ix()]
    }

    /// Number of nodes, root included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn status(&self) -> ScanStatus {
        self.status
    }

    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    /// From the root down to `id`, both included. Drives the breadcrumb.
    pub fn ancestry(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        let mut at = id;
        while let Some(p) = self.node(at).parent {
            out.push(p);
            at = p;
        }
        out.reverse();
        out
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut p = self.root_path.clone();
        for n in self.ancestry(id).into_iter().skip(1) {
            p.push(self.node(n).name());
        }
        p
    }

    /// Finds the node at `path`, which must be inside the scanned root.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rest = path.strip_prefix(&self.root_path).ok()?;
        let mut at = self.root();
        for part in rest.components() {
            let name = part.as_os_str();
            at = *self.children(at).iter().find(|&&c| self.node(c).name().eq_ignore_ascii_case(name))?;
        }
        Some(at)
    }

    /// Children biggest first, ties by name, each with its share of the parent.
    pub fn children_by_size(&self, id: NodeId) -> Vec<ChildShare> {
        let total = self.node(id).size;
        let mut rows: Vec<ChildShare> = self
            .children(id)
            .iter()
            .map(|&c| {
                let size = self.node(c).size;
                let percent_of_parent = if total == 0 { 0.0 } else { size as f64 * 100.0 / total as f64 };
                ChildShare { id: c, size, percent_of_parent }
            })
            .collect();
        rows.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| self.node(a.id).name.cmp(&self.node(b.id).name)));
        rows
    }

    /// The `n` biggest files anywhere under `id`, biggest first.
    pub fn largest_files(&self, id: NodeId, n: usize) -> Vec<NodeId> {
        if n == 0 {
            return Vec::new();
        }
        // Min-heap of the best n so far; ties prefer the earlier node for a stable order.
        let mut heap = BinaryHeap::with_capacity(n + 1);
        for f in self.files_under(id) {
            heap.push(Reverse((self.node(f).size, Reverse(f))));
            if heap.len() > n {
                heap.pop();
            }
        }
        let mut out: Vec<_> = heap.into_iter().map(|Reverse((s, Reverse(f)))| (s, f)).collect();
        out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        out.into_iter().map(|(_, f)| f).collect()
    }

    /// Bytes and file counts per category under `id`. All six categories are present:
    /// the five named ones biggest first, then Other last (it has its own swatch).
    pub fn type_breakdown(&self, id: NodeId) -> Vec<TypeShare> {
        let mut shares: Vec<TypeShare> = Category::ALL.iter().map(|&category| TypeShare { category, bytes: 0, files: 0 }).collect();
        for f in self.files_under(id) {
            let node = self.node(f);
            let s = &mut shares[Category::of(node.name()) as usize];
            s.bytes += node.size;
            s.files += 1;
        }
        shares.sort_by(|a, b| {
            (a.category == Category::Other)
                .cmp(&(b.category == Category::Other))
                .then(b.bytes.cmp(&a.bytes))
                .then((a.category as usize).cmp(&(b.category as usize)))
        });
        shares
    }

    fn files_under(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = vec![id];
        std::iter::from_fn(move || {
            while let Some(n) = stack.pop() {
                let node = self.node(n);
                match node.kind {
                    NodeKind::File => return Some(n),
                    NodeKind::Dir => stack.extend_from_slice(&node.children),
                    NodeKind::Link => {}
                }
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> NewEntry {
        NewEntry { name: OsStr::new(name).into(), kind: NodeKind::File, size, allocated: size, modified: None, hidden: false, flags: 0 }
    }
    fn dir(name: &str) -> NewEntry {
        NewEntry { name: OsStr::new(name).into(), kind: NodeKind::Dir, size: 0, allocated: 0, modified: None, hidden: false, flags: 0 }
    }

    #[test]
    fn totals_reach_every_ancestor_as_listings_arrive() {
        let mut t = Tree::new(PathBuf::from(r"C:\x"), None);
        let subs = t.add_listing(t.root(), vec![file("a.txt", 10), dir("sub")]);
        assert_eq!(t.node(t.root()).size(), 10);
        let deeper = t.add_listing(subs[0], vec![dir("deep")]);
        t.add_listing(deeper[0], vec![file("b.mp4", 90)]);
        assert_eq!(t.node(t.root()).size(), 100);
        assert_eq!(t.node(t.root()).files(), 2);
        assert_eq!(t.node(t.root()).dirs(), 2);
        assert_eq!(t.node(subs[0]).size(), 90);
        let rows = t.children_by_size(t.root());
        assert_eq!(rows[0].id, subs[0]);
        assert!((rows[0].percent_of_parent - 90.0).abs() < 1e-9);
        assert!((rows[1].percent_of_parent - 10.0).abs() < 1e-9);
    }

    #[test]
    fn empty_parent_gives_zero_percent_not_nan() {
        let mut t = Tree::new(PathBuf::from(r"C:\x"), None);
        t.add_listing(t.root(), vec![file("empty", 0)]);
        assert_eq!(t.children_by_size(t.root())[0].percent_of_parent, 0.0);
    }

    #[test]
    fn links_are_listed_but_count_nothing() {
        let mut t = Tree::new(PathBuf::from(r"C:\x"), None);
        let link = NewEntry { kind: NodeKind::Link, size: 500, ..file("junction", 500) };
        t.add_listing(t.root(), vec![link]);
        assert_eq!(t.children(t.root()).len(), 1);
        assert_eq!(t.node(t.root()).size(), 0);
        assert_eq!(t.node(t.root()).files(), 0);
    }
}
