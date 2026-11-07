use app_habits::drink::*;
use app_habits::{mins, Date, Moment};

fn day() -> Date {
    Date::new(2026, 11, 6).unwrap()
}

fn at(h: u32, m: u32) -> Moment {
    day().at(h, m)
}

/// Kota Yogyakarta, 6 Nov 2026 (app-core): Imsak 03:51, Subuh 04:01, Dzuhur 11:32,
/// Ashar 14:46, Maghrib 17:44, Isya 18:57.
fn prayers() -> PrayerTimes {
    PrayerTimes { imsak: 231, subuh: 241, dzuhur: 692, ashar: 886, maghrib: 1064, isya: 1137 }
}

fn ctx() -> ReminderContext {
    ReminderContext {
        presence: Presence::Active,
        focus_until: None,
        meeting_recording: false,
        prayers: prayers(),
        fasting: false,
    }
}

fn fasting() -> ReminderContext {
    ReminderContext { fasting: true, ..ctx() }
}

fn drink() -> Drink {
    Drink::new(DrinkConfig::default())
}

/// Just the due time, for tests about the clock rather than the pace.
fn due(d: &mut Drink, now: Moment, ctx: &ReminderContext) -> Option<Moment> {
    d.next_reminder(now, ctx).map(|r| r.at)
}

fn interval(at: Moment) -> Reminder {
    Reminder { at, kind: ReminderKind::Interval, quiet: false, at_break: false }
}

#[test]
fn defaults_match_the_spec() {
    let c = DrinkConfig::default();
    assert_eq!(c.goal_ml, 2000);
    assert_eq!(c.glasses_ml, vec![150, 250, 500]);
    assert_eq!(c.default_glass_ml, 250);
    assert_eq!(c.active_hours, ActiveHours { start_min: 480, end_min: 1260 });
    assert_eq!(c.interval, mins(60));
    assert_eq!(c.catch_up, Some(mins(45)));
    assert_eq!(c.after_prayer, None);
    assert_eq!(c.sahur_lead, mins(30));
    assert_eq!(c.idle_after, mins(5));
    assert_eq!(suggest_goal_ml(60), (1800, 2100));
}

#[test]
fn log_totals_per_day_and_undo_removes_the_latest() {
    let mut d = drink();
    d.log(250, at(9, 0), Source::Local).unwrap();
    d.log(500, at(11, 0), Source::Device("phone".into())).unwrap();
    d.log(150, day().prev().at(20, 0), Source::Local).unwrap();
    assert_eq!(d.total_for(day()), 750);
    assert_eq!(d.total_for(day().prev()), 150);
    assert_eq!(d.log(0, at(12, 0), Source::Local), Err(DrinkError::ZeroAmount));

    let undone = d.undo_last().unwrap();
    assert_eq!(undone.ml, 500);
    assert_eq!(d.total_for(day()), 250);
    d.undo_last();
    d.undo_last();
    assert_eq!(d.undo_last(), None);
}

#[test]
fn pace_is_linear_over_active_hours() {
    let d = drink();
    assert_eq!(d.pace_target(at(7, 0)), 0);
    assert_eq!(d.pace_target(at(8, 0)), 0);
    // 6 of 13 hours: 2000 * 6 / 13 = 923.
    assert_eq!(d.pace_target(at(14, 0)), 923);
    assert_eq!(d.pace_target(at(21, 0)), 2000);
    assert_eq!(d.pace_target(at(23, 0)), 2000);
}

#[test]
fn spec_example_600_by_1400_against_1150_shortens_the_interval_to_45() {
    let c = DrinkConfig::default();
    assert_eq!(c.interval_for(600, 1150), mins(45));
    assert_eq!(c.interval_for(1150, 1150), mins(60));
    assert_eq!(DrinkConfig { catch_up: None, ..c }.interval_for(600, 1150), mins(60));

    // Goal 2300 over 08:00 to 20:00 puts the 14:00 target at exactly 1150 ml.
    let mut d = Drink::new(DrinkConfig {
        goal_ml: 2300,
        active_hours: ActiveHours { start_min: 480, end_min: 1200 },
        ..DrinkConfig::default()
    });
    d.log(250, at(9, 0), Source::Local).unwrap();
    d.log(250, at(11, 0), Source::Local).unwrap();
    d.log(100, at(14, 0), Source::Local).unwrap();
    assert_eq!(d.pace_target(at(14, 0)), 1150);
    assert_eq!(d.total_for(day()), 600);
    assert!(d.is_behind(at(14, 0)));
    let r = d.next_reminder(at(14, 0), &ctx()).unwrap();
    assert_eq!(r, Reminder { at: at(14, 45), kind: ReminderKind::CatchUp, quiet: false, at_break: false });
}

#[test]
fn on_pace_or_ahead_keeps_the_normal_interval_and_ahead_is_quiet() {
    let mut d = drink();
    d.log(1000, at(10, 0), Source::Local).unwrap();
    assert!(!d.is_behind(at(10, 0)));
    let r = d.next_reminder(at(10, 0), &ctx()).unwrap();
    assert_eq!(r, Reminder { quiet: true, ..interval(at(11, 0)) });
}

#[test]
fn the_first_reminder_comes_an_interval_after_active_hours_open() {
    let mut d = drink();
    d.log(250, day().prev().at(20, 30), Source::Local).unwrap();
    assert_eq!(d.next_reminder(at(7, 0), &ctx()), Some(interval(at(9, 0))));
}

#[test]
fn nothing_is_due_after_active_hours_end() {
    let mut d = drink();
    d.log(2000, at(20, 15), Source::Local).unwrap();
    assert_eq!(d.next_reminder(at(20, 15), &ctx()), None);
}

#[test]
fn an_overdue_reminder_never_moves_past_the_end_of_active_hours() {
    let mut d = drink();
    d.log(250, at(19, 30), Source::Local).unwrap();
    assert!(d.next_reminder(at(20, 0), &ctx()).is_some_and(|r| r.at < at(21, 0)));
    // The PC slept through the due time and woke after 21:00.
    assert_eq!(d.next_reminder(at(21, 10), &ctx()), None);
    // A break after 21:00 does not carry it either.
    let late_break = ReminderContext { focus_until: Some(at(21, 5)), ..ctx() };
    assert_eq!(d.next_reminder(at(20, 50), &late_break), None);
}

#[test]
fn a_launch_starts_the_clock_and_never_reminds_outside_active_hours() {
    // Nothing logged: without a start the clock would count from the epoch.
    let mut d = drink();
    d.start_watching(at(22, 38));
    assert_eq!(d.next_reminder(at(22, 38), &ctx()), None);

    // A daytime launch waits a full interval (catch-up, being behind) instead of firing at once.
    let mut d = drink();
    d.start_watching(at(14, 0));
    let r = d.next_reminder(at(14, 0), &ctx()).unwrap();
    assert_eq!((r.at, r.kind), (at(14, 45), ReminderKind::CatchUp));

    // An early launch still waits for active hours to open.
    let mut d = drink();
    d.start_watching(at(6, 30));
    assert_eq!(due(&mut d, at(6, 30), &ctx()), Some(at(9, 0)));
}

#[test]
fn showing_a_reminder_restarts_the_clock_and_snooze_brings_it_back_in_15() {
    let mut d = drink();
    d.log(250, at(9, 0), Source::Local).unwrap();
    d.log(500, at(10, 0), Source::Local).unwrap();
    assert_eq!(d.next_reminder(at(11, 0), &ctx()).unwrap().at, at(11, 0));
    d.snooze(at(11, 0));
    assert_eq!(d.next_reminder(at(11, 1), &ctx()).unwrap().at, at(11, 15));
    d.reminder_shown(at(11, 15));
    assert_eq!(d.next_reminder(at(11, 16), &ctx()).unwrap().at, at(12, 15));
}

#[test]
fn idle_over_five_minutes_pauses_the_clock() {
    let mut d = drink();
    d.log(500, at(9, 0), Source::Local).unwrap();
    // Away from 09:20; noticed at 09:40.
    let away = ReminderContext { presence: Presence::Idle { since: at(9, 20) }, ..ctx() };
    assert_eq!(d.next_reminder(at(9, 40), &away), None);
    // Back at 10:00 with 20 active minutes banked: 40 more to go.
    assert_eq!(due(&mut d, at(10, 0), &ctx()), Some(at(10, 40)));
}

#[test]
fn a_short_idle_does_not_pause_the_clock() {
    let mut d = drink();
    d.log(500, at(9, 0), Source::Local).unwrap();
    let reading = ReminderContext { presence: Presence::Idle { since: at(9, 20) }, ..ctx() };
    assert_eq!(due(&mut d, at(9, 24), &reading), Some(at(10, 0)));
}

#[test]
fn a_screen_lock_resets_the_clock() {
    let mut d = drink();
    d.log(500, at(9, 0), Source::Local).unwrap();
    let locked = ReminderContext { presence: Presence::Locked, ..ctx() };
    assert_eq!(d.next_reminder(at(9, 50), &locked), None);
    assert_eq!(due(&mut d, at(10, 30), &ctx()), Some(at(11, 30)));
}

#[test]
fn a_reminder_during_focus_waits_for_the_break() {
    let mut d = drink();
    d.log(500, at(9, 0), Source::Local).unwrap();
    let focus = ReminderContext { focus_until: Some(at(10, 20)), ..ctx() };
    let r = d.next_reminder(at(9, 55), &focus).unwrap();
    assert_eq!((r.at, r.at_break), (at(10, 20), true));
    let early_break = ReminderContext { focus_until: Some(at(9, 30)), ..ctx() };
    let r = d.next_reminder(at(9, 0), &early_break).unwrap();
    assert_eq!((r.at, r.at_break), (at(10, 0), false));
}

#[test]
fn a_recorded_meeting_holds_reminders_until_it_ends() {
    let mut d = drink();
    d.log(500, at(9, 0), Source::Local).unwrap();
    let meeting = ReminderContext { meeting_recording: true, ..ctx() };
    assert_eq!(d.next_reminder(at(10, 5), &meeting), None);
    assert_eq!(due(&mut d, at(10, 30), &ctx()), Some(at(10, 30)));
}

#[test]
fn after_prayer_reminder_comes_ten_minutes_after_each_prayer() {
    let mut d = Drink::new(DrinkConfig { after_prayer: Some(mins(10)), ..DrinkConfig::default() });
    d.log(600, at(11, 30), Source::Local).unwrap();
    let r = d.next_reminder(at(11, 31), &ctx()).unwrap();
    assert_eq!(
        r,
        Reminder { at: at(11, 42), kind: ReminderKind::AfterPrayer(Prayer::Dzuhur), quiet: false, at_break: false }
    );
    d.reminder_shown(at(11, 42));
    // Shown once; the clock restarted at 11:42.
    assert_eq!(d.next_reminder(at(11, 43), &ctx()), Some(Reminder { quiet: true, ..interval(at(12, 42)) }));
}

#[test]
fn after_prayer_is_off_by_default_and_skips_prayers_outside_active_hours() {
    let mut d = drink();
    d.log(1000, at(11, 30), Source::Local).unwrap();
    assert_eq!(d.next_reminder(at(11, 31), &ctx()).unwrap().kind, ReminderKind::Interval);

    // Subuh 04:01 + 10 is before 08:00, so the first one is the interval at 09:00.
    let mut d = Drink::new(DrinkConfig { after_prayer: Some(mins(10)), ..DrinkConfig::default() });
    assert_eq!(d.next_reminder(Date::new(2026, 11, 6).unwrap().at(4, 5), &ctx()), Some(interval(at(9, 0))));
}

#[test]
fn puasa_has_no_reminders_during_the_fast_then_buka_at_maghrib() {
    let mut d = drink();
    d.log(500, at(3, 30), Source::Local).unwrap();
    let r = d.next_reminder(at(10, 0), &fasting()).unwrap();
    assert_eq!(r, Reminder { at: at(17, 44), kind: ReminderKind::Buka, quiet: false, at_break: false });
    d.reminder_shown(at(17, 44));
    d.log(250, at(17, 44), Source::Local).unwrap();
    // The night window spaces the rest on the normal clock.
    assert_eq!(d.next_reminder(at(17, 45), &fasting()).unwrap().at, at(18, 44));
}

#[test]
fn puasa_sends_the_sahur_pill_thirty_minutes_before_imsak() {
    let mut d = drink();
    d.log(250, at(2, 50), Source::Local).unwrap();
    let r = d.next_reminder(at(2, 55), &fasting()).unwrap();
    assert_eq!(r, Reminder { at: at(3, 21), kind: ReminderKind::Sahur { imsak: 231 }, quiet: false, at_break: false });

    // Late in the evening the sahur pill is the one for tomorrow's Imsak.
    let mut d = drink();
    d.log(500, at(23, 0), Source::Local).unwrap();
    d.reminder_shown(at(23, 0));
    let locked_overnight = ReminderContext { presence: Presence::Locked, ..fasting() };
    assert_eq!(d.next_reminder(at(23, 30), &locked_overnight), None);
    let r = d.next_reminder(day().next().at(3, 0), &fasting()).unwrap();
    assert_eq!(r.kind, ReminderKind::Sahur { imsak: 231 });
    assert_eq!(r.at, day().next().at(3, 21));
}

#[test]
fn puasa_paces_the_goal_over_the_night() {
    let mut d = drink();
    let mode = fasting().mode();
    assert_eq!(mode, DayMode::Fasting { imsak: 231, maghrib: 1064 });
    assert_eq!(d.pace_target_in(at(12, 0), mode), 0);
    assert!(!d.is_behind_in(at(12, 0), mode));
    // Night window 17:44 to 03:51 is 607 min; at 22:47 (303 min in) the target is just under half.
    assert_eq!(d.pace_target_in(at(22, 47), mode), 998);
    d.log(500, at(18, 0), Source::Local).unwrap();
    d.log(300, day().next().at(1, 0), Source::Local).unwrap();
    assert_eq!(d.consumed_in(day().next().at(1, 30), mode), 800);
    assert!(d.is_behind_in(at(22, 47), mode));
}
