//! Calendar engine for the orb: ICS parsing, recurrence expansion and the merged Today timeline.
//!
//! Pure logic. Nothing here reads the system clock or the network: callers pass the window, the
//! display offset and the ICS text in. Fetching feeds and Google sign-in belong to the app.

mod ics;
mod link;
mod model;
mod occurrences;
mod rrule;
mod time;
mod today;
mod tz;

pub use ics::parse;
pub use link::{find_meeting_link, LinkKind, MeetingLink};
pub use model::{Calendar, DateTimeValue, Event, EventEnd, Status};
pub use occurrences::{occurrences, Occurrence};
pub use rrule::{Freq, RRule, RecurEnd, WeekdayNum};
pub use time::{days_in_month, is_leap_year, Date, Instant, LocalDateTime, Weekday};
pub use today::{today_timeline, Item, PrayerTime, TaskDue};
pub use tz::{fallback_offset_minutes, Observance, VTimezone};
