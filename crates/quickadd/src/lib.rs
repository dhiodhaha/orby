//! Quick-add parser for tasks (ORB_SPEC 6.2). One typed line in Indonesian or English becomes a
//! title plus a due date and time, a repeat, a #list, a /folder and an @person, with the byte
//! range of every recognised part so the UI can draw token chips over it.
//!
//! Pure logic: the caller passes "now"; nothing here reads the system clock.

mod calendar;
mod lexicon;

pub use calendar::{days_in_month, Date, Now, Time, Weekday};

use calendar::{
    next_day_and_month, next_day_of_month, next_weekday_after, next_weekday_from, weekday_of_next_week,
};
use std::ops::Range;

/// Names the user already has, used to resolve `#list`, `/folder/sub` and `@person`.
/// Folder paths are written with `/` between levels, e.g. `"Work/Client A"`.
#[derive(Clone, Debug, Default)]
pub struct Known {
    pub lists: Vec<String>,
    pub folders: Vec<String>,
    pub people: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    pub date: Date,
    pub time: Option<Time>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekly(Weekday),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Date,
    Time,
    Repeat,
    List,
    Folder,
    Person,
}

/// One recognised part of the input. `range` is a byte range into the input and `text` is that
/// slice, so `&input[token.range] == token.text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickAdd {
    /// The input with every recognised part removed and whitespace collapsed.
    pub title: String,
    pub due: Option<Due>,
    /// Canonical names and paths, exactly as given in [`Known`].
    pub list: Option<String>,
    pub folder_path: Option<String>,
    pub assignee: Option<String>,
    pub repeat: Repeat,
    /// In input order.
    pub tokens: Vec<Token>,
}

/// What one matched phrase means.
enum Piece {
    Date(Date),
    Time(Time),
    Repeat(Repeat),
    List(String),
    Folder(String),
    Person(String),
}

impl Piece {
    fn kind(&self) -> TokenKind {
        match self {
            Piece::Date(_) => TokenKind::Date,
            Piece::Time(_) => TokenKind::Time,
            Piece::Repeat(_) => TokenKind::Repeat,
            Piece::List(_) => TokenKind::List,
            Piece::Folder(_) => TokenKind::Folder,
            Piece::Person(_) => TokenKind::Person,
        }
    }
}

/// A whitespace-separated word. `core` drops trailing sentence punctuation so "besok," still reads.
struct Word {
    span: Range<usize>,
    core_end: usize,
    lower: String,
}

fn split_words(input: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, ch) in input.char_indices().chain(std::iter::once((input.len(), ' '))) {
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                let core = input[s..i].trim_end_matches([',', '.', ';', '!', '?']);
                words.push(Word { span: s..i, core_end: s + core.len(), lower: core.to_lowercase() });
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    words
}

/// Filler words dropped together with the date or time they introduce ("on monday", "at 9am").
const CONNECTORS: [&str; 3] = ["at", "on", "pada"];

pub fn parse(input: &str, now: Now, known: &Known) -> QuickAdd {
    let words = split_words(input);
    let lower: Vec<&str> = words.iter().map(|w| w.lower.as_str()).collect();
    let mut consumed = vec![false; words.len()];
    let mut taken: Vec<TokenKind> = Vec::new();
    let mut out = QuickAdd {
        title: String::new(),
        due: None,
        list: None,
        folder_path: None,
        assignee: None,
        repeat: Repeat::None,
        tokens: Vec::new(),
    };
    let (mut date, mut time) = (None, None);

    let mut i = 0;
    while i < words.len() {
        // A second date (or list, person...) is left in the title rather than overriding the first.
        let fresh = |at: usize| match_at(&lower, at, now, known).filter(|(_, p)| !taken.contains(&p.kind()));
        let found = fresh(i).or_else(|| {
            if !CONNECTORS.contains(&lower[i]) {
                return None;
            }
            fresh(i + 1)
                .filter(|(_, p)| matches!(p.kind(), TokenKind::Date | TokenKind::Time))
                .map(|(n, p)| (n + 1, p))
        });
        let Some((n, piece)) = found else {
            i += 1;
            continue;
        };
        let range = words[i].span.start..words[i + n - 1].core_end;
        taken.push(piece.kind());
        out.tokens.push(Token { kind: piece.kind(), text: input[range.clone()].to_string(), range });
        consumed[i..i + n].fill(true);
        match piece {
            Piece::Date(d) => date = Some(d),
            Piece::Time(t) => time = Some(t),
            Piece::Repeat(r) => out.repeat = r,
            Piece::List(l) => out.list = Some(l),
            Piece::Folder(f) => out.folder_path = Some(f),
            Piece::Person(p) => out.assignee = Some(p),
        }
        i += n;
    }

    out.due = resolve_due(date, time, out.repeat, now);
    out.title = words
        .iter()
        .zip(&consumed)
        .filter(|(_, c)| !**c)
        .map(|(w, _)| &input[w.span.clone()])
        .collect::<Vec<_>>()
        .join(" ");
    out
}

fn resolve_due(date: Option<Date>, time: Option<Time>, repeat: Repeat, now: Now) -> Option<Due> {
    let today = now.date;
    let today_or_tomorrow = |t: Time| if t > now.time { today } else { today.add_days(1) };
    let date = match (date, time, repeat) {
        (Some(d), _, _) => d,
        // A weekly series includes today, unless today's slot has already gone.
        (None, Some(t), Repeat::Weekly(wd)) => {
            let d = next_weekday_from(today, wd);
            if d == today && t <= now.time {
                d.add_days(7)
            } else {
                d
            }
        }
        (None, None, Repeat::Weekly(wd)) => next_weekday_from(today, wd),
        (None, Some(t), _) => today_or_tomorrow(t),
        (None, None, Repeat::Daily) => today,
        (None, None, Repeat::None) => return None,
    };
    Some(Due { date, time })
}

/// The words from one position on; past the end reads as "".
#[derive(Clone, Copy)]
struct Ahead<'w> {
    words: &'w [&'w str],
    i: usize,
}

impl<'w> Ahead<'w> {
    fn get(self, k: usize) -> &'w str {
        self.words.get(self.i + k).copied().unwrap_or("")
    }
}

fn match_at(w: &[&str], i: usize, now: Now, known: &Known) -> Option<(usize, Piece)> {
    let a = *w.get(i)?;
    let at = Ahead { words: w, i };
    if let Some(rest) = a.strip_prefix('#') {
        return lexicon::match_name(rest, &known.lists).map(|n| (1, Piece::List(n.to_string())));
    }
    if let Some(rest) = a.strip_prefix('@') {
        return lexicon::match_name(rest, &known.people).map(|n| (1, Piece::Person(n.to_string())));
    }
    if a.starts_with('/') {
        return lexicon::match_folder(a, &known.folders).map(|f| (1, Piece::Folder(f.to_string())));
    }
    repeat_at(at)
        .map(|(n, r)| (n, Piece::Repeat(r)))
        .or_else(|| date_at(at, now.date).map(|(n, d)| (n, Piece::Date(d))))
        .or_else(|| time_at(at).map(|(n, t)| (n, Piece::Time(t))))
}

fn repeat_at(at: Ahead<'_>) -> Option<(usize, Repeat)> {
    let wd = lexicon::weekday;
    match (at.get(0), at.get(1)) {
        ("setiap" | "tiap", "hari") => match wd(at.get(2)) {
            Some(d) => Some((3, Repeat::Weekly(d))),
            None => Some((2, Repeat::Daily)),
        },
        ("every", "day") => Some((2, Repeat::Daily)),
        ("everyday" | "daily", _) => Some((1, Repeat::Daily)),
        ("setiap" | "tiap" | "every", b) => wd(b).map(|d| (2, Repeat::Weekly(d))),
        _ => None,
    }
}

fn date_at(at: Ahead<'_>, today: Date) -> Option<(usize, Date)> {
    let wd = lexicon::weekday;
    let (a, b) = (at.get(0), at.get(1));
    let rel = |n: usize, days: i64| Some((n, today.add_days(days)));
    match (a, b) {
        ("hari", "ini") => return rel(2, 0),
        ("today", _) => return rel(1, 0),
        ("besok" | "tomorrow", _) => return rel(1, 1),
        ("lusa", _) => return rel(1, 2),
        ("the", "day") if (at.get(2), at.get(3)) == ("after", "tomorrow") => return rel(4, 2),
        ("day", "after") if at.get(2) == "tomorrow" => return rel(3, 2),
        ("minggu" | "pekan", "depan") | ("next", "week") => {
            return Some((2, weekday_of_next_week(today, Weekday::Monday)))
        }
        ("next", b) => return wd(b).map(|d| (2, weekday_of_next_week(today, d))),
        // "sat set" is Indonesian slang for "quick", not Saturday.
        ("sat", "set") => return None,
        _ => {}
    }
    // Optional "hari" before a weekday: "hari senin", "hari senin depan".
    let skip = usize::from(a == "hari" && wd(b).is_some());
    if let Some(d) = wd(at.get(skip)) {
        return Some(if at.get(skip + 1) == "depan" {
            (skip + 2, weekday_of_next_week(today, d))
        } else {
            (skip + 1, next_weekday_after(today, d))
        });
    }
    if matches!(a, "tgl" | "tanggal") {
        let day = lexicon::day_of_month(b)?;
        if let Some(m) = lexicon::month(at.get(2)) {
            return next_day_and_month(today, day, m).map(|d| (3, d));
        }
        return next_day_of_month(today, day).map(|d| (2, d));
    }
    if let (Some(day), Some(m)) = (lexicon::day_of_month(a), lexicon::month(b)) {
        return next_day_and_month(today, day, m).map(|d| (2, d));
    }
    if let (Some(m), Some(day)) = (lexicon::month(a), lexicon::day_of_month(b)) {
        return next_day_and_month(today, day, m).map(|d| (2, d));
    }
    let (day, m) = lexicon::day_slash_month(a)?;
    next_day_and_month(today, day, m).map(|d| (1, d))
}

fn time_at(at: Ahead<'_>) -> Option<(usize, Time)> {
    let a = at.get(0);
    if matches!(a, "jam" | "pukul") {
        let c = lexicon::clock(at.get(1))?;
        if let Some(m) = c.meridiem {
            return c.with_meridiem(m).map(|t| (2, t));
        }
        if let Some(m) = lexicon::meridiem(at.get(2)) {
            return c.with_meridiem(m).map(|t| (3, t));
        }
        if let Some(p) = lexicon::day_part(at.get(2)) {
            return c.with_day_part(p).map(|t| (3, t));
        }
        return c.after_jam().map(|t| (2, t));
    }
    let c = lexicon::clock(a)?;
    if let Some(m) = c.meridiem {
        return c.with_meridiem(m).map(|t| (1, t));
    }
    if let Some(m) = lexicon::meridiem(at.get(1)) {
        return c.with_meridiem(m).map(|t| (2, t));
    }
    // A bare number is just a number ("2 malam" can be two nights); only a clock with minutes
    // counts on its own.
    c.minute?;
    if let Some(p) = lexicon::day_part(at.get(1)) {
        return c.with_day_part(p).map(|t| (2, t));
    }
    c.literal().map(|t| (1, t))
}
