# app-habits

OS-free habit engines for the orb: the focus timer (ORB_SPEC 6.4, DESIGN.md "Focus (v6.2)"), the held-notices gate, and drink pacing (ORB_SPEC 6.5, DESIGN.md "Drink (v6.2)"). No dependencies, no I/O, no UI, and no system clock: every time-dependent call takes `now`.

## Time (`time`)
- `Moment`: local wall-clock seconds since 1970-01-01 00:00. Build one with `Moment::from_unix(unix_secs, utc_offset_secs)` at the app's top level, or `Date::at(h, m)` / `Date::at_minute(m)`.
- `Date::new(y, m, d) -> Option<Date>`, `date.next()`, `date.prev()`, `moment.date()`, `moment.minute_of_day()`, `moment.since(earlier) -> Duration`, `mins(n) -> Duration`.

## Focus (`focus`)
- `FocusConfig` (default 25 min focus, 5 min short break, 15 min long break after every 4 sessions, `auto_start_next: false`).
- `Phase`: `Idle`, `Focus { session, started_at, length, task }`, `Paused { session, length, remaining, task, reason: User | Prayer(name) }`, `Break { kind: Short | Long, started_at, length }`. `session` is `Session { n, of }`.
- `FocusTimer::new(config)`, then:
  - `start(now, length: Option<Duration>, task: Option<TaskId>)` from Idle, or from a break to end it early.
  - `pause(now)`, `resume(now)`, `stop(now)`, `skip_to_break(now)`. Commands that do not fit the phase return `Err(FocusError::NotAllowed(kind))`.
  - `pause_for_prayer(now, name)`, `resume_after_prayer(now)`: no-ops outside focus. The second only resumes a prayer pause, never a user pause.
  - `tick(now) -> Outcome { transitions, events }`, with events `FocusDone { session, minutes }`, `BreakDone { kind }`, `SessionLogged(SessionRecord { date, ended_at, minutes, task })`. A late tick catches up through every phase that ended and stamps each at the moment it was due.
  - Queries: `remaining(now)`, `progress(now)` (0 to 1 for the ring), `break_at(now)` (for drink), `is_holding()`, `sessions_done()`, `log()` (History). A prayer pause holds nothing: `is_holding()` is false and `break_at` is `None`, since the user is away praying with no end in sight.

## Held notices (`held`)
`HeldNotices<T>`: `offer(notice, Priority, holding) -> Gate::Show(T) | Held | Dropped`, `count()`, `retain(keep)` (drop notices that went stale while waiting), `release()`. During focus, urgent passes, normal is held, and quiet is dropped. Pass `timer.is_holding()` and call `release()` when the timer leaves focus.

## Drink (`drink`)
- `DrinkConfig` (default goal 2000 ml, glasses 150/250/500 plus `custom_glass`, default glass 250, active hours 08:00 to 21:00, interval 60 min, catch-up 45 min, after-prayer off (`Some(10 min)` turns it on), sahur lead 30 min, idle 5 min, snooze 15 min). `config.interval_for(consumed, target)`. `suggest_goal_ml(kg)` gives 30 to 35 ml per kg.
- `Drink::new(config)`, then `log(ml, at, Source::Local | Device(name))`, `undo_last()`, `total_for(date)`, `pace_target(now)` and `is_behind(now)` (linear over active hours), `pace_target_in`, `is_behind_in` and `consumed_in` with a `DayMode` (`Normal` or `Fasting { imsak, maghrib }`).
- `next_reminder(&mut self, now, &ReminderContext) -> Option<Reminder { at, kind, quiet, at_break }>`. The context carries `presence` (`Active`, `Idle { since }`, `Locked`), `focus_until`, `meeting_recording`, `prayers: PrayerTimes` (minutes after midnight, as app-core gives them) and `fasting`. Kinds: `Interval`, `CatchUp`, `AfterPrayer(prayer)`, `Buka`, `Sahur { imsak }`. Then `reminder_shown(at)` or `snooze(now)` once the pill has shown. Call `start_watching(now)` at launch: time before it counts as away, so the clock runs from the launch.

How the reminder clock works: it counts active time since the last drink, shown reminder or unlock. Idle over 5 min pauses it from the last input; a lock resets it. While away, or while a recorded meeting runs, `next_reminder` returns `None`. A reminder due during focus moves to the break (`at_break`). A reminder is never due at or after the end of the window, even when it is overdue or waits for a late break. In puasa mode nothing is due between Imsak and Maghrib. Buka comes at Maghrib and sahur 30 min before Imsak, and the goal is paced over the night.

## Verified
```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\habits\target"; Set-Location C:\Users\dopamine\dev\clippy\crates\habits; cargo test; cargo clippy --all-targets -- -D warnings
```
43 tests pass (4 unit, 19 focus, 20 drink) and clippy is clean. The tests include the spec example: 600 ml by 14:00 against a 1150 ml target makes the interval 45 min, both through `interval_for` and through `next_reminder`.
