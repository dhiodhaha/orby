# app-quickadd

The quick-add parser for tasks (ORB_SPEC 6.2, DESIGN.md "Tasks panel and sharing (v6.2)").
One typed line in Indonesian or English becomes a title plus a due date and time, a repeat,
a `#list`, a `/folder/sub` and an `@person`. Every recognised part comes back with its byte
range so the UI can draw token chips over it as the user types.

Pure std, no dependencies, no I/O. "Now" is always passed in; nothing reads the system clock.

## Public API

```rust
pub fn parse(input: &str, now: Now, known: &Known) -> QuickAdd;

pub struct Now { pub date: Date, pub time: Time }      // weekday is derived: now.weekday()
pub struct Known { pub lists: Vec<String>, pub folders: Vec<String>, pub people: Vec<String> }
// folders are paths with "/" between levels, e.g. "Work/Client A"

pub struct QuickAdd {
    pub title: String,               // input minus recognised parts, whitespace collapsed
    pub due: Option<Due>,
    pub list: Option<String>,        // canonical name from Known
    pub folder_path: Option<String>, // canonical path from Known
    pub assignee: Option<String>,    // canonical name from Known
    pub repeat: Repeat,
    pub tokens: Vec<Token>,          // input order; &input[t.range] == t.text
}
pub struct Due { pub date: Date, pub time: Option<Time> }
pub enum Repeat { None, Daily, Weekly(Weekday) }
pub struct Token { pub kind: TokenKind, pub range: Range<usize>, pub text: String }
pub enum TokenKind { Date, Time, Repeat, List, Folder, Person }

pub struct Date;  // Date::new(y, m, d) -> Option<Date>, .year() .month() .day() .weekday() .add_days(n)
pub struct Time;  // Time::new(h, m) -> Option<Time>, .hour() .minute()
pub enum Weekday { Monday, ..., Sunday }
pub fn days_in_month(year: i32, month: u8) -> u8;
```

`Date` and `Time` can only be built valid, so a `Due` never holds 31 November or 24:00.

## What it reads

| Kind | Indonesian | English |
|---|---|---|
| Relative day | `hari ini`, `besok`, `lusa` | `today`, `tomorrow`, `(the) day after tomorrow` |
| Weekday (next one, 1 to 7 days ahead) | `senin` .. `minggu`, `ahad`, `rab kam jum sab`, `jum'at`, optional `hari` before | `monday` .. `sunday`, `mon tue(s) wed thu(r)(s) fri sat sun` |
| Weekday of next week | `senin depan`, `hari senin depan` | `next monday` |
| Next week (its Monday) | `minggu depan`, `pekan depan` | `next week` |
| Day of month | `tgl 15`, `tanggal 15`, `tgl 15 nov` | |
| Day and month | `15 nov`, `17 agustus`, `15/11` | `nov 15`, `december 1` |
| Time | `jam 3` (1 to 6 means afternoon), `jam 3 sore`, `jam 7 pagi`, `jam 8 malam`, `jam 1 siang`, `pukul 10`, `jam 3.30` | `9am`, `9 pm`, `7:30pm` |
| Clock | `15.30`, `7.30 malam` | `15:30` |
| Repeat | `setiap hari`, `tiap hari`, `tiap senin`, `setiap hari kamis` | `every day`, `everyday`, `daily`, `every monday` |
| Sigils | `#list`, `/folder/sub`, `@person` | same |

Rules worth knowing:
- A time with no date is today if still ahead, otherwise tomorrow. An explicit date keeps its time even if it has passed.
- A calendar date with no year is the next one on or after today (`tgl 31` in November means 31 December).
- A weekly repeat with no date starts on the next matching weekday, today included unless its time has passed. A daily repeat with no date starts today (or tomorrow if its time has passed).
- `on`, `at` and `pada` right before a date or time are removed with it ("on monday at 3pm").
- `#`, `/` and `@` match case-insensitively. Spaces and underscores in a name read as hyphens, so `#client-a` finds "Client A". An exact name wins; otherwise a prefix must fit exactly one name (per segment for folders, same depth). Unknown or ambiguous ones stay in the title.
- The first match of each kind wins; a second date, list or person stays in the title.
- Not matched on purpose: `jam dinding` (no number after `jam`), weekday words inside longer words (`kesenian`, `fridays`), bare numbers (`2 malam` is two nights, `15.000` is a price), `sen`/`sel`/`min` (cent, cell, admin), `sat set` (slang for quick).

## Verified

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\quickadd\target"; cd C:\Users\dopamine\dev\clippy\crates\quickadd; cargo test; cargo clippy --all-targets -- -D warnings
```

Result: 55 tests pass (5 unit tests for date math and clock readings, 50 behaviour tests in
`tests/parse.rs` covering both languages, the spec examples `beli galon besok 9am #belanja`,
`rapat lusa jam 3 @dimas`, `Kirim invoice jumat /work/client-a`, and the false positives).
Clippy is clean with `-D warnings`; `cargo fmt --check` is clean.
