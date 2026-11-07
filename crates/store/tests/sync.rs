//! Convergence: stores on different devices edit concurrently, swap outboxes in different
//! orders, and must end in the same state.

use app_store::*;

const T0: u64 = 1_793_000_000_000;

fn store(device: &str, user: &str) -> Store {
    Store::open_in_memory(device.into(), user.into()).unwrap()
}

/// Ayah's PC sets up a shared Family space with Ibu as editor, and Ibu's phone pulls it.
/// Also returns the setup changes, which the PC then acks.
fn family() -> (Store, Store, Vec<Change>, ListId, TaskId, TaskId) {
    let mut pc = store("pc", "ayah");
    let family = pc.create_space("Family", T0).unwrap();
    pc.set_member(&family, &"ibu".into(), Some(Role::Editor), T0).unwrap();
    pc.edit_space(&family, SpaceEdit::Shared(true), T0).unwrap();
    let rumah = pc.create_folder(&family, None, "Rumah", 0.0, T0).unwrap();
    let belanja = pc.create_list(ListParent::Folder(rumah), "Belanja", 0.0, T0).unwrap();
    let galon = pc.create_task(&belanja, NewTask::titled("Beli galon"), T0).unwrap();
    let gula = pc.create_task(&belanja, NewTask::titled("Gula 1 kg"), T0).unwrap();

    let setup = pc.outbox().unwrap();
    let mut phone = store("phone", "ibu");
    phone.apply_remote(&setup).unwrap();
    assert_eq!(phone.snapshot().unwrap(), pc.snapshot().unwrap());
    pc.ack(&setup.iter().map(|c| c.change_id.clone()).collect::<Vec<_>>()).unwrap();
    (pc, phone, setup, belanja, galon, gula)
}

/// A fixed permutation that is neither the original order nor its reverse: odd positions
/// backwards, then even positions forwards.
fn shuffle(v: Vec<Change>) -> Vec<Change> {
    let (even, odd): (Vec<_>, Vec<_>) = v.into_iter().enumerate().partition(|(i, _)| i % 2 == 0);
    odd.into_iter().rev().chain(even).map(|(_, c)| c).collect()
}
#[test]
fn rename_and_tick_done_both_survive() {
    let (mut pc, mut phone, _, belanja, galon, _) = family();

    // Offline at the same moment: the PC renames, the phone ticks it done.
    pc.edit_task(&galon, TaskEdit::Title("Beli galon 2".into()), T0 + 100).unwrap();
    phone.set_done(&galon, true, T0 + 100).unwrap();

    let from_pc = pc.outbox().unwrap();
    let from_phone = phone.outbox().unwrap();
    phone.apply_remote(&from_pc.iter().rev().cloned().collect::<Vec<_>>()).unwrap();
    pc.apply_remote(&shuffle(from_phone)).unwrap();

    assert_eq!(pc.snapshot().unwrap(), phone.snapshot().unwrap());
    let t = pc.task(&galon).unwrap().unwrap();
    assert_eq!(t.title, "Beli galon 2");
    assert!(t.done);
    assert_eq!(t.completed_by, Some("ibu".into()));
    assert_eq!(t.created_by, "ayah".into());
    assert_eq!(pc.tasks_of_list(&belanja).unwrap().done.len(), 1);
}

#[test]
fn same_field_last_writer_wins_whatever_the_order() {
    let (mut pc, mut phone, _, _, galon, _) = family();
    pc.edit_task(&galon, TaskEdit::Title("from pc".into()), T0 + 200).unwrap();
    phone.edit_task(&galon, TaskEdit::Title("from phone".into()), T0 + 100).unwrap();

    let from_pc = pc.outbox().unwrap();
    let from_phone = phone.outbox().unwrap();
    pc.apply_remote(&from_phone).unwrap();
    phone.apply_remote(&from_pc).unwrap();

    assert_eq!(pc.snapshot().unwrap(), phone.snapshot().unwrap());
    assert_eq!(phone.task(&galon).unwrap().unwrap().title, "from pc", "later HLC wins");

    // A write made after seeing the other side wins even with a slower wall clock.
    phone.edit_task(&galon, TaskEdit::Title("phone again".into()), T0 + 150).unwrap();
    pc.apply_remote(&phone.outbox().unwrap()).unwrap();
    assert_eq!(pc.task(&galon).unwrap().unwrap().title, "phone again");
}

#[test]
fn delete_versus_edit_converges_to_deleted() {
    for pc_first in [true, false] {
        let (mut pc, mut phone, _, belanja, galon, gula) = family();
        // The PC deletes Galon while the phone edits its notes; the phone also edits Gula.
        pc.delete(&EntityRef::Task(galon.clone()), T0 + 100).unwrap();
        phone.edit_task(&galon, TaskEdit::Notes("Aqua 19 L".into()), T0 + 101).unwrap();
        phone.edit_task(&gula, TaskEdit::Title("Gula 2 kg".into()), T0 + 101).unwrap();

        let from_pc = pc.outbox().unwrap();
        let from_phone = phone.outbox().unwrap();
        if pc_first {
            phone.apply_remote(&from_pc).unwrap();
            pc.apply_remote(&from_phone).unwrap();
        } else {
            pc.apply_remote(&from_phone.iter().rev().cloned().collect::<Vec<_>>()).unwrap();
            phone.apply_remote(&from_pc).unwrap();
        }

        let snap = pc.snapshot().unwrap();
        assert_eq!(snap, phone.snapshot().unwrap());
        assert!(!snap.tasks.contains_key(&galon), "a delete is not undone by a concurrent edit");
        assert_eq!(snap.tasks[&gula].title, "Gula 2 kg");
        assert_eq!(pc.tasks_of_list(&belanja).unwrap().todo.len(), 1);
    }
}

#[test]
fn a_third_device_converges_from_any_delivery_order() {
    let (mut pc, mut phone, setup, _, galon, gula) = family();
    pc.edit_task(&galon, TaskEdit::Title("Beli galon 2".into()), T0 + 100).unwrap();
    phone.set_done(&galon, true, T0 + 100).unwrap();
    pc.delete(&EntityRef::Task(gula.clone()), T0 + 100).unwrap();
    phone.edit_task(&gula, TaskEdit::Notes("x".into()), T0 + 100).unwrap();
    pc.apply_remote(&phone.outbox().unwrap()).unwrap();
    phone.apply_remote(&pc.outbox().unwrap()).unwrap();
    assert_eq!(pc.snapshot().unwrap(), phone.snapshot().unwrap());

    // Everything ever written, as the relay saw it.
    let history: Vec<Change> = [setup, pc.outbox().unwrap(), phone.outbox().unwrap()].concat();
    let orders = [history.iter().rev().cloned().collect::<Vec<_>>(), shuffle(history.clone())];
    for (i, order) in orders.into_iter().enumerate() {
        // Task fields arrive before the space, folder and list they belong to.
        let mut tablet = store(&format!("tablet{i}"), "ayah");
        // Half now and half later, so the tablet also sits in a partial state in between.
        let (a, b) = order.split_at(order.len() / 2);
        tablet.apply_remote(a).unwrap();
        tablet.apply_remote(b).unwrap();
        assert_eq!(tablet.snapshot().unwrap(), pc.snapshot().unwrap());
    }
}
#[test]
fn duplicate_delivery_is_a_no_op() {
    let (mut pc, mut phone, _, _, galon, _) = family();
    pc.edit_task(&galon, TaskEdit::Title("Beli galon 2".into()), T0 + 100).unwrap();
    let batch = pc.outbox().unwrap();
    assert_eq!(phone.apply_remote(&batch).unwrap(), 1);
    let after_once = phone.snapshot().unwrap();

    // Redelivery, and redelivery after a newer local write: nothing moves.
    assert_eq!(phone.apply_remote(&batch).unwrap(), 0);
    phone.edit_task(&galon, TaskEdit::Title("phone".into()), T0 + 200).unwrap();
    let after_edit = phone.snapshot().unwrap();
    assert_eq!(phone.apply_remote(&[batch.clone(), batch].concat()).unwrap(), 0);
    assert_eq!(phone.snapshot().unwrap(), after_edit);
    assert_ne!(after_once, after_edit);

    // An echo of the phone's own changes from the server is also ignored.
    assert_eq!(phone.apply_remote(&phone.outbox().unwrap()).unwrap(), 0);
    assert_eq!(phone.outbox().unwrap().len(), 1);
}

#[test]
fn changes_for_a_purged_entity_are_ignored() {
    let (mut pc, mut phone, _, belanja, galon, _) = family();
    phone.edit_task(&galon, TaskEdit::Notes("late".into()), T0 + 100).unwrap();
    let late = phone.outbox().unwrap();

    pc.delete(&EntityRef::Task(galon.clone()), T0 + 100).unwrap();
    let sent: Vec<ChangeId> = pc.outbox().unwrap().into_iter().map(|c| c.change_id).collect();
    pc.ack(&sent).unwrap();
    assert_eq!(pc.purge_tombstones(T0 + 100 + TOMBSTONE_TTL_MS).unwrap(), 1);

    assert_eq!(pc.apply_remote(&late).unwrap(), 0);
    assert!(pc.task(&galon).unwrap().is_none());
    assert_eq!(pc.tasks_of_list(&belanja).unwrap().todo.len(), 1);
}
