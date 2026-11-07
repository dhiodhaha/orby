//! ICS text to a [`Calendar`]. Lenient: a broken line or an event without DTSTART is skipped,
//! never an error, because one bad event in a published feed must not hide the rest.

use crate::link::find_meeting_link;
use crate::model::{parse_date_time, Calendar, DateTimeValue, Event, EventEnd, Status};
use crate::rrule::RRule;
use crate::time::{parse_duration, LocalDateTime};
use crate::tz::{parse_utc_offset, Observance, VTimezone};

pub fn parse(text: &str) -> Calendar {
    let mut cal = Calendar::default();
    let mut stack: Vec<Frame> = Vec::new();
    for line in unfold(text.trim_start_matches('\u{feff}')) {
        let Some(prop) = Property::parse(&line) else { continue };
        match prop.name.as_str() {
            "BEGIN" => stack.push(Frame::open(&prop.value)),
            "END" => {
                if let Some(frame) = stack.pop() {
                    frame.close(stack.last_mut(), &mut cal);
                }
            }
            _ => match stack.last_mut() {
                Some(Frame::Calendar) if prop.name == "X-WR-CALNAME" => cal.name = Some(unescape(&prop.value)),
                Some(Frame::Event(draft)) => draft.apply(&prop),
                Some(Frame::Timezone(tz)) if prop.name == "TZID" => tz.tzid = prop.value.trim().to_string(),
                Some(Frame::Observance(obs)) => obs.apply(&prop),
                _ => {}
            },
        }
    }
    cal
}

/// Joins folded lines: a line starting with a space or tab continues the previous one.
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let (Some(rest), Some(last)) = (line.strip_prefix([' ', '\t']), out.last_mut()) {
            last.push_str(rest);
        } else if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// TEXT value escapes: `\n` `\N` `\,` `\;` `\\`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// One content line: NAME;PARAM=VALUE:value. Parameter values may be quoted and hold ':' or ';'.
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn parse(line: &str) -> Option<Property> {
        let mut in_quotes = false;
        let mut cuts = Vec::new();
        let mut colon = None;
        for (i, c) in line.char_indices() {
            match c {
                '"' => in_quotes = !in_quotes,
                ';' if !in_quotes => cuts.push(i),
                ':' if !in_quotes => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let colon = colon?;
        let head = &line[..colon];
        let mut segments = Vec::with_capacity(cuts.len() + 1);
        let mut from = 0;
        for cut in cuts {
            segments.push(&head[from..cut]);
            from = cut + 1;
        }
        segments.push(&head[from..]);
        let name = segments[0].trim().to_ascii_uppercase();
        if name.is_empty() {
            return None;
        }
        let params = segments[1..]
            .iter()
            .filter_map(|s| s.split_once('='))
            .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
            .collect();
        Some(Property { name, params, value: line[colon + 1..].to_string() })
    }

    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn is_date(&self) -> bool {
        self.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
    }

    fn date_time(&self) -> Option<DateTimeValue> {
        parse_date_time(&self.value, self.param("TZID"), self.is_date())
    }

    /// EXDATE may list several values on one line.
    fn date_times(&self) -> Vec<DateTimeValue> {
        self.value.split(',').filter_map(|v| parse_date_time(v, self.param("TZID"), self.is_date())).collect()
    }
}

enum Frame {
    Calendar,
    Event(Box<EventDraft>),
    Timezone(VTimezone),
    Observance(Box<ObservanceDraft>),
    /// VALARM, VTODO and anything else: its properties are ignored.
    Other,
}

impl Frame {
    fn open(name: &str) -> Frame {
        match name.trim().to_ascii_uppercase().as_str() {
            "VCALENDAR" => Frame::Calendar,
            "VEVENT" => Frame::Event(Box::default()),
            "VTIMEZONE" => Frame::Timezone(VTimezone { tzid: String::new(), observances: Vec::new() }),
            "STANDARD" | "DAYLIGHT" => Frame::Observance(Box::default()),
            _ => Frame::Other,
        }
    }

    fn close(self, parent: Option<&mut Frame>, cal: &mut Calendar) {
        match self {
            Frame::Event(draft) => cal.events.extend(draft.build()),
            Frame::Timezone(tz) if !tz.tzid.is_empty() => cal.timezones.push(tz),
            Frame::Observance(obs) => {
                if let (Some(Frame::Timezone(tz)), Some(o)) = (parent, obs.build()) {
                    tz.observances.push(o);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct EventDraft {
    uid: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    url: Option<String>,
    /// X-GOOGLE-CONFERENCE and X-MICROSOFT-SKYPETEAMSMEETINGURL: where exports put the join link.
    conference: Vec<String>,
    status: Status,
    start: Option<DateTimeValue>,
    end: Option<DateTimeValue>,
    duration: Option<i64>,
    rrule: Option<RRule>,
    exdates: Vec<DateTimeValue>,
    recurrence_id: Option<DateTimeValue>,
}

impl EventDraft {
    fn apply(&mut self, p: &Property) {
        let text = || Some(unescape(&p.value)).filter(|s| !s.trim().is_empty());
        match p.name.as_str() {
            "UID" => self.uid = Some(p.value.trim().to_string()),
            "SUMMARY" => self.summary = text(),
            "DESCRIPTION" => self.description = text(),
            "LOCATION" => self.location = text(),
            "URL" => self.url = Some(p.value.trim().to_string()),
            "X-GOOGLE-CONFERENCE" | "X-MICROSOFT-SKYPETEAMSMEETINGURL" => {
                self.conference.push(p.value.trim().to_string())
            }
            "STATUS" => self.status = Status::parse(&p.value),
            "DTSTART" => self.start = p.date_time(),
            "DTEND" => self.end = p.date_time(),
            "DURATION" => self.duration = parse_duration(&p.value),
            "RRULE" => self.rrule = RRule::parse(&p.value),
            "EXDATE" => self.exdates.extend(p.date_times()),
            "RECURRENCE-ID" => self.recurrence_id = p.date_time(),
            _ => {}
        }
    }

    fn build(self) -> Option<Event> {
        let start = self.start?;
        let end = match (self.end, self.duration) {
            (Some(e), _) => EventEnd::At(e),
            (None, Some(secs)) => EventEnd::After(secs),
            (None, None) => EventEnd::Unspecified,
        };
        let sources = self.url.iter().chain(&self.conference).chain(&self.location).chain(&self.description);
        let link = find_meeting_link(sources.map(String::as_str));
        Some(Event {
            uid: self.uid.unwrap_or_default(),
            summary: self.summary.unwrap_or_default(),
            description: self.description,
            location: self.location,
            url: self.url,
            link,
            status: self.status,
            start,
            end,
            rrule: self.rrule,
            exdates: self.exdates,
            recurrence_id: self.recurrence_id,
        })
    }
}

#[derive(Default)]
struct ObservanceDraft {
    onset: Option<LocalDateTime>,
    offset_from: Option<i32>,
    offset_to: Option<i32>,
    rule: Option<RRule>,
}

impl ObservanceDraft {
    fn apply(&mut self, p: &Property) {
        match p.name.as_str() {
            "DTSTART" => self.onset = p.date_time().map(|v| v.wall()),
            "TZOFFSETFROM" => self.offset_from = parse_utc_offset(&p.value),
            "TZOFFSETTO" => self.offset_to = parse_utc_offset(&p.value),
            "RRULE" => self.rule = RRule::parse(&p.value),
            _ => {}
        }
    }

    fn build(self) -> Option<Observance> {
        let offset_to = self.offset_to?;
        Some(Observance {
            onset: self.onset?,
            offset_from_minutes: self.offset_from.unwrap_or(offset_to),
            offset_to_minutes: offset_to,
            rule: self.rule,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_crlf_and_lf_with_space_or_tab() {
        let text = "SUMMARY:Long\r\n  title\r\nDESCRIPTION:a\n\tb\nX:1\r\n";
        assert_eq!(unfold(text), vec!["SUMMARY:Long title", "DESCRIPTION:ab", "X:1"]);
    }

    #[test]
    fn unescapes_text() {
        assert_eq!(unescape(r"Review\, notes\; plan\nline two\\done\N"), "Review, notes; plan\nline two\\done\n");
    }

    #[test]
    fn content_line_with_quoted_params() {
        let p = Property::parse(r#"ATTENDEE;CN="Doe; Jane: PM";ROLE=REQ-PARTICIPANT:mailto:jane@example.com"#).unwrap();
        assert_eq!(p.name, "ATTENDEE");
        assert_eq!(p.param("CN"), Some("Doe; Jane: PM"));
        assert_eq!(p.value, "mailto:jane@example.com");
        let p = Property::parse(r#"dtstart;tzid="Asia/Jakarta":20261106T103000"#).unwrap();
        assert_eq!(p.name, "DTSTART");
        assert_eq!(p.param("TZID"), Some("Asia/Jakarta"));
        assert!(Property::parse("no colon here").is_none());
    }

    #[test]
    fn alarm_properties_do_not_leak_into_the_event() {
        let cal = parse(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20261106T030000Z\nSUMMARY:Real\n\
             BEGIN:VALARM\nDESCRIPTION:Reminder\nSUMMARY:Alarm\nEND:VALARM\nEND:VEVENT\nEND:VCALENDAR\n",
        );
        assert_eq!(cal.events.len(), 1);
        assert_eq!(cal.events[0].summary, "Real");
        assert_eq!(cal.events[0].description, None);
    }

    #[test]
    fn event_without_dtstart_is_skipped() {
        let cal = parse("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nSUMMARY:No time\nEND:VEVENT\nEND:VCALENDAR\n");
        assert!(cal.events.is_empty());
    }
}
