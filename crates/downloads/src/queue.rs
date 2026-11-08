//! The download queue: who waits, who runs (at most two), and the orb ring's progress.
//!
//! The queue never spawns or kills anything. It says which jobs to start ([`Queue::start_ready`])
//! and what to stop or delete ([`Queue::cancel`]); the caller does the I/O with [`crate::runner`]
//! and feeds the runner's [`Update`]s back through [`Queue::apply`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::args::Request;
use crate::format::Format;
use crate::link::MediaLink;
use crate::parse::{ErrorKind, Outcome, Progress, Update};

/// "2 at a time" in the panel.
pub const MAX_RUNNING: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(u64);

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Waiting,
    /// Started, no size yet ("finding formats").
    Reading,
    Running(Progress),
    /// The process is stopped and the part file kept.
    Paused { downloaded: u64, total: Option<u64> },
    Done { path: PathBuf, size: u64, at: SystemTime },
    /// Progress is kept so disk full can hold its ring and offline can show "34%".
    Failed { kind: ErrorKind, downloaded: u64, total: Option<u64> },
}

impl JobState {
    /// A process is (or is about to be) running for this job.
    pub fn is_active(&self) -> bool {
        matches!(self, JobState::Reading | JobState::Running(_))
    }

    /// Still to finish: counts toward the orb ring and keeps the current batch open.
    fn is_pending(&self) -> bool {
        match self {
            JobState::Waiting | JobState::Reading | JobState::Running(_) | JobState::Paused { .. } => true,
            JobState::Failed { kind, .. } => kind.keeps_progress(),
            JobState::Done { .. } => false,
        }
    }

    /// (bytes so far, size) for the ring. An unknown size counts as what is already there.
    fn bytes(&self) -> (u64, u64) {
        match self {
            JobState::Waiting | JobState::Reading => (0, 0),
            JobState::Running(p) => (p.downloaded, p.total.unwrap_or(p.downloaded).max(p.downloaded)),
            JobState::Paused { downloaded, total } | JobState::Failed { downloaded, total, .. } => {
                (*downloaded, total.unwrap_or(*downloaded).max(*downloaded))
            }
            JobState::Done { size, .. } => (*size, *size),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub request: Request,
    pub title: Option<String>,
    pub state: JobState,
    /// Files this job's runs created, for cancel to delete.
    pub files: Vec<PathBuf>,
    /// yt-dlp's words for the last failure, for the log.
    pub error_text: Option<String>,
    /// This run already failed once and went on by itself ([`Queue::apply`]).
    auto_retried: bool,
    batch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    NotFound,
    /// The action does not apply in the job's current state (for example pausing a done job).
    NotAllowed,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            QueueError::NotFound => "no such download",
            QueueError::NotAllowed => "not possible in this state",
        })
    }
}

impl std::error::Error for QueueError {}

/// What the caller must do after a cancel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cancelled {
    /// Stop the job's running process first.
    pub stop_process: bool,
    /// Delete these with [`crate::runner::delete_partial`].
    pub delete: Vec<PathBuf>,
}

/// The whole current batch by size, as the orb ring and its hover pill ("2 of 3 · 46%") show it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overall {
    pub files_done: usize,
    pub files: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

impl Overall {
    /// `None` while no size is known yet.
    pub fn fraction(&self) -> Option<f64> {
        (self.bytes_total > 0).then(|| (self.bytes_done as f64 / self.bytes_total as f64).min(1.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ink,
    /// Paused or offline: the ring holds still in grey.
    Grey,
    /// Needs you (disk full).
    Ember,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ring {
    /// Nothing left to do in this batch.
    Hidden,
    /// Running, but no size known yet: the short travelling arc.
    Reading,
    Progress { fraction: f64, tone: Tone },
}

/// The panel title's count, for example "2 running · 1 waiting".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub running: usize,
    pub waiting: usize,
    pub paused: usize,
    pub failed: usize,
    pub done: usize,
}

#[derive(Debug, Default)]
pub struct Queue {
    jobs: Vec<Job>,
    next_id: u64,
    batch: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Adds a job in Waiting. A job added while nothing is pending starts a new ring batch.
    pub fn add(&mut self, link: MediaLink, format: Format, folder: PathBuf) -> JobId {
        if !self.jobs.iter().any(|j| j.state.is_pending()) {
            self.batch += 1;
        }
        let id = JobId(self.next_id);
        self.next_id += 1;
        self.jobs.push(Job { id, request: Request { link, format, folder }, title: None, state: JobState::Waiting, files: Vec::new(), error_text: None, auto_retried: false, batch: self.batch });
        id
    }

    /// The finished download of this link, for the "Already saved" pill.
    pub fn saved(&self, link: &MediaLink) -> Option<&Job> {
        self.jobs.iter().find(|j| j.request.link == *link && matches!(j.state, JobState::Done { .. }))
    }

    /// Moves waiting jobs to Reading, oldest first, while fewer than [`MAX_RUNNING`] are active.
    /// The caller starts a runner for each returned job.
    pub fn start_ready(&mut self) -> Vec<(JobId, Request)> {
        let mut free = MAX_RUNNING.saturating_sub(self.jobs.iter().filter(|j| j.state.is_active()).count());
        let mut started = Vec::new();
        for job in self.jobs.iter_mut().filter(|j| j.state == JobState::Waiting) {
            if free == 0 {
                break;
            }
            job.state = JobState::Reading;
            job.auto_retried = false;
            started.push((job.id, job.request.clone()));
            free -= 1;
        }
        started
    }

    /// Feeds one runner update. Updates for jobs that are no longer active (paused, cancelled,
    /// or already finished) are ignored, since a stopped process can still flush a few lines.
    ///
    /// Returns true when the run failed for no reason we can name ([`ErrorKind::Other`], often a
    /// passing YouTube hiccup) for the first time since it started: the job stays active, holding
    /// its slot and progress, and the caller starts its process again after a short pause. Only a
    /// second failure in a row shows as Failed.
    pub fn apply(&mut self, id: JobId, update: Update, now: SystemTime) -> bool {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id && j.state.is_active()) else { return false };
        match update {
            Update::Title(t) => job.title = Some(t),
            Update::Progress(p) => job.state = JobState::Running(p),
            Update::Destination(p) => {
                if !job.files.contains(&p) {
                    job.files.push(p);
                }
            }
            Update::ErrorText(t) => job.error_text = Some(t),
            Update::Finished(Outcome::Done { path, size }) => job.state = JobState::Done { path, size, at: now },
            Update::Finished(Outcome::Failed(ErrorKind::Other)) if !job.auto_retried => {
                job.auto_retried = true;
                if let JobState::Running(p) = &mut job.state {
                    // Nothing is moving until the process is back.
                    p.speed = None;
                    p.eta = None;
                }
                return true;
            }
            Update::Finished(Outcome::Failed(kind)) => {
                let (downloaded, total) = progress_of(&job.state);
                job.state = JobState::Failed { kind, downloaded, total };
            }
        }
        false
    }

    /// Running or reading to Paused. The caller stops the process; the part file stays.
    pub fn pause(&mut self, id: JobId) -> Result<(), QueueError> {
        let job = self.job_mut(id)?;
        if !job.state.is_active() {
            return Err(QueueError::NotAllowed);
        }
        let (downloaded, total) = progress_of(&job.state);
        job.state = JobState::Paused { downloaded, total };
        Ok(())
    }

    /// Paused back to Waiting; it starts when a slot is free. yt-dlp continues the part file.
    pub fn resume(&mut self, id: JobId) -> Result<(), QueueError> {
        let job = self.job_mut(id)?;
        if !matches!(job.state, JobState::Paused { .. }) {
            return Err(QueueError::NotAllowed);
        }
        job.state = JobState::Waiting;
        Ok(())
    }

    /// Failed back to Waiting.
    pub fn retry(&mut self, id: JobId) -> Result<(), QueueError> {
        let job = self.job_mut(id)?;
        if !matches!(job.state, JobState::Failed { .. }) {
            return Err(QueueError::NotAllowed);
        }
        job.state = JobState::Waiting;
        Ok(())
    }

    /// The internet is back: every offline job goes on by itself. Returns how many.
    pub fn retry_offline(&mut self) -> usize {
        let mut n = 0;
        for job in self.jobs.iter_mut().filter(|j| matches!(j.state, JobState::Failed { kind: ErrorKind::Offline, .. })) {
            job.state = JobState::Waiting;
            n += 1;
        }
        n
    }

    /// Removes any job that is not done. The caller stops its process if asked and deletes the
    /// listed files.
    pub fn cancel(&mut self, id: JobId) -> Result<Cancelled, QueueError> {
        let i = self.jobs.iter().position(|j| j.id == id).ok_or(QueueError::NotFound)?;
        if matches!(self.jobs[i].state, JobState::Done { .. }) {
            return Err(QueueError::NotAllowed);
        }
        let job = self.jobs.remove(i);
        Ok(Cancelled { stop_process: job.state.is_active(), delete: job.files })
    }

    /// The waiting row's format chip.
    pub fn set_format(&mut self, id: JobId, format: Format) -> Result<(), QueueError> {
        let job = self.job_mut(id)?;
        if job.state != JobState::Waiting {
            return Err(QueueError::NotAllowed);
        }
        job.request.format = format;
        Ok(())
    }

    /// "Change folder" (disk full). Any job without a running process may move.
    pub fn set_folder(&mut self, id: JobId, folder: &Path) -> Result<(), QueueError> {
        let job = self.job_mut(id)?;
        if job.state.is_active() || matches!(job.state, JobState::Done { .. }) {
            return Err(QueueError::NotAllowed);
        }
        job.request.folder = folder.to_path_buf();
        Ok(())
    }

    /// "Clear list" under Done today.
    pub fn clear_done(&mut self) {
        self.jobs.retain(|j| !matches!(j.state, JobState::Done { .. }));
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for j in &self.jobs {
            match j.state {
                JobState::Reading | JobState::Running(_) => c.running += 1,
                JobState::Waiting => c.waiting += 1,
                JobState::Paused { .. } => c.paused += 1,
                JobState::Failed { .. } => c.failed += 1,
                JobState::Done { .. } => c.done += 1,
            }
        }
        c
    }

    /// The current batch: jobs added since the queue last had nothing pending. Failures that end
    /// a job (private, removed, and so on) leave the batch.
    pub fn overall(&self) -> Overall {
        let mut o = Overall::default();
        for j in self.batch_jobs() {
            let (done, total) = j.state.bytes();
            o.files += 1;
            o.files_done += usize::from(matches!(j.state, JobState::Done { .. }));
            o.bytes_done += done;
            o.bytes_total += total;
        }
        o
    }

    pub fn ring(&self) -> Ring {
        let batch: Vec<&Job> = self.batch_jobs().collect();
        if !batch.iter().any(|j| j.state.is_pending()) {
            return Ring::Hidden;
        }
        let tone = if batch.iter().any(|j| matches!(j.state, JobState::Failed { kind: ErrorKind::DiskFull, .. })) {
            Tone::Ember
        } else if batch.iter().any(|j| j.state.is_active()) {
            Tone::Ink
        } else {
            Tone::Grey
        };
        match (self.overall().fraction(), tone) {
            (Some(fraction), _) => Ring::Progress { fraction, tone },
            (None, Tone::Ink) => Ring::Reading,
            (None, tone) => Ring::Progress { fraction: 0.0, tone },
        }
    }

    fn batch_jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.iter().filter(move |j| j.batch == self.batch && (j.state.is_pending() || matches!(j.state, JobState::Done { .. })))
    }

    fn job_mut(&mut self, id: JobId) -> Result<&mut Job, QueueError> {
        self.jobs.iter_mut().find(|j| j.id == id).ok_or(QueueError::NotFound)
    }
}

fn progress_of(state: &JobState) -> (u64, Option<u64>) {
    match state {
        JobState::Running(p) => (p.downloaded, p.total),
        JobState::Paused { downloaded, total } | JobState::Failed { downloaded, total, .. } => (*downloaded, *total),
        _ => (0, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MB: u64 = 1_000_000;

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_000_000)
    }

    fn link(n: u8) -> MediaLink {
        MediaLink::parse(&format!("https://youtu.be/vid0000000{}", n % 10)).unwrap()
    }

    fn add(q: &mut Queue, n: u8) -> JobId {
        q.add(link(n), Format::default(), PathBuf::from(r"C:\D"))
    }

    fn running(downloaded: u64, total: u64) -> Update {
        Update::Progress(Progress { downloaded, total: Some(total), speed: Some(4.2 * MB as f64), eta: Some(53) })
    }

    fn done(size: u64) -> Update {
        Update::Finished(Outcome::Done { path: PathBuf::from(r"C:\D\a.mp4"), size })
    }

    #[test]
    fn at_most_two_run_and_the_next_starts_when_one_finishes() {
        let mut q = Queue::new();
        let (a, b, c) = (add(&mut q, 1), add(&mut q, 2), add(&mut q, 3));
        let started: Vec<JobId> = q.start_ready().into_iter().map(|(id, _)| id).collect();
        assert_eq!(started, vec![a, b]);
        assert!(q.start_ready().is_empty());
        assert_eq!(q.counts(), Counts { running: 2, waiting: 1, ..Counts::default() });
        q.apply(a, done(10), t0());
        let started: Vec<JobId> = q.start_ready().into_iter().map(|(id, _)| id).collect();
        assert_eq!(started, vec![c]);
    }

    #[test]
    fn reading_then_running_then_done_with_the_injected_time() {
        let mut q = Queue::new();
        let a = add(&mut q, 1);
        q.start_ready();
        assert_eq!(q.get(a).unwrap().state, JobState::Reading);
        q.apply(a, Update::Title("Clip".into()), t0());
        q.apply(a, running(190 * MB, 412 * MB), t0());
        let JobState::Running(p) = &q.get(a).unwrap().state else { panic!() };
        assert_eq!((p.downloaded, p.total, p.eta), (190 * MB, Some(412 * MB), Some(53)));
        q.apply(a, done(412 * MB), t0());
        assert_eq!(q.get(a).unwrap().state, JobState::Done { path: PathBuf::from(r"C:\D\a.mp4"), size: 412 * MB, at: t0() });
        assert_eq!(q.get(a).unwrap().title.as_deref(), Some("Clip"));
    }

    #[test]
    fn pause_keeps_progress_resume_waits_for_a_slot_and_late_lines_are_ignored() {
        let mut q = Queue::new();
        let (a, b) = (add(&mut q, 1), add(&mut q, 2));
        q.start_ready();
        q.apply(a, running(120 * MB, 300 * MB), t0());
        q.pause(a).unwrap();
        assert_eq!(q.get(a).unwrap().state, JobState::Paused { downloaded: 120 * MB, total: Some(300 * MB) });
        // The stopped process still flushes a line and then exits with an error.
        q.apply(a, running(121 * MB, 300 * MB), t0());
        q.apply(a, Update::Finished(Outcome::Failed(ErrorKind::Other)), t0());
        assert!(matches!(q.get(a).unwrap().state, JobState::Paused { .. }));

        let c = add(&mut q, 3);
        assert_eq!(q.start_ready().len(), 1, "c takes the freed slot");
        q.resume(a).unwrap();
        assert!(q.start_ready().is_empty(), "b and c hold both slots");
        q.apply(c, done(1), t0());
        assert_eq!(q.start_ready()[0].0, a);
        assert!(q.get(b).unwrap().state.is_active());
        assert_eq!(q.resume(b), Err(QueueError::NotAllowed));
        assert_eq!(q.pause(c), Err(QueueError::NotAllowed));
    }

    #[test]
    fn cancel_stops_running_jobs_and_lists_their_files_but_never_done_ones() {
        let mut q = Queue::new();
        let (a, b) = (add(&mut q, 1), add(&mut q, 2));
        q.start_ready();
        q.apply(a, Update::Destination(PathBuf::from(r"C:\D\a.f137.mp4")), t0());
        q.apply(a, Update::Destination(PathBuf::from(r"C:\D\a.f137.mp4")), t0());
        assert_eq!(q.cancel(a), Ok(Cancelled { stop_process: true, delete: vec![PathBuf::from(r"C:\D\a.f137.mp4")] }));
        assert!(q.get(a).is_none());
        q.apply(b, done(5), t0());
        assert_eq!(q.cancel(b), Err(QueueError::NotAllowed));
        let w = add(&mut q, 3);
        q.pause(w).unwrap_err();
        assert_eq!(q.cancel(w), Ok(Cancelled { stop_process: false, delete: vec![] }));
        assert_eq!(q.cancel(w), Err(QueueError::NotFound));
    }

    #[test]
    fn failure_keeps_progress_and_retry_requeues() {
        let mut q = Queue::new();
        let a = add(&mut q, 1);
        q.start_ready();
        q.apply(a, running(250 * MB, 412 * MB), t0());
        q.apply(a, Update::Finished(Outcome::Failed(ErrorKind::DiskFull)), t0());
        assert_eq!(q.get(a).unwrap().state, JobState::Failed { kind: ErrorKind::DiskFull, downloaded: 250 * MB, total: Some(412 * MB) });
        q.set_folder(a, Path::new(r"E:\More")).unwrap();
        q.retry(a).unwrap();
        let started = q.start_ready();
        assert_eq!(started[0].1.folder, PathBuf::from(r"E:\More"));
        assert_eq!(q.retry(a), Err(QueueError::NotAllowed));
        assert_eq!(q.set_folder(a, Path::new(r"F:\")), Err(QueueError::NotAllowed));
    }

    fn failed(kind: ErrorKind) -> Update {
        Update::Finished(Outcome::Failed(kind))
    }

    #[test]
    fn an_unnamed_failure_goes_on_once_by_itself_and_shows_only_the_second_time() {
        let mut q = Queue::new();
        let (a, b) = (add(&mut q, 1), add(&mut q, 2));
        q.start_ready();
        q.apply(a, running(100 * MB, 400 * MB), t0());
        q.apply(a, Update::ErrorText("ERROR: [youtube] a: HTTP Error 403: Forbidden".into()), t0());
        assert!(q.apply(a, failed(ErrorKind::Other), t0()), "first failure: start it again");
        let JobState::Running(p) = q.get(a).unwrap().state else { panic!("{:?}", q.get(a).unwrap().state) };
        assert_eq!((p.downloaded, p.speed, p.eta), (100 * MB, None, None), "keeps its progress, nothing moving");
        assert_eq!(q.get(a).unwrap().error_text.as_deref(), Some("ERROR: [youtube] a: HTTP Error 403: Forbidden"));
        let c = add(&mut q, 3);
        assert!(q.start_ready().is_empty(), "a keeps its slot while it waits to go again");

        q.apply(a, Update::ErrorText("ERROR: again".into()), t0());
        assert!(!q.apply(a, failed(ErrorKind::Other), t0()), "second failure in a row shows");
        assert_eq!(q.get(a).unwrap().state, JobState::Failed { kind: ErrorKind::Other, downloaded: 100 * MB, total: Some(400 * MB) });
        assert_eq!(q.get(a).unwrap().error_text.as_deref(), Some("ERROR: again"));
        assert_eq!(q.start_ready()[0].0, c);

        // Retry by hand earns another silent try.
        q.apply(b, done(1), t0());
        q.retry(a).unwrap();
        q.start_ready();
        assert!(q.apply(a, failed(ErrorKind::Other), t0()));
        assert!(q.get(a).unwrap().state.is_active());
    }

    #[test]
    fn named_failures_show_at_once() {
        for kind in [ErrorKind::Private, ErrorKind::Removed, ErrorKind::Unsupported, ErrorKind::DiskFull, ErrorKind::FfmpegMissing, ErrorKind::Offline] {
            let mut q = Queue::new();
            let a = add(&mut q, 1);
            q.start_ready();
            assert!(!q.apply(a, failed(kind), t0()), "{kind:?}");
            assert!(matches!(q.get(a).unwrap().state, JobState::Failed { kind: k, .. } if k == kind));
        }
        let mut q = Queue::new();
        let a = add(&mut q, 1);
        q.start_ready();
        q.pause(a).unwrap();
        assert!(!q.apply(a, failed(ErrorKind::Other), t0()), "a paused job's killed process is not retried");
    }

    #[test]
    fn offline_jobs_go_on_by_themselves() {
        let mut q = Queue::new();
        let (a, b) = (add(&mut q, 1), add(&mut q, 2));
        q.start_ready();
        q.apply(a, Update::Finished(Outcome::Failed(ErrorKind::Offline)), t0());
        q.apply(b, Update::Finished(Outcome::Failed(ErrorKind::Private)), t0());
        assert_eq!(q.retry_offline(), 1);
        assert_eq!(q.get(a).unwrap().state, JobState::Waiting);
        assert!(matches!(q.get(b).unwrap().state, JobState::Failed { kind: ErrorKind::Private, .. }));
    }

    #[test]
    fn format_chip_changes_only_while_waiting() {
        let mut q = Queue::new();
        let (a, _, c) = (add(&mut q, 1), add(&mut q, 2), add(&mut q, 3));
        q.start_ready();
        let p720 = Format::Video { max_height: Some(720) };
        assert_eq!(q.set_format(c, p720), Ok(()));
        assert_eq!(q.get(c).unwrap().request.format, p720);
        assert_eq!(q.set_format(a, p720), Err(QueueError::NotAllowed));
    }

    #[test]
    fn already_saved_finds_the_done_job_for_a_recopied_link() {
        let mut q = Queue::new();
        let a = add(&mut q, 1);
        assert!(q.saved(&link(1)).is_none());
        q.start_ready();
        q.apply(a, done(7), t0());
        assert_eq!(q.saved(&link(1)).map(|j| j.id), Some(a));
        assert!(q.saved(&link(2)).is_none());
        q.clear_done();
        assert!(q.jobs().is_empty());
    }

    #[test]
    fn overall_ring_is_the_whole_queue_by_size() {
        let mut q = Queue::new();
        assert_eq!(q.ring(), Ring::Hidden);
        let (a, b, c) = (add(&mut q, 1), add(&mut q, 2), add(&mut q, 3));
        q.start_ready();
        assert_eq!(q.ring(), Ring::Reading, "no size yet");
        q.apply(a, done(100 * MB), t0());
        q.apply(b, running(30 * MB, 100 * MB), t0());
        q.start_ready();
        q.apply(c, running(0, 50 * MB), t0());
        let o = q.overall();
        assert_eq!((o.files_done, o.files, o.bytes_done, o.bytes_total), (1, 3, 130 * MB, 250 * MB));
        assert_eq!(q.ring(), Ring::Progress { fraction: 0.52, tone: Tone::Ink });

        q.pause(b).unwrap();
        q.pause(c).unwrap();
        assert!(matches!(q.ring(), Ring::Progress { tone: Tone::Grey, .. }), "paused holds still in grey");
        q.resume(b).unwrap();
        q.resume(c).unwrap();
        q.start_ready();
        q.apply(b, Update::Finished(Outcome::Failed(ErrorKind::DiskFull)), t0());
        assert!(matches!(q.ring(), Ring::Progress { tone: Tone::Ember, .. }), "disk full needs you");

        q.cancel(b).unwrap();
        q.apply(c, done(50 * MB), t0());
        assert_eq!(q.overall().fraction(), Some(1.0));
        assert_eq!(q.ring(), Ring::Hidden, "the batch is finished");

        // The next link starts a fresh batch; earlier done jobs no longer count.
        let d = add(&mut q, 4);
        q.start_ready();
        q.apply(d, running(10, 40), t0());
        assert_eq!(q.overall(), Overall { files_done: 0, files: 1, bytes_done: 10, bytes_total: 40 });
    }

    #[test]
    fn ending_failures_leave_the_ring() {
        let mut q = Queue::new();
        let (a, b) = (add(&mut q, 1), add(&mut q, 2));
        q.start_ready();
        q.apply(a, Update::Finished(Outcome::Failed(ErrorKind::Removed)), t0());
        q.apply(b, running(1, 4), t0());
        assert_eq!(q.overall().files, 1);
        assert_eq!(q.ring(), Ring::Progress { fraction: 0.25, tone: Tone::Ink });
    }
}
