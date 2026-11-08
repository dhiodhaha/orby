//! Reading yt-dlp's output: progress, destinations, and why it failed.

use std::path::{Path, PathBuf};

use crate::args::PROGRESS_PREFIX;

/// Why a download failed, as the pills and panel rows name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Private,
    Removed,
    Unsupported,
    DiskFull,
    Offline,
    FfmpegMissing,
    /// "Couldn't read this link", offered with Retry.
    Other,
}

impl ErrorKind {
    /// Disk full and offline are paused, not lost: the row keeps its progress and the job goes on
    /// later (in a new folder, or when the internet is back).
    pub fn keeps_progress(self) -> bool {
        matches!(self, ErrorKind::DiskFull | ErrorKind::Offline)
    }
}

/// Classifies yt-dlp's error text. Order matters: a disk or network failure can mention the
/// video page too, so the causes the user can fix come first.
pub fn classify_error(text: &str) -> ErrorKind {
    let t = text.to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if any(&["no space left", "errno 28", "not enough space on the disk", "winerror 112", "disk full"]) {
        ErrorKind::DiskFull
    } else if any(&["ffmpeg not found", "ffmpeg is not installed", "ffprobe and ffmpeg not found", "ffmpeg-location"]) {
        ErrorKind::FfmpegMissing
    } else if any(&[
        "getaddrinfo failed",
        "failed to resolve",
        "name or service not known",
        "temporary failure in name resolution",
        "nodename nor servname",
        "network is unreachable",
        "no route to host",
        "winerror 10051",
        "winerror 10065",
        "errno 11001",
        "errno 11004",
    ]) {
        ErrorKind::Offline
    } else if any(&["private video", "video is private", "this account is private", "is private"]) {
        ErrorKind::Private
    } else if any(&[
        "removed by the uploader",
        "has been removed",
        "video has been deleted",
        "video unavailable",
        "no longer available",
        "account associated with this video has been terminated",
        "copyright claim",
    ]) {
        ErrorKind::Removed
    } else if any(&["unsupported url", "is not a valid url"]) {
        ErrorKind::Unsupported
    } else {
        ErrorKind::Other
    }
}

/// One progress report from our template, for the file being downloaded right now.
#[derive(Clone, Debug, PartialEq)]
pub struct RawProgress {
    pub title: Option<String>,
    pub file: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Bytes per second.
    pub speed: Option<f64>,
    /// Seconds left.
    pub eta: Option<u64>,
}

/// What one output line means. Lines we do not care about are `Other`.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Progress(RawProgress),
    /// A file this run is creating.
    Destination(PathBuf),
    /// A file that was already there before this run. Never ours to delete.
    AlreadyDownloaded(PathBuf),
    Error(ErrorKind),
    Other,
}

pub fn parse_line(line: &str) -> Line {
    let line = line.trim_end();
    if let Some(json) = line.strip_prefix(PROGRESS_PREFIX) {
        return parse_progress(json).map_or(Line::Other, Line::Progress);
    }
    if let Some(msg) = line.strip_prefix("ERROR:") {
        return Line::Error(classify_error(msg));
    }
    let dest = |s: &str| PathBuf::from(s.trim().trim_matches('"'));
    if let Some(p) = line.strip_prefix("[download] Destination: ").or_else(|| line.strip_prefix("[ExtractAudio] Destination: ")) {
        return Line::Destination(dest(p));
    }
    if let Some(p) = line.strip_prefix("[Merger] Merging formats into ") {
        return Line::Destination(dest(p));
    }
    if let Some(p) = line.strip_prefix("[download] ").and_then(|s| s.strip_suffix(" has already been downloaded")) {
        return Line::AlreadyDownloaded(dest(p));
    }
    Line::Other
}

fn parse_progress(json: &str) -> Option<RawProgress> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let p = v.get("progress")?;
    let num = |key: &str| p.get(key).and_then(serde_json::Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0);
    let downloaded = num("downloaded_bytes")? as u64;
    let total = num("total_bytes").or_else(|| num("total_bytes_estimate")).map(|t| t as u64);
    let text = |s: &serde_json::Value| s.as_str().map(str::to_string);
    Some(RawProgress {
        title: v.get("title").and_then(text),
        file: p.get("filename").and_then(text),
        downloaded,
        total,
        speed: num("speed"),
        eta: num("eta").map(|e| e as u64),
    })
}

/// Progress of the whole job, across the separate video and audio files a merge downloads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Bytes per second.
    pub speed: Option<f64>,
    /// Seconds left for the current file.
    pub eta: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done { path: PathBuf, size: u64 },
    Failed(ErrorKind),
}

/// What the runner reports for a job, in order. `Finished` is always last.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Title(String),
    Progress(Progress),
    /// A file this run creates; cancel deletes it (and its `.part`).
    Destination(PathBuf),
    /// yt-dlp's own words for a failure, for the log. Sent once, right before a failed `Finished`.
    ErrorText(String),
    Finished(Outcome),
}

/// Longest error text kept for the log.
pub const ERROR_TEXT_MAX: usize = 400;

/// How many recent warnings stand in when a run fails without an ERROR line.
const WARNINGS_KEPT: usize = 2;

fn capped(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(ERROR_TEXT_MAX) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// Folds the lines of one run into job-level updates, the final outcome and, on failure, why.
#[derive(Debug, Default)]
pub struct Tracker {
    title: Option<String>,
    file: Option<String>,
    /// Bytes of the files finished before the current one.
    base: u64,
    /// Best known size of the current file.
    current_size: u64,
    progress: Progress,
    final_path: Option<PathBuf>,
    error: Option<ErrorKind>,
    /// The first ERROR line, as yt-dlp wrote it.
    error_line: Option<String>,
    /// The last few WARNING lines.
    warnings: Vec<String>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, line: &str) -> Vec<Update> {
        let mut out = Vec::new();
        match parse_line(line) {
            Line::Progress(raw) => {
                if let Some(t) = raw.title.filter(|t| self.title.as_ref() != Some(t)) {
                    self.title = Some(t.clone());
                    out.push(Update::Title(t));
                }
                if raw.file.is_some() && self.file.is_some() && raw.file != self.file {
                    self.base += self.current_size;
                    self.current_size = 0;
                }
                if raw.file.is_some() {
                    self.file = raw.file;
                }
                self.current_size = raw.total.unwrap_or(0).max(raw.downloaded);
                self.progress = Progress {
                    downloaded: self.base + raw.downloaded,
                    total: raw.total.map(|t| self.base + t),
                    speed: raw.speed,
                    eta: raw.eta,
                };
                out.push(Update::Progress(self.progress));
            }
            Line::Destination(p) => {
                self.final_path = Some(p.clone());
                out.push(Update::Destination(p));
            }
            Line::AlreadyDownloaded(p) => self.final_path = Some(p),
            // The first error is the cause; later ones are usually its consequences.
            Line::Error(kind) => {
                self.error.get_or_insert(kind);
                self.error_line.get_or_insert_with(|| capped(line));
            }
            Line::Other => {
                if line.starts_with("WARNING:") {
                    if self.warnings.len() == WARNINGS_KEPT {
                        self.warnings.remove(0);
                    }
                    self.warnings.push(capped(line));
                }
            }
        }
        out
    }

    /// Why the run failed in yt-dlp's words: the first ERROR line, else the last warnings.
    pub fn error_text(&self) -> Option<String> {
        self.error_line.clone().or_else(|| (!self.warnings.is_empty()).then(|| self.warnings.join(" | ")))
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// The outcome once the process has exited. `size_of` reads the final file's size; the
    /// tracked byte count stands in when it cannot.
    pub fn finish(&self, exit_ok: bool, size_of: impl Fn(&Path) -> Option<u64>) -> Outcome {
        match (&self.error, &self.final_path) {
            (None, Some(path)) if exit_ok => {
                Outcome::Done { path: path.clone(), size: size_of(path).unwrap_or(self.progress.downloaded) }
            }
            (Some(kind), _) => Outcome::Failed(*kind),
            _ => Outcome::Failed(ErrorKind::Other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress_line(title: &str, file: &str, downloaded: u64, total: Option<u64>) -> String {
        let total = total.map_or("null".to_string(), |t| t.to_string());
        format!(
            "[orb] {{\"title\":\"{title}\",\"progress\":{{\"status\":\"downloading\",\"downloaded_bytes\":{downloaded},\"total_bytes\":{total},\"filename\":\"{file}\",\"speed\":4200000.5,\"eta\":53,\"elapsed\":1.2,\"ctx_id\":null}}}}"
        )
    }

    #[test]
    fn progress_line_gives_bytes_total_speed_eta() {
        let l = parse_line(&progress_line("Clip", "a.f137.mp4", 190, Some(412)));
        assert_eq!(
            l,
            Line::Progress(RawProgress {
                title: Some("Clip".into()),
                file: Some("a.f137.mp4".into()),
                downloaded: 190,
                total: Some(412),
                speed: Some(4200000.5),
                eta: Some(53),
            })
        );
    }

    #[test]
    fn estimated_total_and_missing_speed_are_handled() {
        let l = parse_line("[orb] {\"title\":\"T\",\"progress\":{\"downloaded_bytes\":10,\"total_bytes_estimate\":99.6,\"speed\":null,\"eta\":null}}");
        let Line::Progress(p) = l else { panic!("{l:?}") };
        assert_eq!((p.downloaded, p.total, p.speed, p.eta), (10, Some(99), None, None));
        assert_eq!(parse_line("[orb] not json"), Line::Other);
    }

    #[test]
    fn destination_lines() {
        assert_eq!(parse_line(r"[download] Destination: C:\D\Clip [x].f137.mp4"), Line::Destination(r"C:\D\Clip [x].f137.mp4".into()));
        assert_eq!(parse_line(r#"[Merger] Merging formats into "C:\D\Clip [x].mp4""#), Line::Destination(r"C:\D\Clip [x].mp4".into()));
        assert_eq!(parse_line(r"[ExtractAudio] Destination: C:\D\Song [y].mp3"), Line::Destination(r"C:\D\Song [y].mp3".into()));
        assert_eq!(parse_line(r"[download] C:\D\Clip [x].mp4 has already been downloaded"), Line::AlreadyDownloaded(r"C:\D\Clip [x].mp4".into()));
        assert_eq!(parse_line("[youtube] x: Downloading webpage"), Line::Other);
    }

    #[test]
    fn error_classifier_from_real_stderr_text() {
        let cases = [
            ("ERROR: [youtube] abc: Private video. Sign in if you've been granted access to this video", ErrorKind::Private),
            ("ERROR: [youtube] abc: Video unavailable. This video has been removed by the uploader", ErrorKind::Removed),
            ("ERROR: [youtube] abc: Video unavailable", ErrorKind::Removed),
            ("ERROR: Unsupported URL: https://example.com/", ErrorKind::Unsupported),
            ("ERROR: unable to write data: [Errno 28] No space left on device", ErrorKind::DiskFull),
            ("ERROR: unable to write data: [WinError 112] There is not enough space on the disk", ErrorKind::DiskFull),
            (
                "ERROR: [youtube] abc: Unable to download API page: <urlopen error [Errno 11001] getaddrinfo failed> (caused by TransportError)",
                ErrorKind::Offline,
            ),
            ("ERROR: [youtube] abc: Failed to resolve 'www.youtube.com' ([Errno 11001] getaddrinfo failed)", ErrorKind::Offline),
            ("ERROR: You have requested merging of multiple formats but ffmpeg is not installed. Aborting due to --abort-on-error", ErrorKind::FfmpegMissing),
            ("ERROR: Postprocessing: ffprobe and ffmpeg not found. Please install or provide the path using --ffmpeg-location", ErrorKind::FfmpegMissing),
            ("ERROR: [generic] Unable to extract something", ErrorKind::Other),
        ];
        for (text, want) in cases {
            assert_eq!(parse_line(text), Line::Error(want), "{text}");
        }
        assert_eq!(parse_line("WARNING: video is private maybe"), Line::Other);
    }

    #[test]
    fn tracker_adds_up_the_video_and_audio_files_of_a_merge() {
        let mut t = Tracker::new();
        assert_eq!(t.feed(&progress_line("Clip", "a.f137.mp4", 0, Some(300)))[0], Update::Title("Clip".into()));
        t.feed(&progress_line("Clip", "a.f137.mp4", 300, Some(300)));
        let u = t.feed(&progress_line("Clip", "a.f140.m4a", 50, Some(100)));
        assert_eq!(u.len(), 1, "title is only reported once");
        assert_eq!(t.progress().downloaded, 350);
        assert_eq!(t.progress().total, Some(400));
    }

    #[test]
    fn tracker_outcome_done_uses_last_destination() {
        let mut t = Tracker::new();
        t.feed(r"[download] Destination: C:\D\a.f137.mp4");
        t.feed(&progress_line("Clip", "a.f137.mp4", 300, Some(300)));
        t.feed(r#"[Merger] Merging formats into "C:\D\a.mp4""#);
        assert_eq!(t.finish(true, |_| Some(412)), Outcome::Done { path: r"C:\D\a.mp4".into(), size: 412 });
        assert_eq!(t.finish(true, |_| None), Outcome::Done { path: r"C:\D\a.mp4".into(), size: 300 });
    }

    #[test]
    fn tracker_outcome_failed_keeps_the_first_error() {
        let mut t = Tracker::new();
        t.feed("ERROR: unable to write data: [Errno 28] No space left on device");
        t.feed("ERROR: something after");
        assert_eq!(t.finish(false, |_| None), Outcome::Failed(ErrorKind::DiskFull));
        assert_eq!(Tracker::new().finish(false, |_| None), Outcome::Failed(ErrorKind::Other));
        assert_eq!(Tracker::new().finish(true, |_| None), Outcome::Failed(ErrorKind::Other), "success without a file is not done");
    }

    #[test]
    fn error_text_is_the_first_error_line_else_the_last_warnings() {
        let mut t = Tracker::new();
        assert_eq!(t.error_text(), None);
        t.feed("WARNING: [youtube] No supported JavaScript runtime could be found.");
        t.feed("WARNING: [youtube] abc: Some formats may be missing");
        t.feed("WARNING: [youtube] abc: nsig extraction failed");
        assert_eq!(
            t.error_text().as_deref(),
            Some("WARNING: [youtube] abc: Some formats may be missing | WARNING: [youtube] abc: nsig extraction failed"),
            "only the last two warnings"
        );
        t.feed("ERROR: [youtube] abc: Requested format is not available.  \r");
        t.feed("ERROR: later consequence");
        assert_eq!(t.error_text().as_deref(), Some("ERROR: [youtube] abc: Requested format is not available."));
    }

    #[test]
    fn error_text_is_capped() {
        let mut t = Tracker::new();
        t.feed(&format!("ERROR: {}", "é".repeat(1000)));
        let text = t.error_text().unwrap();
        assert_eq!(text.chars().count(), ERROR_TEXT_MAX + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn already_downloaded_is_done_but_not_reported_as_ours() {
        let mut t = Tracker::new();
        assert!(t.feed(r"[download] C:\D\a.mp4 has already been downloaded").is_empty());
        assert_eq!(t.finish(true, |_| Some(5)), Outcome::Done { path: r"C:\D\a.mp4".into(), size: 5 });
    }
}
