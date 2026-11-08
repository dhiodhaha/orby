//! What the Downloads panel, the dial and the download pills say, read from the app_downloads
//! queue. Pure: the clock comes in as a function, so tests pin "10:42".

use std::path::Path;
use std::time::SystemTime;

use app_downloads::{
    AudioCodec, Counts, ErrorKind, Format, Job, JobId, JobState, MediaKind, MediaLink, Overall,
    Queue, Ring, Site, Tone,
};

const MB: f64 = 1_000_000.0;

/// "412 MB", "6.1 MB", "1.2 GB", "49 KB".
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    let one = |v: f64, unit: &str| {
        if v < 10.0 {
            format!("{v:.1} {unit}")
        } else {
            format!("{} {unit}", v.round() as u64)
        }
    };
    if b >= 1000.0 * MB {
        format!("{:.1} GB", b / (1000.0 * MB))
    } else if b >= MB {
        one(b / MB, "MB")
    } else {
        format!("{} KB", (b / 1000.0).ceil() as u64)
    }
}

/// "190 of 412 MB": both in the bigger number's unit, so the pair reads at a glance.
pub fn of(done: u64, total: u64) -> String {
    if total >= 1000 * 1_000_000 {
        format!("{:.1} of {}", done as f64 / (1000.0 * MB), size(total))
    } else if total >= 1_000_000 {
        let d = done as f64 / MB;
        let d = if total as f64 / MB < 10.0 {
            format!("{d:.1}")
        } else {
            format!("{}", d.round() as u64)
        };
        format!("{d} of {}", size(total))
    } else {
        format!(
            "{} of {}",
            (done as f64 / 1000.0).ceil() as u64,
            size(total)
        )
    }
}

/// "4.2 MB/s".
pub fn speed(bytes_per_s: f64) -> String {
    if bytes_per_s >= MB {
        format!("{:.1} MB/s", bytes_per_s / MB)
    } else {
        format!("{} KB/s", (bytes_per_s / 1000.0).round() as u64)
    }
}

/// "53 s left", "4 min left", "1 h 5 min left".
pub fn eta(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s left"),
        60..=3599 => format!("{} min left", secs.div_ceil(60)),
        _ => format!("{} h {} min left", secs / 3600, (secs % 3600) / 60),
    }
}

/// The link as a row or pill shows it: "youtu.be/q7Lm3kV", "instagram.com/reel/C9xT2".
pub fn short_link(link: &MediaLink) -> String {
    let url = link.url();
    if link.site() == Site::YouTube {
        if let Some(id) = url.split("v=").nth(1) {
            return format!("youtu.be/{id}");
        }
    }
    url.trim_start_matches("https://")
        .trim_start_matches("www.")
        .trim_end_matches('/')
        .to_string()
}

pub fn title(job: &Job) -> String {
    job.title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| short_link(&job.request.link))
}

/// "D:" for the folder's drive, for "Disk full on D:".
pub fn drive(folder: &Path) -> String {
    let s = folder.to_string_lossy();
    match s.as_bytes() {
        [l, b':', ..] => format!("{}:", *l as char),
        _ => "this drive".into(),
    }
}

/// The format menu's rows, in the frame's order. The second value is the kind ("Video",
/// "Audio only"), the third the format in muted ("1080p MP4"), the fourth a key hint.
pub fn menu_rows() -> [(Format, &'static str, String, &'static str); 4] {
    let v = |h| Format::Video {
        max_height: Some(h),
    };
    let a = |codec| Format::Audio { codec };
    [
        (v(1080), "Video", v(1080).label(), ""),
        (v(720), "Video", v(720).label(), ""),
        (
            a(AudioCodec::M4a),
            "Audio only",
            a(AudioCodec::M4a).label(),
            "A",
        ),
        (
            a(AudioCodec::Mp3),
            "Audio only",
            a(AudioCodec::Mp3).label(),
            "",
        ),
    ]
}

/// The offer pill's words: "Save this video?" with the format it will use.
pub fn offer_words(format: Format) -> (&'static str, String) {
    match format.kind() {
        MediaKind::Video => ("Save this video?", format.label()),
        MediaKind::Audio => ("Save this audio?", format.label()),
    }
}

/// "2 running · 1 waiting", "3 need you · 2 paused", "2 done today", "nothing yet".
pub fn count_label(c: Counts, offline: usize) -> String {
    let need = c.failed - offline;
    let paused = c.paused + offline;
    let parts: Vec<String> = [
        (need, "need you"),
        (c.running, "running"),
        (c.waiting, "waiting"),
        (paused, "paused"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, w)| format!("{n} {w}"))
    .collect();
    if !parts.is_empty() {
        parts.join(" · ")
    } else if c.done > 0 {
        format!("{} done today", c.done)
    } else {
        "nothing yet".into()
    }
}

/// The hover pill on the orb: "2 of 3 · 46%".
pub fn overall_words(o: Overall) -> String {
    let pct = o
        .fraction()
        .map(|f| format!(" · {}%", (f * 100.0).floor() as u32))
        .unwrap_or_default();
    format!("{} of {}{pct}", o.files_done, o.files)
}

/// The More chip's live value on the dial: "64%", "reading", "paused", "waiting", "disk full".
/// The grey ring covers both a pause and a queue that only waits (no engine yet).
pub fn dial_value(ring: Ring, c: Counts, offline: usize) -> String {
    match ring {
        Ring::Hidden => String::new(),
        Ring::Reading => "reading".into(),
        Ring::Progress {
            tone: Tone::Grey, ..
        } if c.paused + offline == 0 => "waiting".into(),
        Ring::Progress {
            tone: Tone::Grey, ..
        } => "paused".into(),
        Ring::Progress {
            tone: Tone::Ember, ..
        } => "disk full".into(),
        Ring::Progress { fraction, .. } => format!("{}%", (fraction * 100.0).floor() as u32),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    /// Failures you can act on (offline is not one: it waits).
    NeedsYou,
    Now,
    Done,
}

/// The status chip on the left of a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chip {
    /// No size yet: the quarter arc.
    Reading,
    /// The progress ring: ink while running, grey while offline, ember on disk full.
    Ring {
        fraction: f32,
        tone: Tone,
    },
    /// The grey arc with a pause glyph.
    Paused {
        fraction: f32,
    },
    /// Still to come: the dotted ring.
    Waiting,
    Done,
    /// An ember chip with its icon.
    Failed(ErrorKind),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Btn {
    Pause,
    Resume,
    Cancel,
    /// The waiting row's format chip.
    Format,
    Open,
    Folder,
    Remove,
    Retry,
    ChangeFolder,
}

impl Btn {
    /// Text buttons; the others are 28 px round icon buttons.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Btn::Open => Some("Open"),
            Btn::Remove => Some("Remove"),
            Btn::Retry => Some("Retry"),
            Btn::ChangeFolder => Some("Change folder"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: JobId,
    pub section: Section,
    pub title: String,
    pub line: String,
    pub chip: Chip,
    pub audio: bool,
    /// Left to right.
    pub buttons: Vec<Btn>,
    /// "720p" on a waiting row's format chip.
    pub format_chip: String,
}

fn fraction(done: u64, total: Option<u64>) -> f32 {
    match total {
        Some(t) if t > 0 => (done as f32 / t as f32).min(1.0),
        _ => 0.0,
    }
}

fn format_chip(f: Format) -> String {
    match f {
        Format::Video {
            max_height: Some(h),
        } => format!("{h}p"),
        Format::Video { max_height: None } => "Best".into(),
        Format::Audio { codec } => codec.label().into(),
    }
}

fn row(job: &Job, first_waiting: bool, hhmm: &dyn Fn(SystemTime) -> String) -> Row {
    let f = job.request.format;
    let audio = f.kind() == MediaKind::Audio;
    let prefix = if audio {
        format!("Audio {} · ", f.label())
    } else {
        String::new()
    };
    let mut title = title(job);
    let (section, chip, line, buttons) = match &job.state {
        JobState::Reading => (
            Section::Now,
            Chip::Reading,
            format!("{} · finding formats", short_link(&job.request.link)),
            vec![Btn::Cancel],
        ),
        JobState::Running(p) => {
            let mut parts = vec![match p.total {
                Some(t) => format!("{prefix}{}", of(p.downloaded, t)),
                None => format!("{prefix}{}", size(p.downloaded)),
            }];
            parts.extend(p.speed.filter(|s| *s > 0.0).map(speed));
            parts.extend(p.eta.map(eta));
            (
                Section::Now,
                Chip::Ring {
                    fraction: fraction(p.downloaded, p.total),
                    tone: Tone::Ink,
                },
                parts.join(" · "),
                vec![Btn::Pause, Btn::Cancel],
            )
        }
        JobState::Waiting => {
            let next = if first_waiting { " · starts next" } else { "" };
            (
                Section::Now,
                Chip::Waiting,
                format!("Waiting · {}{next}", f.label()),
                vec![Btn::Format, Btn::Cancel],
            )
        }
        JobState::Paused { downloaded, total } => {
            let amount = total.map_or_else(|| size(*downloaded), |t| of(*downloaded, t));
            (
                Section::Now,
                Chip::Paused {
                    fraction: fraction(*downloaded, *total),
                },
                format!("Paused · {amount}"),
                vec![Btn::Resume, Btn::Cancel],
            )
        }
        JobState::Done { size: s, at, .. } => (
            Section::Done,
            Chip::Done,
            format!("{} · {} · {}", f.label(), size(*s), hhmm(*at)),
            vec![Btn::Open, Btn::Folder],
        ),
        JobState::Failed {
            kind,
            downloaded,
            total,
        } => {
            let pct = || {
                format!(
                    "{}%",
                    (fraction(*downloaded, *total) * 100.0).floor() as u32
                )
            };
            match kind {
                ErrorKind::Offline => (
                    Section::Now,
                    Chip::Ring {
                        fraction: fraction(*downloaded, *total),
                        tone: Tone::Grey,
                    },
                    format!("Waiting for internet · {}", pct()),
                    vec![Btn::Retry, Btn::Cancel],
                ),
                ErrorKind::DiskFull => {
                    let amount = total.map_or_else(|| size(*downloaded), |t| of(*downloaded, t));
                    (
                        Section::NeedsYou,
                        Chip::Ring {
                            fraction: fraction(*downloaded, *total),
                            tone: Tone::Ember,
                        },
                        format!("Disk full on {} · {amount}", drive(&job.request.folder)),
                        vec![Btn::ChangeFolder],
                    )
                }
                ErrorKind::Private => (
                    Section::NeedsYou,
                    Chip::Failed(*kind),
                    "Private. Only the owner can share it.".into(),
                    vec![Btn::Remove],
                ),
                ErrorKind::Removed => (
                    Section::NeedsYou,
                    Chip::Failed(*kind),
                    "Removed by the uploader".into(),
                    vec![Btn::Remove],
                ),
                ErrorKind::FfmpegMissing => (
                    Section::NeedsYou,
                    Chip::Failed(*kind),
                    "Needs ffmpeg to put video and sound together".into(),
                    vec![Btn::Retry, Btn::Cancel],
                ),
                ErrorKind::Unsupported | ErrorKind::Other => {
                    title = "Couldn't read this link".into();
                    (
                        Section::NeedsYou,
                        Chip::Failed(*kind),
                        short_link(&job.request.link),
                        vec![Btn::Retry, Btn::Cancel],
                    )
                }
            }
        }
    };
    Row {
        id: job.id,
        section,
        title,
        line,
        chip,
        audio,
        buttons,
        format_chip: format_chip(f),
    }
}

/// What Cancel deletes from the files a job reported: their part files, and the separate video
/// and audio pieces of a merge ("name.f137.mp4"). A file under the final name stays, because it
/// can be an earlier, finished copy of the same video.
pub fn leftovers(files: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let with = |f: &Path, s: &str| {
        let mut o = f.as_os_str().to_owned();
        o.push(s);
        std::path::PathBuf::from(o)
    };
    let piece = |f: &Path| {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        stem.rsplit('.').next().is_some_and(|last| {
            last.len() > 1 && last.starts_with('f') && last[1..].chars().all(|c| c.is_ascii_digit())
        })
    };
    let mut out = Vec::new();
    for f in files {
        out.push(with(f, ".part"));
        out.push(with(f, ".ytdl"));
        if piece(f) {
            out.push(f.clone());
        }
    }
    out
}

/// Change folder: where each half-done file goes in the new folder, so the job goes on from
/// where it stopped instead of starting over (and the full disk gets its space back).
pub fn moves(
    files: &[std::path::PathBuf],
    to: &Path,
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    leftovers(files)
        .into_iter()
        .filter(|f| f.parent() != Some(to))
        .filter_map(|f| f.file_name().map(|n| to.join(n)).map(|dst| (f, dst)))
        .collect()
}

/// The link already has a job that is not done; a second one would write the same part file.
pub fn in_queue(q: &Queue, link: &MediaLink) -> bool {
    q.jobs()
        .iter()
        .any(|j| j.request.link == *link && !matches!(j.state, JobState::Done { .. }))
}

/// The rows in panel order: Needs you, Now (in queue order), then Done today, latest first.
pub fn rows(q: &Queue, hhmm: &dyn Fn(SystemTime) -> String) -> Vec<Row> {
    let first_waiting = q
        .jobs()
        .iter()
        .find(|j| j.state == JobState::Waiting)
        .map(|j| j.id);
    let all: Vec<Row> = q
        .jobs()
        .iter()
        .map(|j| row(j, Some(j.id) == first_waiting, hhmm))
        .collect();
    let mut out: Vec<Row> = Vec::with_capacity(all.len());
    for s in [Section::NeedsYou, Section::Now] {
        out.extend(all.iter().filter(|r| r.section == s).cloned());
    }
    out.extend(
        all.iter()
            .rev()
            .filter(|r| r.section == Section::Done)
            .cloned(),
    );
    out
}

/// Offline failures count as paused in the title, not as needing you.
pub fn offline_count(q: &Queue) -> usize {
    q.jobs()
        .iter()
        .filter(|j| {
            matches!(
                j.state,
                JobState::Failed {
                    kind: ErrorKind::Offline,
                    ..
                }
            )
        })
        .count()
}

/// The error pill's words and whether it offers a next step: (title, value, action label).
pub fn error_words(kind: ErrorKind, folder: &Path) -> (&'static str, String, Option<&'static str>) {
    match kind {
        ErrorKind::Private => ("Couldn't save", "video is private".into(), None),
        ErrorKind::Removed => ("Couldn't save", "video was removed".into(), None),
        ErrorKind::DiskFull => (
            "Couldn't save",
            format!("{} is full", drive(folder)),
            Some("Change folder"),
        ),
        ErrorKind::Offline => ("Offline", "downloads paused".into(), None),
        ErrorKind::FfmpegMissing => ("Couldn't save", "needs ffmpeg".into(), None),
        ErrorKind::Unsupported | ErrorKind::Other => {
            ("Couldn't read this link", String::new(), Some("Retry"))
        }
    }
}

/// The done pill's muted title, cut so the pill stays short.
pub fn pill_title(t: &str) -> String {
    const MAX: usize = 28;
    if t.chars().count() <= MAX {
        t.to_string()
    } else {
        format!(
            "{}…",
            t.chars().take(MAX - 1).collect::<String>().trim_end()
        )
    }
}

/// What the paste field says after Paste.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Empty,
    /// A link from a site the engine does not know: ember outline and the sites it does.
    Unsupported(String),
    /// Nothing that looks like a link on the clipboard.
    NoLink,
}

/// Paste's verdict on the clipboard text.
pub fn paste_verdict(text: &str) -> Result<MediaLink, Field> {
    let t = text.trim();
    if let Some(link) = MediaLink::parse(t) {
        return Ok(link);
    }
    let looks_like_link = !t.contains(char::is_whitespace)
        && (t.starts_with("http://")
            || t.starts_with("https://")
            || t.starts_with("www.")
            || t.contains(".com/")
            || t.contains(".id/"));
    if looks_like_link {
        Err(Field::Unsupported(
            t.trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_string(),
        ))
    } else {
        Err(Field::NoLink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_downloads::{Outcome, Progress, Update};
    use std::path::PathBuf;
    use std::time::Duration;

    fn link(id: &str) -> MediaLink {
        MediaLink::parse(&format!("https://youtu.be/{id}")).unwrap()
    }

    fn clock(_: SystemTime) -> String {
        "10:42".into()
    }

    fn at() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_000_000)
    }

    #[test]
    fn sizes_speeds_and_time_left_read_like_the_frame() {
        assert_eq!(of(190_000_000, 412_000_000), "190 of 412 MB");
        assert_eq!(of(6_100_000, 7_800_000), "6.1 of 7.8 MB");
        assert_eq!(of(25_000, 41_200), "25 of 42 KB");
        assert_eq!(size(412_000_000), "412 MB");
        assert_eq!(size(1_200_000_000), "1.2 GB");
        assert_eq!(speed(4_200_000.0), "4.2 MB/s");
        assert_eq!(eta(53), "53 s left");
        assert_eq!(eta(125), "3 min left");
        assert_eq!(eta(3900), "1 h 5 min left");
    }

    #[test]
    fn youtube_links_show_short() {
        assert_eq!(short_link(&link("q7Lm3kVabcd")), "youtu.be/q7Lm3kVabcd");
        let ig = MediaLink::parse("https://www.instagram.com/reel/C9xT2abc/?igsh=x").unwrap();
        assert_eq!(short_link(&ig), "instagram.com/reel/C9xT2abc");
    }

    #[test]
    fn rows_follow_the_queue_states_and_sections() {
        let mut q = Queue::new();
        let folder = PathBuf::from(r"D:\Video");
        let a = q.add(link("aaaaaaaaaaa"), Format::default(), folder.clone());
        let b = q.add(
            link("bbbbbbbbbbb"),
            Format::Audio {
                codec: AudioCodec::M4a,
            },
            folder.clone(),
        );
        let c = q.add(
            link("ccccccccccc"),
            Format::Video {
                max_height: Some(720),
            },
            folder.clone(),
        );
        let d = q.add(link("ddddddddddd"), Format::default(), folder.clone());
        q.start_ready();
        q.apply(a, Update::Title("Kajian Jumat (sample)".into()), at());
        q.apply(
            a,
            Update::Progress(Progress {
                downloaded: 190_000_000,
                total: Some(412_000_000),
                speed: Some(4_200_000.0),
                eta: Some(53),
            }),
            at(),
        );
        q.apply(
            b,
            Update::Finished(Outcome::Failed(ErrorKind::DiskFull)),
            at(),
        );

        let rows = rows(&q, &clock);
        let ids: Vec<JobId> = rows.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec![b, a, c, d],
            "needs you first, then the queue in order"
        );
        assert_eq!(rows[0].line, "Disk full on D: · 0 KB");
        assert_eq!(rows[0].buttons, vec![Btn::ChangeFolder]);
        assert!(matches!(
            rows[0].chip,
            Chip::Ring {
                tone: Tone::Ember,
                ..
            }
        ));
        assert_eq!(rows[1].title, "Kajian Jumat (sample)");
        assert_eq!(rows[1].line, "190 of 412 MB · 4.2 MB/s · 53 s left");
        assert_eq!(rows[1].buttons, vec![Btn::Pause, Btn::Cancel]);
        assert_eq!(rows[2].line, "Waiting · 720p MP4 · starts next");
        assert_eq!(rows[2].format_chip, "720p");
        assert_eq!(rows[3].line, "Waiting · 1080p MP4");

        q.pause(a).unwrap();
        let r = &super::rows(&q, &clock)[1];
        assert_eq!(
            (r.line.as_str(), r.buttons.clone()),
            ("Paused · 190 of 412 MB", vec![Btn::Resume, Btn::Cancel])
        );
    }

    #[test]
    fn done_rows_come_last_latest_first_with_their_time() {
        let mut q = Queue::new();
        let a = q.add(
            link("aaaaaaaaaaa"),
            Format::default(),
            PathBuf::from(r"C:\D"),
        );
        let b = q.add(
            link("bbbbbbbbbbb"),
            Format::Audio {
                codec: AudioCodec::M4a,
            },
            PathBuf::from(r"C:\D"),
        );
        q.start_ready();
        q.apply(
            a,
            Update::Finished(Outcome::Done {
                path: PathBuf::from(r"C:\D\a.mp4"),
                size: 412_000_000,
            }),
            at(),
        );
        q.apply(
            b,
            Update::Finished(Outcome::Done {
                path: PathBuf::from(r"C:\D\b.m4a"),
                size: 9_400_000,
            }),
            at(),
        );
        let rows = rows(&q, &clock);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(rows[1].line, "1080p MP4 · 412 MB · 10:42");
        assert_eq!(rows[0].line, "M4A · 9.4 MB · 10:42");
        assert_eq!(rows[0].buttons, vec![Btn::Open, Btn::Folder]);
        assert_eq!(count_label(q.counts(), 0), "2 done today");
    }

    #[test]
    fn failures_name_the_cause_and_the_next_step() {
        let mut q = Queue::new();
        let ids: Vec<JobId> = ["ppppppppppp", "rrrrrrrrrrr", "ooooooooooo", "xxxxxxxxxxx"]
            .iter()
            .map(|id| q.add(link(id), Format::default(), PathBuf::from(r"C:\D")))
            .collect();
        q.start_ready();
        q.start_ready();
        for (id, kind) in ids.iter().zip([ErrorKind::Private, ErrorKind::Removed]) {
            q.apply(*id, Update::Finished(Outcome::Failed(kind)), at());
        }
        q.start_ready();
        q.apply(
            ids[2],
            Update::Progress(Progress {
                downloaded: 34,
                total: Some(100),
                speed: None,
                eta: None,
            }),
            at(),
        );
        q.apply(
            ids[2],
            Update::Finished(Outcome::Failed(ErrorKind::Offline)),
            at(),
        );
        // An unnamed failure goes on once by itself; the second one in a row shows.
        for _ in 0..2 {
            q.apply(
                ids[3],
                Update::Finished(Outcome::Failed(ErrorKind::Other)),
                at(),
            );
        }
        let rows = rows(&q, &clock);
        let by = |id: JobId| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by(ids[0]).line, "Private. Only the owner can share it.");
        assert_eq!(by(ids[0]).buttons, vec![Btn::Remove]);
        assert_eq!(by(ids[1]).line, "Removed by the uploader");
        assert_eq!(by(ids[2]).line, "Waiting for internet · 34%");
        assert_eq!(by(ids[2]).section, Section::Now, "offline is not an error");
        assert!(matches!(
            by(ids[2]).chip,
            Chip::Ring {
                tone: Tone::Grey,
                ..
            }
        ));
        assert_eq!(by(ids[3]).title, "Couldn't read this link");
        assert_eq!(by(ids[3]).line, "youtu.be/xxxxxxxxxxx");
        assert_eq!(
            count_label(q.counts(), offline_count(&q)),
            "3 need you · 1 paused"
        );
        assert_eq!(
            error_words(ErrorKind::DiskFull, Path::new(r"D:\Video")),
            (
                "Couldn't save",
                "D: is full".to_string(),
                Some("Change folder")
            )
        );
        assert_eq!(
            error_words(ErrorKind::Private, Path::new(r"D:\")).2,
            None,
            "nothing to retry"
        );
    }

    #[test]
    fn count_title_ring_words_and_dial_value() {
        let c = Counts {
            running: 2,
            waiting: 1,
            ..Counts::default()
        };
        assert_eq!(count_label(c, 0), "2 running · 1 waiting");
        assert_eq!(count_label(Counts::default(), 0), "nothing yet");
        assert_eq!(
            overall_words(Overall {
                files_done: 2,
                files: 3,
                bytes_done: 46,
                bytes_total: 100
            }),
            "2 of 3 · 46%"
        );
        let none = Counts::default();
        assert_eq!(
            dial_value(
                Ring::Progress {
                    fraction: 0.64,
                    tone: Tone::Ink
                },
                none,
                0
            ),
            "64%"
        );
        assert_eq!(dial_value(Ring::Hidden, none, 0), "");
        let grey = Ring::Progress {
            fraction: 0.0,
            tone: Tone::Grey,
        };
        assert_eq!(
            dial_value(grey, Counts { waiting: 2, ..none }, 0),
            "waiting"
        );
        assert_eq!(dial_value(grey, Counts { paused: 1, ..none }, 0), "paused");
    }

    #[test]
    fn offer_names_video_or_audio_with_its_format() {
        assert_eq!(
            offer_words(Format::default()),
            ("Save this video?", "1080p MP4".to_string())
        );
        assert_eq!(
            offer_words(Format::Audio {
                codec: AudioCodec::M4a
            }),
            ("Save this audio?", "M4A".to_string())
        );
        let rows = menu_rows();
        assert_eq!(
            rows.iter().map(|r| r.2.as_str()).collect::<Vec<_>>(),
            vec!["1080p MP4", "720p MP4", "M4A", "MP3"]
        );
    }

    #[test]
    fn paste_tells_unsupported_links_from_no_link() {
        assert!(paste_verdict("  https://youtu.be/dQw4w9WgXcQ ").is_ok());
        assert_eq!(
            paste_verdict("https://shopee.co.id/product/17820443"),
            Err(Field::Unsupported("shopee.co.id/product/17820443".into()))
        );
        assert_eq!(paste_verdict("beli beras 5 kg"), Err(Field::NoLink));
    }

    #[test]
    fn cancel_deletes_parts_and_merge_pieces_but_never_a_file_under_the_final_name() {
        let files = vec![
            PathBuf::from(r"C:\D\Clip [abc].f137.mp4"),
            PathBuf::from(r"C:\D\Clip [abc].mp4"),
        ];
        let gone = leftovers(&files);
        assert!(gone.contains(&PathBuf::from(r"C:\D\Clip [abc].f137.mp4")));
        assert!(gone.contains(&PathBuf::from(r"C:\D\Clip [abc].f137.mp4.part")));
        assert!(gone.contains(&PathBuf::from(r"C:\D\Clip [abc].mp4.part")));
        assert!(gone.contains(&PathBuf::from(r"C:\D\Clip [abc].mp4.ytdl")));
        assert!(!gone.contains(&PathBuf::from(r"C:\D\Clip [abc].mp4")));
    }

    #[test]
    fn change_folder_moves_the_half_done_files_and_nothing_under_the_final_name() {
        let files = vec![
            PathBuf::from(r"C:\D\Clip [abc].f137.mp4"),
            PathBuf::from(r"C:\D\Clip [abc].mp4"),
        ];
        let m = moves(&files, Path::new(r"E:\New"));
        assert!(m.contains(&(
            PathBuf::from(r"C:\D\Clip [abc].mp4.part"),
            PathBuf::from(r"E:\New\Clip [abc].mp4.part")
        )));
        assert!(m.contains(&(
            PathBuf::from(r"C:\D\Clip [abc].f137.mp4"),
            PathBuf::from(r"E:\New\Clip [abc].f137.mp4")
        )));
        assert!(m
            .iter()
            .all(|(src, _)| src != &PathBuf::from(r"C:\D\Clip [abc].mp4")));
        assert!(
            moves(&files, Path::new(r"C:\D")).is_empty(),
            "the same folder moves nothing"
        );
    }

    #[test]
    fn a_link_with_an_unfinished_job_is_in_the_queue_and_a_saved_one_is_not() {
        let mut q = Queue::new();
        let (a, b) = (link("aaaaaaaaaaa"), link("bbbbbbbbbbb"));
        let ja = q.add(a.clone(), Format::default(), PathBuf::from(r"C:\D"));
        assert!(in_queue(&q, &a), "waiting");
        q.start_ready();
        assert!(in_queue(&q, &a), "reading");
        q.apply(
            ja,
            Update::Finished(Outcome::Failed(ErrorKind::DiskFull)),
            at(),
        );
        assert!(in_queue(&q, &a), "failed rows still hold the link");
        assert!(!in_queue(&q, &b));
        let jb = q.add(b.clone(), Format::default(), PathBuf::from(r"C:\D"));
        q.start_ready();
        q.apply(
            jb,
            Update::Finished(Outcome::Done {
                path: PathBuf::from(r"C:\D\b.mp4"),
                size: 1,
            }),
            at(),
        );
        assert!(
            !in_queue(&q, &b),
            "a saved link is offered as Already saved instead"
        );
    }

    #[test]
    fn long_titles_are_cut_for_the_pill() {
        assert_eq!(pill_title("Ngaji Filsafat 412"), "Ngaji Filsafat 412");
        let t = pill_title("A very long sample title that goes on and on");
        assert!(t.ends_with('…') && t.chars().count() <= 28);
    }
}
