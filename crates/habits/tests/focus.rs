use app_habits::focus::*;
use app_habits::held::{Gate, HeldNotices, Priority};
use app_habits::{mins, Date, Moment};

fn at(h: u32, m: u32) -> Moment {
    Date::new(2026, 11, 6).unwrap().at(h, m)
}

fn task() -> Option<TaskId> {
    Some(TaskId("kirim-invoice".into()))
}

fn timer() -> FocusTimer {
    FocusTimer::new(FocusConfig::default())
}

fn auto_timer() -> FocusTimer {
    FocusTimer::new(FocusConfig { auto_start_next: true, ..FocusConfig::default() })
}

#[test]
fn defaults_match_the_spec() {
    let c = FocusConfig::default();
    assert_eq!(c.focus, mins(25));
    assert_eq!(c.short_break, mins(5));
    assert_eq!(c.long_break, mins(15));
    assert_eq!(c.sessions_per_long_break, 4);
    assert!(!c.auto_start_next);
}

#[test]
fn a_session_counts_down_and_fills_the_ring() {
    let mut t = timer();
    t.start(at(9, 0), None, task()).unwrap();
    assert_eq!(
        t.phase(),
        &Phase::Focus { session: Session { n: 1, of: 4 }, started_at: at(9, 0), length: mins(25), task: task() }
    );
    assert_eq!(t.remaining(at(9, 0)), Some(mins(25)));
    assert_eq!(t.progress(at(9, 0)), 0.0);
    assert_eq!(t.remaining(at(9, 10)), Some(mins(15)));
    assert!((t.progress(at(9, 10)) - 0.4).abs() < 1e-6);
    assert!(t.tick(at(9, 24)).events.is_empty());
}

#[test]
fn finishing_focus_logs_the_session_and_starts_a_short_break() {
    let mut t = timer();
    t.start(at(9, 0), None, task()).unwrap();
    let out = t.tick(at(9, 25));
    let rec = SessionRecord { date: Date::new(2026, 11, 6).unwrap(), ended_at: at(9, 25), minutes: 25, task: task() };
    assert_eq!(
        out.events,
        vec![
            FocusEvent::FocusDone { session: Session { n: 1, of: 4 }, minutes: 25 },
            FocusEvent::SessionLogged(rec.clone())
        ]
    );
    assert_eq!(
        out.transitions,
        vec![Transition { at: at(9, 25), from: PhaseKind::Focus, to: PhaseKind::Break(BreakKind::Short) }]
    );
    assert_eq!(t.phase(), &Phase::Break { kind: BreakKind::Short, started_at: at(9, 25), length: mins(5) });
    assert_eq!(t.log(), &[rec]);
    assert_eq!(t.sessions_done(), 1);
}

#[test]
fn break_end_waits_for_the_user_by_default() {
    let mut t = timer();
    t.start(at(9, 0), None, None).unwrap();
    t.tick(at(9, 25));
    let out = t.tick(at(9, 30));
    assert_eq!(out.events, vec![FocusEvent::BreakDone { kind: BreakKind::Short }]);
    assert_eq!(t.phase(), &Phase::Idle);
    t.start(at(9, 40), None, None).unwrap();
    assert!(matches!(t.phase(), Phase::Focus { session: Session { n: 2, of: 4 }, .. }));
}

#[test]
fn auto_start_begins_the_next_session_with_the_same_length_and_task() {
    let mut t = auto_timer();
    t.start(at(9, 0), Some(mins(50)), task()).unwrap();
    t.tick(at(9, 50));
    let out = t.tick(at(9, 55));
    assert_eq!(out.transitions.last().unwrap().to, PhaseKind::Focus);
    assert_eq!(
        t.phase(),
        &Phase::Focus { session: Session { n: 2, of: 4 }, started_at: at(9, 55), length: mins(50), task: task() }
    );
}

#[test]
fn every_fourth_session_earns_a_long_break_and_then_the_cycle_restarts() {
    let mut t = auto_timer();
    t.start(at(8, 0), None, None).unwrap();
    // 3 x (25 + 5) = 90 min, then the 4th focus ends at 09:55.
    let out = t.tick(at(9, 55));
    assert_eq!(t.log().len(), 4);
    assert_eq!(t.phase(), &Phase::Break { kind: BreakKind::Long, started_at: at(9, 55), length: mins(15) });
    assert_eq!(out.events.iter().filter(|e| matches!(e, FocusEvent::FocusDone { .. })).count(), 4);
    t.tick(at(10, 10));
    assert!(
        matches!(t.phase(), Phase::Focus { session: Session { n: 1, of: 4 }, started_at, .. } if *started_at == at(10, 10))
    );
}

#[test]
fn a_late_tick_stamps_each_transition_when_it_was_due() {
    let mut t = timer();
    t.start(at(9, 0), None, None).unwrap();
    let out = t.tick(at(11, 0));
    let times: Vec<_> = out.transitions.iter().map(|tr| tr.at).collect();
    assert_eq!(times, vec![at(9, 25), at(9, 30)]);
    assert_eq!(t.phase(), &Phase::Idle);
}

#[test]
fn pause_and_resume_keep_the_remaining_time() {
    let mut t = timer();
    t.start(at(9, 0), None, None).unwrap();
    t.pause(at(9, 10)).unwrap();
    assert_eq!(t.remaining(at(9, 30)), Some(mins(15)));
    assert!(t.tick(at(9, 40)).events.is_empty());
    t.resume(at(10, 0)).unwrap();
    assert_eq!(t.remaining(at(10, 0)), Some(mins(15)));
    assert!((t.progress(at(10, 0)) - 0.4).abs() < 1e-6);
    let out = t.tick(at(10, 15));
    assert!(out.events.contains(&FocusEvent::FocusDone { session: Session { n: 1, of: 4 }, minutes: 25 }));
}

#[test]
fn prayer_pauses_focus_and_its_dismissal_resumes_it() {
    let mut t = timer();
    t.start(at(11, 40), None, task()).unwrap();
    t.pause_for_prayer(at(11, 50), "Dzuhur");
    assert!(
        matches!(t.phase(), Phase::Paused { reason: PauseReason::Prayer(p), remaining, .. } if p == "Dzuhur" && *remaining == mins(15))
    );
    t.resume_after_prayer(at(12, 5));
    assert!(matches!(t.phase(), Phase::Focus { .. }));
    assert_eq!(t.remaining(at(12, 5)), Some(mins(15)));
}

#[test]
fn a_prayer_pause_holds_no_notices_and_no_drink_reminder() {
    let mut t = timer();
    t.start(at(11, 40), None, None).unwrap();
    t.pause_for_prayer(at(11, 50), "Dzuhur");
    assert!(!t.is_holding());
    assert_eq!(t.break_at(at(12, 30)), None);
    t.resume(at(12, 5)).unwrap();
    assert!(t.is_holding());
    t.pause(at(12, 10)).unwrap();
    assert!(t.is_holding(), "a pause the user made is still focus");
}

#[test]
fn dismissing_a_prayer_pill_never_resumes_a_user_pause() {
    let mut t = timer();
    t.start(at(11, 40), None, None).unwrap();
    t.pause(at(11, 45)).unwrap();
    assert_eq!(t.pause_for_prayer(at(11, 50), "Dzuhur"), Outcome::default());
    t.resume_after_prayer(at(12, 0));
    assert!(matches!(t.phase(), Phase::Paused { reason: PauseReason::User, .. }));
}

#[test]
fn prayer_outside_focus_changes_nothing() {
    let mut t = timer();
    assert_eq!(t.pause_for_prayer(at(11, 50), "Dzuhur"), Outcome::default());
    assert_eq!(t.phase(), &Phase::Idle);
}

#[test]
fn stop_ends_without_logging_and_restarts_the_cycle() {
    let mut t = timer();
    t.start(at(9, 0), None, None).unwrap();
    t.tick(at(9, 25));
    t.start(at(9, 30), None, None).unwrap();
    t.stop(at(9, 40)).unwrap();
    assert_eq!(t.phase(), &Phase::Idle);
    assert_eq!(t.log().len(), 1);
    t.start(at(10, 0), None, None).unwrap();
    assert!(matches!(t.phase(), Phase::Focus { session: Session { n: 1, .. }, .. }));
}

#[test]
fn skip_to_break_logs_the_minutes_focused_so_far() {
    let mut t = timer();
    t.start(at(9, 0), None, task()).unwrap();
    let out = t.skip_to_break(at(9, 18)).unwrap();
    assert!(matches!(&out.events[..], [FocusEvent::SessionLogged(r)] if r.minutes == 18 && r.task == task()));
    assert_eq!(t.phase(), &Phase::Break { kind: BreakKind::Short, started_at: at(9, 18), length: mins(5) });
}

#[test]
fn starting_during_a_break_ends_it_early() {
    let mut t = timer();
    t.start(at(9, 0), None, None).unwrap();
    t.tick(at(9, 25));
    let out = t.start(at(9, 27), None, None).unwrap();
    assert_eq!(out.transitions[0].from, PhaseKind::Break(BreakKind::Short));
    assert!(matches!(t.phase(), Phase::Focus { session: Session { n: 2, .. }, .. }));
}

#[test]
fn commands_that_do_not_fit_the_phase_are_refused() {
    let mut t = timer();
    assert_eq!(t.pause(at(9, 0)), Err(FocusError::NotAllowed(PhaseKind::Idle)));
    assert_eq!(t.resume(at(9, 0)), Err(FocusError::NotAllowed(PhaseKind::Idle)));
    assert_eq!(t.stop(at(9, 0)), Err(FocusError::NotAllowed(PhaseKind::Idle)));
    assert_eq!(t.skip_to_break(at(9, 0)), Err(FocusError::NotAllowed(PhaseKind::Idle)));
    assert_eq!(t.start(at(9, 0), Some(mins(0)), None), Err(FocusError::ZeroLength));
    t.start(at(9, 0), None, None).unwrap();
    assert_eq!(t.start(at(9, 1), None, None), Err(FocusError::NotAllowed(PhaseKind::Focus)));
    assert_eq!(t.resume(at(9, 1)), Err(FocusError::NotAllowed(PhaseKind::Focus)));
}

#[test]
fn break_at_tells_drink_when_the_break_comes() {
    let mut t = timer();
    assert_eq!(t.break_at(at(9, 0)), None);
    t.start(at(9, 0), None, None).unwrap();
    assert_eq!(t.break_at(at(9, 5)), Some(at(9, 25)));
    t.pause(at(9, 5)).unwrap();
    assert_eq!(t.break_at(at(9, 30)), Some(at(9, 50)));
}

#[test]
fn normal_notices_wait_for_the_break_and_urgent_ones_pass() {
    let mut t = timer();
    let mut held = HeldNotices::new();
    t.start(at(13, 0), None, None).unwrap();
    assert_eq!(held.offer("Kirim invoice due", Priority::Normal, t.is_holding()), Gate::Held);
    assert_eq!(held.offer("Minum dulu", Priority::Normal, t.is_holding()), Gate::Held);
    assert_eq!(
        held.offer("Memasuki waktu Ashar", Priority::Urgent, t.is_holding()),
        Gate::Show("Memasuki waktu Ashar")
    );
    assert_eq!(held.offer("Download this?", Priority::Quiet, t.is_holding()), Gate::Dropped);
    assert_eq!(held.count(), 2);

    t.tick(at(13, 25));
    assert!(!t.is_holding());
    assert_eq!(held.release(), vec!["Kirim invoice due", "Minum dulu"]);
    assert_eq!(held.count(), 0);
    assert_eq!(held.offer("Standup in 10 min", Priority::Normal, t.is_holding()), Gate::Show("Standup in 10 min"));
}

#[test]
fn held_notices_that_went_stale_can_be_dropped() {
    let mut held = HeldNotices::new();
    held.offer("10 menit lagi Subuh", Priority::Normal, true);
    held.offer("Minum dulu", Priority::Normal, true);
    held.retain(|n| !n.ends_with("Subuh"));
    assert_eq!(held.count(), 1);
    assert_eq!(held.release(), vec!["Minum dulu"]);
}
