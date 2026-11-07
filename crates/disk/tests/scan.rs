#![cfg(windows)]
use app_disk::{drive_space, scan, Category, NodeKind, ScanError, ScanOptions, ScanStatus, SkipReason, Tree};
use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scan").join(name);
    if dir.exists() {
        // A crashed earlier run may have left a deny rule behind.
        let _ = Command::new("icacls").arg(&dir).args(["/reset", "/T", "/Q"]).output();
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes]).unwrap();
}

fn run(root: &Path) -> Tree {
    scan(root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap()
}

fn names(t: &Tree, ids: impl IntoIterator<Item = app_disk::NodeId>) -> Vec<String> {
    ids.into_iter().map(|id| t.node(id).name().to_string_lossy().into_owned()).collect()
}

/// a.txt 100, videos/{movie.mp4 5000, clip.mkv 3000}, docs/{report.pdf 700, nested/deep.zip 2000}, empty/
fn sample(root: &Path) {
    write(&root.join("a.txt"), 100);
    write(&root.join("videos/movie.mp4"), 5000);
    write(&root.join("videos/clip.mkv"), 3000);
    write(&root.join("docs/report.pdf"), 700);
    write(&root.join("docs/nested/deep.zip"), 2000);
    fs::create_dir_all(root.join("empty")).unwrap();
}

#[test]
fn totals_counts_and_children_by_size() {
    let root = fresh("totals");
    sample(&root);
    let t = run(&root);
    let r = t.node(t.root());
    assert_eq!(t.status(), ScanStatus::Complete);
    assert!(t.skipped().is_empty());
    assert_eq!(r.size(), 10_800);
    assert_eq!(r.files(), 5);
    assert_eq!(r.dirs(), 4);
    assert!(r.allocated() >= r.size(), "cluster rounding never shrinks a file");

    let rows = t.children_by_size(t.root());
    assert_eq!(names(&t, rows.iter().map(|c| c.id)), ["videos", "docs", "a.txt", "empty"]);
    assert_eq!(rows.iter().map(|c| c.size).collect::<Vec<_>>(), [8000, 2700, 100, 0]);
    assert!((rows[0].percent_of_parent - 8000.0 * 100.0 / 10_800.0).abs() < 1e-9);
    let total: f64 = rows.iter().map(|c| c.percent_of_parent).sum();
    assert!((total - 100.0).abs() < 1e-9);
    assert!(t.node(rows[0].id).is_dir());
    assert!(!t.node(rows[2].id).is_dir());
}

#[test]
fn largest_files_and_type_breakdown() {
    let root = fresh("types");
    sample(&root);
    let t = run(&root);
    assert_eq!(names(&t, t.largest_files(t.root(), 3)), ["movie.mp4", "clip.mkv", "deep.zip"]);
    assert_eq!(t.largest_files(t.root(), 50).len(), 5);
    let docs = t.find(&root.join("docs")).unwrap();
    assert_eq!(names(&t, t.largest_files(docs, 10)), ["deep.zip", "report.pdf"]);

    let types = t.type_breakdown(t.root());
    let got: Vec<_> = types.iter().map(|s| (s.category, s.bytes, s.files)).collect();
    assert_eq!(
        got,
        [
            (Category::Video, 8000, 2),
            (Category::Archives, 2000, 1),
            (Category::Documents, 800, 2),
            (Category::AppsAndGames, 0, 0),
            (Category::Images, 0, 0),
            (Category::Other, 0, 0),
        ]
    );
}

#[test]
fn paths_round_trip_and_folder_dates_are_newest_in_subtree() {
    let root = fresh("paths");
    sample(&root);
    let newest = SystemTime::now() + Duration::from_secs(3 * 86_400);
    File::options().write(true).open(root.join("docs/nested/deep.zip")).unwrap().set_times(FileTimes::new().set_modified(newest)).unwrap();
    let t = run(&root);
    let deep = t.find(&root.join("docs/nested/deep.zip")).unwrap();
    assert_eq!(t.path(deep), root.join("docs").join("nested").join("deep.zip"));
    assert_eq!(names(&t, t.ancestry(deep).into_iter().skip(1)), ["docs", "nested", "deep.zip"]);
    assert_eq!(t.path(t.root()), root);
    let docs = t.find(&root.join("docs")).unwrap();
    assert_eq!(t.node(docs).modified(), Some(newest));
    assert_eq!(t.node(t.root()).modified(), Some(newest));
    assert!(t.find(&root.join("nope")).is_none());
}

#[test]
fn junctions_are_listed_but_not_followed() {
    let root = fresh("junction");
    write(&root.join("real/big.bin"), 10_000);
    let link = root.join("link");
    let out = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(root.join("real")).output().unwrap();
    assert!(out.status.success(), "mklink failed: {out:?}");
    assert!(link.join("big.bin").exists(), "junction resolves");

    let t = run(&root);
    assert_eq!(t.node(t.root()).size(), 10_000, "bytes behind the junction are counted once");
    assert_eq!(t.node(t.root()).files(), 1);
    let l = t.find(&link).unwrap();
    assert_eq!(t.node(l).kind(), NodeKind::Link);
    assert!(t.children(l).is_empty());
    assert_eq!(t.node(l).size(), 0);
}

fn wide_tree(root: &Path, dirs: usize, files_per_dir: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("d{}", d % 17)).join(format!("sub{d}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files_per_dir {
            fs::write(dir.join(format!("f{f}.dat")), vec![1u8; d + f]).unwrap();
        }
    }
}

#[test]
fn many_threads_count_the_same_as_one_and_progress_is_live() {
    let root = fresh("threads");
    wide_tree(&root, 200, 10);
    let one = scan(&root, &ScanOptions { threads: 1, progress_every: 1 }, |_| {}, &AtomicBool::new(false)).unwrap();

    let mut seen = Vec::new();
    let many = scan(
        &root,
        &ScanOptions { threads: 8, progress_every: 1 },
        |p| {
            assert!(p.current.starts_with(&root));
            seen.push(p.tree.node(p.tree.root()).size());
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    for t in [&one, &many] {
        assert_eq!(t.node(t.root()).files(), 2000);
        assert_eq!(t.node(t.root()).dirs(), 217);
    }
    assert_eq!(one.node(one.root()).size(), many.node(many.root()).size());
    assert_eq!(one.len(), many.len());
    assert_eq!(seen.len(), 218, "one call per folder at progress_every = 1");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "counted bytes only grow");
    assert_eq!(*seen.last().unwrap(), many.node(many.root()).size());
}

#[test]
fn stop_is_prompt_and_keeps_what_was_counted() {
    let root = fresh("cancel");
    wide_tree(&root, 300, 5);
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let t = scan(
        &root,
        &ScanOptions { threads: 4, progress_every: 1 },
        |_| {
            calls += 1;
            if calls == 3 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(t.status(), ScanStatus::Stopped);
    let r = t.node(t.root());
    assert!(r.files() < 1500, "stopped before the end, counted {}", r.files());
    assert!(calls < 30, "no more than a few in-flight folders after stop, got {calls}");
    // What is kept is consistent: the root equals the sum of its children.
    let sum: u64 = t.children(t.root()).iter().map(|&c| t.node(c).size()).sum();
    assert_eq!(sum, r.size());

    let pre = scan(&root, &ScanOptions::default(), |_| {}, &AtomicBool::new(true)).unwrap();
    assert_eq!(pre.status(), ScanStatus::Stopped);
    assert_eq!(pre.len(), 1);
}

#[test]
fn access_denied_folders_are_listed_and_the_scan_continues() {
    let root = fresh("denied");
    write(&root.join("open/ok.txt"), 300);
    write(&root.join("private/secret.txt"), 900);
    let private = root.join("private");
    let deny = Command::new("icacls").arg(&private).args(["/deny", "*S-1-1-0:(RD)"]).output().unwrap();
    assert!(deny.status.success(), "{deny:?}");
    let t = run(&root);
    let _ = Command::new("icacls").arg(&private).args(["/remove:d", "*S-1-1-0"]).output();

    assert_eq!(t.status(), ScanStatus::Complete);
    assert_eq!(t.skipped().len(), 1);
    assert_eq!(t.skipped()[0].path, private);
    assert_eq!(t.skipped()[0].reason, SkipReason::AccessDenied);
    assert_eq!(t.node(t.root()).size(), 300);
    assert!(t.find(&private).is_some(), "the folder itself still shows, at zero");
}

#[test]
fn unusable_roots_are_typed_errors() {
    let root = fresh("errors");
    write(&root.join("file.txt"), 1);
    let none = AtomicBool::new(false);
    let opts = ScanOptions::default();
    assert!(matches!(scan(&root.join("missing"), &opts, |_| {}, &none), Err(ScanError::NotFound)));
    assert!(matches!(scan(&root.join("file.txt"), &opts, |_| {}, &none), Err(ScanError::NotADirectory)));
    let free_letter = (b'D'..=b'Z').map(|c| format!("{}:\\", c as char)).find(|d| !Path::new(d).exists());
    if let Some(d) = free_letter {
        let r = scan(Path::new(&d), &opts, |_| {}, &none);
        assert!(matches!(r, Err(ScanError::NotFound | ScanError::NotReady)), "{d}: {r:?}");
    }
}

#[test]
fn drive_space_of_the_target_drive() {
    let s = drive_space(Path::new(env!("CARGO_TARGET_TMPDIR"))).unwrap();
    assert!(s.total > 0);
    assert!(s.free <= s.total);
    assert_eq!(s.used(), s.total - s.free);
}
