//! Local behavior: CRUD, nesting, queries, persistence, permissions, tombstones.

use app_store::*;

const T0: u64 = 1_793_000_000_000; // early Nov 2026
const DAY: u64 = 24 * 60 * 60 * 1000;

fn store(device: &str, user: &str) -> Store {
    Store::open_in_memory(device.into(), user.into()).unwrap()
}

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

fn names(nodes: &[TreeNode]) -> Vec<String> {
    nodes
        .iter()
        .map(|n| match n {
            TreeNode::Folder { folder, children } => format!("{}/[{}]", folder.name, names(children).join(",")),
            TreeNode::List(l) => l.name.clone(),
        })
        .collect()
}

fn titles(tasks: &[Task]) -> Vec<&str> {
    tasks.iter().map(|t| t.title.as_str()).collect()
}

/// The ORB_SPEC 6.2 example tree, inside one space each.
struct Example {
    family: SpaceId,
    rumah: FolderId,
    belanja: ListId,
    tagihan: ListId,
    liburan: ListId,
}

fn spec_example(s: &mut Store) -> Example {
    let family = s.create_space("Family", T0).unwrap();
    let rumah = s.create_folder(&family, None, "Rumah", 1.0, T0).unwrap();
    let belanja = s.create_list(ListParent::Folder(rumah.clone()), "Belanja", 1.0, T0).unwrap();
    let tagihan = s.create_list(ListParent::Folder(rumah.clone()), "Tagihan", 2.0, T0).unwrap();
    let liburan = s.create_list(ListParent::Space(family.clone()), "Liburan", 2.0, T0).unwrap();
    Example { family, rumah, belanja, tagihan, liburan }
}

#[test]
fn spec_folder_tree() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let work = s.create_space("Work", T0).unwrap();
    let client = s.create_folder(&work, None, "Client A", 1.0, T0).unwrap();
    s.create_list(ListParent::Folder(client.clone()), "Invoice", 2.0, T0).unwrap();
    s.create_list(ListParent::Folder(client), "Sprint", 1.0, T0).unwrap();
    s.create_list(ListParent::Space(work.clone()), "Admin", 2.0, T0).unwrap();

    assert_eq!(names(&s.folder_tree(&ex.family).unwrap()), ["Rumah/[Belanja,Tagihan]", "Liburan"]);
    assert_eq!(names(&s.folder_tree(&work).unwrap()), ["Client A/[Sprint,Invoice]", "Admin"]);
    let spaces = s.spaces().unwrap();
    assert_eq!(spaces.len(), 2);
    assert!(spaces.iter().all(|sp| !sp.shared && sp.role_of(&"ayah".into()) == Some(Role::Owner)));
}

#[test]
fn folders_nest_to_any_depth_and_move() {
    let mut s = store("pc", "ayah");
    let sp = s.create_space("Deep", T0).unwrap();
    let mut parent = None;
    let mut ids = vec![];
    for i in 0..6 {
        let f = s.create_folder(&sp, parent.as_ref(), &format!("L{i}"), 0.0, T0).unwrap();
        parent = Some(f.clone());
        ids.push(f);
    }
    let leaf = s.create_list(ListParent::Folder(ids[5].clone()), "Leaf", 0.0, T0).unwrap();
    assert_eq!(names(&s.folder_tree(&sp).unwrap()), ["L0/[L1/[L2/[L3/[L4/[L5/[Leaf]]]]]]"]);

    // A folder cannot go inside itself or its own descendant.
    let err = s.edit_folder(&ids[1], FolderEdit::Parent(Some(ids[4].clone())), T0 + 1).unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)));
    assert!(matches!(
        s.edit_folder(&ids[1], FolderEdit::Parent(Some(ids[1].clone())), T0 + 1),
        Err(StoreError::Invalid(_))
    ));

    // Moving L3 to the top keeps its subtree.
    s.edit_folder(&ids[3], FolderEdit::Parent(None), T0 + 2).unwrap();
    s.edit_folder(&ids[3], FolderEdit::Order(1.0), T0 + 2).unwrap();
    assert_eq!(names(&s.folder_tree(&sp).unwrap()), ["L0/[L1/[L2/[]]]", "L3/[L4/[L5/[Leaf]]]"]);

    s.edit_list(&leaf, ListEdit::Parent(ListParent::Space(sp.clone())), T0 + 3).unwrap();
    s.edit_list(&leaf, ListEdit::Name("Top".into()), T0 + 3).unwrap();
    s.edit_list(&leaf, ListEdit::Order(0.5), T0 + 3).unwrap();
    assert_eq!(names(&s.folder_tree(&sp).unwrap()), ["L0/[L1/[L2/[]]]", "Top", "L3/[L4/[L5/[]]]"]);
}

#[test]
fn task_crud_and_done_tab() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let galon = s.create_task(&ex.belanja, NewTask { order: 1.0, ..NewTask::titled("Beli galon") }, T0).unwrap();
    let gula = s.create_task(&ex.belanja, NewTask { order: 2.0, ..NewTask::titled("Gula 1 kg") }, T0 + 1).unwrap();

    let t = s.task(&galon).unwrap().unwrap();
    assert_eq!((t.created_by.as_str(), t.created_at, t.done), ("ayah", T0, false));
    assert_eq!(t.repeat, Repeat::None);

    s.edit_task(&galon, TaskEdit::Title("Beli galon 2".into()), T0 + 10).unwrap();
    s.edit_task(&galon, TaskEdit::Notes("Aqua".into()), T0 + 10).unwrap();
    s.edit_task(&galon, TaskEdit::Repeat(Repeat::Weekly), T0 + 10).unwrap();
    s.edit_task(&galon, TaskEdit::Assignee(Some("ibu".into())), T0 + 10).unwrap();
    let due = Due { date: date("2026-11-07"), time: TimeOfDay::new(9, 0) };
    s.edit_task(&galon, TaskEdit::Due(Some(due)), T0 + 10).unwrap();
    let t = s.task(&galon).unwrap().unwrap();
    assert_eq!((t.title.as_str(), t.notes.as_str()), ("Beli galon 2", "Aqua"));
    assert_eq!((t.repeat, t.assignee.clone(), t.due), (Repeat::Weekly, Some("ibu".into()), Some(due)));

    s.set_done(&gula, true, T0 + 20).unwrap();
    let tabs = s.tasks_of_list(&ex.belanja).unwrap();
    assert_eq!(titles(&tabs.todo), ["Beli galon 2"]);
    assert_eq!(titles(&tabs.done), ["Gula 1 kg"]);
    let g = &tabs.done[0];
    assert_eq!((g.completed_by.clone(), g.completed_at), (Some("ayah".into()), Some(T0 + 20)));

    s.set_done(&gula, false, T0 + 30).unwrap();
    let g = s.task(&gula).unwrap().unwrap();
    assert_eq!((g.done, g.completed_by, g.completed_at), (false, None, None));

    s.edit_task(&gula, TaskEdit::List(ex.tagihan.clone()), T0 + 40).unwrap();
    assert_eq!(titles(&s.tasks_of_list(&ex.tagihan).unwrap().todo), ["Gula 1 kg"]);

    s.delete(&EntityRef::Task(galon.clone()), T0 + 50).unwrap();
    assert_eq!(s.task(&galon).unwrap(), None);
    assert!(s.tasks_of_list(&ex.belanja).unwrap().todo.is_empty());
    assert!(matches!(s.edit_task(&galon, TaskEdit::Title("x".into()), T0 + 60), Err(StoreError::NotFound(..))));
}

#[test]
fn smart_view_collects_all_descendant_lists() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let dapur = s.create_folder(&ex.family, Some(&ex.rumah), "Dapur", 3.0, T0).unwrap();
    let bumbu = s.create_list(ListParent::Folder(dapur), "Bumbu", 0.0, T0).unwrap();
    s.create_task(&ex.belanja, NewTask { order: 1.0, ..NewTask::titled("Galon") }, T0).unwrap();
    let listrik = s.create_task(&ex.tagihan, NewTask { order: 2.0, ..NewTask::titled("Listrik") }, T0).unwrap();
    s.create_task(&bumbu, NewTask { order: 3.0, ..NewTask::titled("Garam") }, T0).unwrap();
    s.create_task(&ex.liburan, NewTask::titled("Tiket"), T0).unwrap();
    s.set_done(&listrik, true, T0 + 1).unwrap();

    let view = s.smart_view(&ex.rumah).unwrap();
    assert_eq!(titles(&view.todo), ["Galon", "Garam"]);
    assert_eq!(titles(&view.done), ["Listrik"]);
}

#[test]
fn agenda_buckets_by_today() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let mk = |s: &mut Store, title: &str, d: &str, time: Option<TimeOfDay>| {
        let due = Some(Due { date: date(d), time });
        s.create_task(&ex.belanja, NewTask { due, ..NewTask::titled(title) }, T0).unwrap()
    };
    mk(&mut s, "Tagihan air", "2026-11-03", None);
    mk(&mut s, "Galon 9am", "2026-11-06", TimeOfDay::new(9, 0));
    mk(&mut s, "Galon all day", "2026-11-06", None);
    mk(&mut s, "Lusa", "2026-11-08", None);
    mk(&mut s, "Besok", "2026-11-07", TimeOfDay::new(15, 0));
    let done = mk(&mut s, "Done yesterday", "2026-11-05", None);
    s.set_done(&done, true, T0).unwrap();
    s.create_task(&ex.belanja, NewTask::titled("No due"), T0).unwrap();

    let a = s.agenda(date("2026-11-06")).unwrap();
    assert_eq!(titles(&a.overdue), ["Tagihan air"]);
    assert_eq!(titles(&a.today), ["Galon all day", "Galon 9am"]);
    assert_eq!(titles(&a.upcoming), ["Besok", "Lusa"]);
}

#[test]
fn deleting_a_folder_hides_everything_under_it() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let due = Some(Due { date: date("2026-11-06"), time: None });
    s.create_task(&ex.tagihan, NewTask { due, ..NewTask::titled("Listrik") }, T0).unwrap();
    s.delete(&EntityRef::Folder(ex.rumah.clone()), T0 + 1).unwrap();

    assert_eq!(names(&s.folder_tree(&ex.family).unwrap()), ["Liburan"]);
    assert!(s.agenda(date("2026-11-06")).unwrap().today.is_empty());
    assert!(matches!(s.tasks_of_list(&ex.tagihan), Err(StoreError::NotFound(EntityKind::List, _))));
    assert!(matches!(s.smart_view(&ex.rumah), Err(StoreError::NotFound(EntityKind::Folder, _))));
}

#[test]
fn persists_across_reopen() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("persist-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let (family, belanja, task, last_hlc) = {
        let mut s = Store::open(&path, "pc".into(), "ayah".into()).unwrap();
        let ex = spec_example(&mut s);
        let t = s.create_task(&ex.belanja, NewTask::titled("Beli galon"), T0).unwrap();
        s.set_done(&t, true, T0 + 5).unwrap();
        let last = s.outbox().unwrap().last().unwrap().hlc.clone();
        (ex.family, ex.belanja, t, last)
    };

    let mut s = Store::open(&path, "pc".into(), "ayah".into()).unwrap();
    assert_eq!(names(&s.folder_tree(&family).unwrap()), ["Rumah/[Belanja,Tagihan]", "Liburan"]);
    let tabs = s.tasks_of_list(&belanja).unwrap();
    assert_eq!(titles(&tabs.done), ["Beli galon"]);
    // Space 3 fields, folder 4, three lists 3 each, task 12, tick 3.
    assert_eq!(s.outbox().unwrap().len(), 31, "outbox survives a restart");

    // The clock resumes: a write with an earlier wall time still sorts after everything before.
    s.edit_task(&task, TaskEdit::Title("Beli galon lagi".into()), T0 - 1000).unwrap();
    let newest = s.outbox().unwrap().last().unwrap().hlc.clone();
    assert!(newest > last_hlc);
    assert_eq!(s.task(&task).unwrap().unwrap().title, "Beli galon lagi");
    drop(s);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn every_write_lands_in_the_outbox_per_field() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let before = s.outbox().unwrap().len();
    let t = s.create_task(&ex.belanja, NewTask::titled("Galon"), T0 + 1).unwrap();
    let created = s.outbox().unwrap();
    // id, done and the rest: every Task field but id is its own change.
    assert_eq!(created.len() - before, 12);
    s.set_done(&t, true, T0 + 2).unwrap();
    let out = s.outbox().unwrap();
    let last: Vec<&str> = out[out.len() - 3..].iter().map(|c| c.field.as_str()).collect();
    assert_eq!(last, ["done", "completed_by", "completed_at"]);
    assert!(out.iter().all(|c| c.space_id == ex.family && c.device.as_str() == "pc"));
    assert!(out.windows(2).all(|w| w[0].hlc < w[1].hlc));

    let ids: Vec<ChangeId> = out[..10].iter().map(|c| c.change_id.clone()).collect();
    s.ack(&ids).unwrap();
    assert_eq!(s.outbox().unwrap().len(), out.len() - 10);
}

#[test]
fn roles_gate_local_writes() {
    let mut owner = store("pc", "ayah");
    let ex = spec_example(&mut owner);
    owner.set_member(&ex.family, &"ibu".into(), Some(Role::Editor), T0).unwrap();
    owner.set_member(&ex.family, &"adik".into(), Some(Role::Viewer), T0).unwrap();
    owner.edit_space(&ex.family, SpaceEdit::Shared(true), T0).unwrap();
    let all = owner.outbox().unwrap();

    let mut editor = store("phone", "ibu");
    editor.apply_remote(&all).unwrap();
    let t = editor.create_task(&ex.belanja, NewTask::titled("Gas elpiji"), T0 + 1).unwrap();
    editor.set_done(&t, true, T0 + 2).unwrap();
    assert!(matches!(
        editor.set_member(&ex.family, &"om".into(), Some(Role::Editor), T0),
        Err(StoreError::PermissionDenied)
    ));
    assert!(matches!(editor.delete(&EntityRef::Space(ex.family.clone()), T0), Err(StoreError::PermissionDenied)));

    let mut viewer = store("tablet", "adik");
    viewer.apply_remote(&all).unwrap();
    assert!(matches!(viewer.create_task(&ex.belanja, NewTask::titled("x"), T0), Err(StoreError::PermissionDenied)));
    let space = &viewer.spaces().unwrap()[0];
    assert!(space.shared);
    assert_eq!(space.role_of(&"adik".into()), Some(Role::Viewer));
    assert_eq!(space.members.len(), 3);

    let mut stranger = store("laptop", "tamu");
    stranger.apply_remote(&all).unwrap();
    assert!(matches!(stranger.create_folder(&ex.family, None, "x", 0.0, T0), Err(StoreError::PermissionDenied)));

    owner.set_member(&ex.family, &"adik".into(), None, T0 + 3).unwrap();
    assert_eq!(owner.spaces().unwrap()[0].members.len(), 2);
}

#[test]
fn tombstones_purge_after_thirty_days() {
    let mut s = store("pc", "ayah");
    let ex = spec_example(&mut s);
    let keep = s.create_task(&ex.liburan, NewTask::titled("Tiket"), T0).unwrap();
    s.create_task(&ex.tagihan, NewTask::titled("Listrik"), T0).unwrap();
    s.delete(&EntityRef::Folder(ex.rumah.clone()), T0 + 1).unwrap();

    // Unsent changes hold the purge back, so the delete itself is never lost.
    assert_eq!(s.purge_tombstones(T0 + 1 + TOMBSTONE_TTL_MS).unwrap(), 0);
    let sent: Vec<ChangeId> = s.outbox().unwrap().into_iter().map(|c| c.change_id).collect();
    s.ack(&sent).unwrap();

    assert_eq!(s.purge_tombstones(T0 + TOMBSTONE_TTL_MS).unwrap(), 0, "29 days, 23:59:59.999");
    // Rumah, Belanja, Tagihan and Listrik go.
    assert_eq!(s.purge_tombstones(T0 + 1 + TOMBSTONE_TTL_MS).unwrap(), 4);
    assert_eq!(s.purge_tombstones(T0 + 60 * DAY).unwrap(), 0);
    assert_eq!(names(&s.folder_tree(&ex.family).unwrap()), ["Liburan"]);
    assert!(s.task(&keep).unwrap().is_some());
}
