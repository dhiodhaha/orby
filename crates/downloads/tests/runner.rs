//! End to end: the real runner spawns the fake yt-dlp (src/bin/fake-yt-dlp.rs) as a child
//! process and the queue consumes what it reports. Files go under target/tmp only.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use app_downloads::{
    delete_partial, start, Cancelled, ErrorKind, Format, JobId, JobState, MediaLink, Outcome, Queue, Request, Ring, Sidecar, Update,
    MAX_RUNNING,
};

fn sidecar() -> Sidecar {
    Sidecar { yt_dlp: PathBuf::from(env!("CARGO_BIN_EXE_fake-yt-dlp")), ffmpeg: PathBuf::from(r"C:\sidecar\ffmpeg.exe") }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("downloads-tests").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn link(id: &str) -> MediaLink {
    MediaLink::parse(&format!("https://youtu.be/{id}")).unwrap()
}

fn request(id: &str, folder: &Path) -> Request {
    Request { link: link(id), format: Format::default(), folder: folder.to_path_buf() }
}

/// Runs one download to the end and returns every update in order.
fn run(req: &Request) -> Vec<Update> {
    let (tx, rx) = mpsc::channel();
    start(&sidecar(), req, move |u| tx.send(u).unwrap()).unwrap();
    let mut updates = Vec::new();
    loop {
        let u = rx.recv_timeout(Duration::from_secs(20)).expect("runner went quiet");
        let finished = matches!(u, Update::Finished(_));
        updates.push(u);
        if finished {
            return updates;
        }
    }
}

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_000_000)
}

#[test]
fn success_downloads_both_streams_merges_and_reports_done() {
    let dir = temp_dir("success");
    let req = request("success0001", &dir);
    let updates = run(&req);

    let merged = dir.join("Sample clip [success0001].mp4");
    assert_eq!(updates.last(), Some(&Update::Finished(Outcome::Done { path: merged.clone(), size: 50_000 })));
    assert_eq!(fs::metadata(&merged).unwrap().len(), 50_000);
    assert!(!dir.join("Sample clip [success0001].f137.mp4").exists(), "merge deletes the streams");
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::Title(t) if t == "Sample clip")).count(), 1);
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::Destination(_))).count(), 3);

    let progress: Vec<_> = updates.iter().filter_map(|u| if let Update::Progress(p) = u { Some(*p) } else { None }).collect();
    assert!(progress.len() >= 10);
    assert!(progress.windows(2).all(|w| w[0].downloaded <= w[1].downloaded), "job progress never goes back");
    let last = progress.last().unwrap();
    assert_eq!((last.downloaded, last.total), (50_000, Some(50_000)));
    assert!(progress.iter().all(|p| p.speed.is_some() && p.eta.is_some()));

    let mut q = Queue::new();
    let id = q.add(req.link.clone(), req.format, dir.clone());
    q.start_ready();
    for u in updates {
        q.apply(id, u, now());
    }
    assert_eq!(q.get(id).unwrap().state, JobState::Done { path: merged, size: 50_000, at: now() });
    assert_eq!(q.get(id).unwrap().title.as_deref(), Some("Sample clip"));
    assert!(q.saved(&link("success0001")).is_some(), "copying it again says Already saved");
}

#[test]
fn private_video_fails_as_private_without_progress() {
    let dir = temp_dir("private");
    let updates = run(&request("private0001", &dir));
    assert_eq!(updates, vec![Update::Finished(Outcome::Failed(ErrorKind::Private))]);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}

#[test]
fn disk_full_fails_and_keeps_its_progress_and_part_file() {
    let dir = temp_dir("diskfull");
    let req = request("diskfull001", &dir);
    let updates = run(&req);
    assert_eq!(updates.last(), Some(&Update::Finished(Outcome::Failed(ErrorKind::DiskFull))));

    let mut q = Queue::new();
    let id = q.add(req.link.clone(), req.format, dir.clone());
    q.start_ready();
    for u in updates {
        q.apply(id, u, now());
    }
    assert_eq!(q.get(id).unwrap().state, JobState::Failed { kind: ErrorKind::DiskFull, downloaded: 25_000, total: Some(41_200) });
    assert!(matches!(q.ring(), Ring::Progress { tone: app_downloads::Tone::Ember, .. }));
    assert!(dir.join("Sample clip [diskfull001].mp4.part").exists());
}

#[test]
fn pause_stops_the_process_and_keeps_the_part_file_then_cancel_deletes_it() {
    let dir = temp_dir("pause");
    let mut q = Queue::new();
    let id = q.add(link("slowslow001"), Format::default(), dir.clone());
    let (_, req) = q.start_ready().remove(0);

    let (tx, rx) = mpsc::channel();
    let download = start(&sidecar(), &req, move |u| tx.send(u).unwrap()).unwrap();
    let started = Instant::now();
    loop {
        let u = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let first_progress = matches!(u, Update::Progress(_));
        q.apply(id, u, now());
        if first_progress {
            break;
        }
    }
    q.pause(id).unwrap();
    download.stop().unwrap();
    loop {
        let u = rx.recv_timeout(Duration::from_secs(10)).expect("stopped process reports Finished");
        let finished = matches!(u, Update::Finished(_));
        q.apply(id, u, now());
        if finished {
            break;
        }
    }
    assert!(started.elapsed() < Duration::from_secs(10), "stop ended the 15 s download early");
    let JobState::Paused { downloaded, total } = q.get(id).unwrap().state else { panic!("{:?}", q.get(id).unwrap().state) };
    assert!(downloaded > 0 && downloaded < 100_000);
    assert_eq!(total, Some(100_000));
    let part = dir.join("Sample clip [slowslow001].mp4.part");
    assert!(part.exists(), "pause keeps the part file");

    let Cancelled { stop_process, delete } = q.cancel(id).unwrap();
    assert!(!stop_process, "already stopped by pause");
    assert_eq!(delete, vec![dir.join("Sample clip [slowslow001].mp4")]);
    delete_partial(&delete).unwrap();
    assert!(!part.exists(), "cancel deletes the part file");
    assert!(q.jobs().is_empty());
}

#[test]
fn a_queue_of_three_runs_at_most_two_at_once_and_finishes_all() {
    let root = temp_dir("queue");
    let mut q = Queue::new();
    let ids: Vec<JobId> = (0..3)
        .map(|i| {
            let folder = root.join(format!("job{i}"));
            fs::create_dir_all(&folder).unwrap();
            q.add(link("success0001"), Format::default(), folder)
        })
        .collect();
    let (tx, rx) = mpsc::channel::<(JobId, Update)>();
    let mut most_running = 0;
    let mut finished = 0;
    while finished < ids.len() {
        for (id, req) in q.start_ready() {
            let tx = tx.clone();
            start(&sidecar(), &req, move |u| tx.send((id, u)).unwrap()).unwrap();
        }
        most_running = most_running.max(q.counts().running);
        let (id, u) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        finished += usize::from(matches!(u, Update::Finished(_)));
        q.apply(id, u, now());
    }
    assert_eq!(most_running, MAX_RUNNING);
    for id in ids {
        assert!(matches!(q.get(id).unwrap().state, JobState::Done { size: 50_000, .. }));
    }
    let o = q.overall();
    assert_eq!((o.files_done, o.files, o.bytes_done), (3, 3, 150_000));
    assert_eq!(o.fraction(), Some(1.0));
    assert_eq!(q.ring(), Ring::Hidden);
}

#[test]
fn a_missing_sidecar_fails_to_start() {
    let missing = Sidecar { yt_dlp: temp_dir("missing").join("yt-dlp.exe"), ffmpeg: PathBuf::from("ffmpeg.exe") };
    let err = start(&missing, &request("success0001", Path::new(".")), |_| {}).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
