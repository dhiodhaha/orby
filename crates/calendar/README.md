# app-calendar

The calendar engine for the orb (ORB_SPEC §6.3, docs/design/calendar.md). It parses ICS text from
Google and Outlook exports, expands recurring events into concrete occurrences in a time window,
and merges the day's events, tasks due and prayer times into the Today panel's one timeline.

It is pure logic and has no dependencies. Nothing reads the system clock or the network: the caller
passes the window, the viewer's UTC offset and the ICS text in. Fetching feeds, Google sign-in and
polling every 10 minutes belong to the app.

## Public API

```rust
pub fn parse(text: &str) -> Calendar;
pub fn occurrences(cal: &Calendar, from: Instant, to: Instant, display_offset_minutes: i32) -> Vec<Occurrence>;
pub fn today_timeline(
    occurrences: impl IntoIterator<Item = Occurrence>,
    tasks_due: impl IntoIterator<Item = TaskDue>,
    prayer_times: impl IntoIterator<Item = PrayerTime>,
) -> Vec<Item>;
```

| Type | What it is |
|---|---|
| `Calendar { name, events, timezones }` | A parsed file. `name` is X-WR-CALNAME. `zone_offset_minutes(tzid, wall)` resolves a zone. |
| `Event` | One VEVENT: uid, summary, description, location, url, `link`, `status`, `start`, `end`, `rrule`, `exdates`, `recurrence_id`. |
| `DateTimeValue` | `Date` (all-day), `Utc`, `Floating` or `Zoned { wall, tzid }`. |
| `EventEnd` | `At(DateTimeValue)` (DTEND), `After(seconds)` (DURATION) or `Unspecified`. |
| `RRule { freq, interval, end, by_day, by_month_day, by_month }` | `Freq` is Daily, Weekly, Monthly or Yearly. `RecurEnd` is Never, Count or Until. `WeekdayNum` is `MO` or `1MO` / `-1FR`. |
| `VTimezone`, `Observance` | STANDARD and DAYLIGHT blocks with their yearly rules. |
| `Occurrence { uid, title, start, end, all_day, link, location }` | One concrete appearance. `end` is exclusive. |
| `MeetingLink { kind, url }`, `LinkKind` | Meet, Zoom or Teams. `kind.label()` gives "Meet" for "Work · Meet". |
| `TaskDue { id, title, due, done }` | A task handed over by the tasks module. |
| `PrayerTime { name, at }` | A prayer time, already named for display ("Jumat" on Friday if that setting is on). |
| `Item` | `Event(Occurrence)`, `Task(TaskDue)` or `PrayerMarker(PrayerTime)`. `at()`, `is_checkable()`. |
| `Instant`, `LocalDateTime`, `Date`, `Weekday` | Seconds since the Unix epoch, wall time, calendar date. |

## Behaviour

- **Parsing** unfolds lines (CRLF or LF, space or tab), unescapes TEXT (`\n \, \; \\`), reads quoted
  parameters, ignores VALARM, and skips events with no usable DTSTART rather than failing.
- **STATUS:CANCELLED** masters show nothing. A cancelled override removes only its instance.
- **Overrides** (same UID plus RECURRENCE-ID) replace the instance they name, even when the new time
  is in the window and the original is not. **EXDATE** (lists, TZID or VALUE=DATE) removes instances,
  and they still count toward COUNT, as RFC 5545 says.
- **Recurrence** runs on wall time in the event's own zone, so a 09:00 New York meeting stays at 09:00
  across DST. DTSTART is always the first instance. UNTIL is inclusive.
- **Zones**: the file's VTIMEZONE first, then a fallback table (Asia/Jakarta +7, Asia/Makassar +8,
  Asia/Jayapura +9, Asia/Singapore +8, UTC, and Outlook's "SE Asia Standard Time" and "Singapore
  Standard Time"), then the display offset. All-day dates and floating times use the display offset.
- **Meeting links** are searched in URL, X-GOOGLE-CONFERENCE, X-MICROSOFT-SKYPETEAMSMEETINGURL,
  LOCATION, then DESCRIPTION. Recognised: `meet.google.com/<code>`, `*.zoom.us/j|s|w|my/...`,
  `teams.microsoft.com/l/meetup-join/...` and `/meet/...`, `teams.live.com/meet/...`.
- **Today timeline** sorts by time. At the same minute: all-day events, then the prayer marker, then
  timed events, then tasks. Only tasks are checkable. A prayer marker has a name and a time and no
  done state, so it can never be ticked (DESIGN.md, "Prayer is not a task").

## Verified

Fixtures in `tests/fixtures/` are written in Google export style (`google.ics`) and Outlook/Exchange
style (`outlook.ics`, Windows zone names, folded 75-column lines, VALARM blocks). The tests cover
DAILY with COUNT and with INTERVAL plus UNTIL, WEEKLY with BYDAY and biweekly, MONTHLY with 1MO, -1FR,
BYMONTHDAY and BYMONTHDAY=-1, YEARLY plain, with BYMONTHDAY and with 2TU, overrides (moved and
cancelled), EXDATE, all-day and multi-day events, New York to WIB conversion across the end of DST,
the fallback zones, floating times and the Today merge for the design's sample day (Fri 6 Nov 2026).

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\calendar\target"; Set-Location C:\Users\dopamine\dev\clippy\crates\calendar; cargo test; cargo clippy --all-targets -- -D warnings
```

Result: 67 tests pass (25 unit, 13 edge cases, 16 Google, 9 Outlook, 4 Today), clippy clean with
`-D warnings`, `cargo fmt --check` clean.

## Not supported (on purpose, for now)

BYSETPOS, BYWEEKNO, BYYEARDAY, BYHOUR and sub-daily FREQ (such a rule shows the event once), RDATE,
RECURRENCE-ID;RANGE=THISANDFUTURE, and WKST other than Monday. Times inside the hour of a DST
transition may resolve to either side.
