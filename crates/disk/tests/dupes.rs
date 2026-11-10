#![cfg(windows)]
//! Duplicate finding on trees made under the test target folder. Nothing here reads the user's
//! own folders, and nothing goes to the Recycle Bin: removal is only checked, never done.

use app_disk::{
    candidates, check_duplicate_removal, drive_kind, find_duplicates, scan, Candidates, DriveKind, DupeOptions, DupeProgress, DupeRemoval,
    DupeReport, DupeStage, Refusal, RuleEnv, ScanOptions, DUPE_EDGE,
};
use std::fs::{self, File, FileTimes};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dupes").join(name);
    if dir.exists() {
        // A crashed earlier run may have left a deny rule behind.
        let _ = Command::new("icacls").arg(&dir).args(["/reset", "/T", "/Q"]).output();
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `len` bytes from a seed, so two seeds differ everywhere.
fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u32).wrapping_mul(31).wrapping_add(u32::from(seed) * 7) as u8).collect()
}

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn opts(root: &Path) -> DupeOptions {
    DupeOptions {
        min_size: 1,
        env: RuleEnv::under(root),
        system_dirs: Vec::new(),
        cache: None,
        threads: Some(2),
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    }
}

fn stage1(root: &Path, o: &DupeOptions) -> Candidates {
    let tree = scan(root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap();
    candidates(&tree, o)
}

fn check(root: &Path, o: &DupeOptions) -> (DupeReport, Vec<DupeProgress>) {
    let c = stage1(root, o);
    let mut seen = Vec::new();
    let r = find_duplicates(&c, o, |p| seen.push(p), &AtomicBool::new(false));
    (r, seen)
}

fn names(r: &DupeReport) -> Vec<Vec<String>> {
    r.groups.iter().map(|g| g.files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect()).collect()
}

#[test]
fn identical_files_group_and_the_biggest_saving_comes_first() {
    let root = fresh("identical");
    let big = bytes(300_000, 1);
    write(&root.join("a/movie.mp4"), &big);
    write(&root.join("b/movie copy.mp4"), &big);
    write(&root.join("c/deep/movie (2).mp4"), &big);
    let small = bytes(100_000, 2);
    write(&root.join("a/notes.pdf"), &small);
    write(&root.join("b/notes.pdf"), &small);
    write(&root.join("lonely.bin"), &bytes(250_000, 3));
    let o = opts(&root);
    let c = stage1(&root, &o);
    assert_eq!(c.groups, 2);
    assert_eq!(c.possible, 2 * 300_000 + 100_000, "stage 1's upper bound: (n - 1) × size per size");

    let (r, seen) = check(&root, &o);
    assert!(!r.stopped);
    assert_eq!(r.skipped, 0);
    assert_eq!(names(&r), [vec!["movie.mp4", "movie copy.mp4", "movie (2).mp4"], vec!["notes.pdf", "notes.pdf"]]);
    assert_eq!(r.groups[0].reclaimable(), 600_000);
    assert_eq!(r.reclaimable(), 700_000);
    assert!(r.groups[0].files.iter().all(|f| f.modified.is_some()), "each copy has its date");
    // The stages report in order, and never more bytes than they said they would read.
    let stages: Vec<DupeStage> = seen.iter().map(|p| p.stage).collect();
    assert!(stages.windows(2).all(|w| w[0] <= w[1]), "{stages:?}");
    assert!(stages.contains(&DupeStage::Contents));
    assert!(seen.iter().all(|p| p.bytes_done <= p.bytes_total && p.files_done <= p.files_total));
}

#[test]
fn same_size_but_different_bytes_is_not_a_duplicate() {
    let root = fresh("different");
    write(&root.join("one.bin"), &bytes(300_000, 4));
    write(&root.join("two.bin"), &bytes(300_000, 5));
    let o = opts(&root);
    assert_eq!(stage1(&root, &o).groups, 1, "same size, so a possible duplicate");
    let (r, _) = check(&root, &o);
    assert!(r.groups.is_empty());
    assert!(r.bytes_read <= 2 * 2 * DUPE_EDGE, "the ends told them apart, so no whole file was read");
}

#[test]
fn same_ends_with_a_different_middle_is_caught_by_the_whole_file_hash() {
    let root = fresh("middle");
    let a = bytes(1 << 20, 6);
    let mut b = a.clone();
    b[(1 << 20) / 2] ^= 0xFF;
    write(&root.join("a.iso"), &a);
    write(&root.join("b.iso"), &b);
    let (r, seen) = check(&root, &opts(&root));
    assert!(r.groups.is_empty(), "one byte in the middle differs");
    let whole = seen.iter().rev().find(|p| p.stage == DupeStage::Contents).unwrap();
    assert_eq!(whole.bytes_total, 2 << 20, "both survived the ends and were read whole");
}

#[test]
fn a_hard_link_is_one_file_with_two_names() {
    let root = fresh("hardlink");
    let data = bytes(300_000, 7);
    write(&root.join("original.bin"), &data);
    fs::hard_link(root.join("original.bin"), root.join("link.bin")).unwrap();
    let (r, _) = check(&root, &opts(&root));
    assert!(r.groups.is_empty(), "two names for one file are never reported");

    write(&root.join("copy.bin"), &data);
    let (r, _) = check(&root, &opts(&root));
    assert_eq!(r.groups.len(), 1);
    assert_eq!(r.groups[0].files.len(), 2, "the copy and one name of the linked file");
    assert!(r.groups[0].files.iter().any(|f| f.path.ends_with("copy.bin")));
}

#[test]
fn empty_files_and_an_empty_folder_find_nothing() {
    let root = fresh("empty");
    let (r, _) = check(&root, &opts(&root));
    assert_eq!(r, DupeReport { drive: r.drive, threads: r.threads, ..DupeReport::default() });
    write(&root.join("a.txt"), b"");
    write(&root.join("b.txt"), b"");
    assert_eq!(stage1(&root, &opts(&root)).groups, 0, "zero-byte files are never candidates");
}

#[test]
fn locked_and_unreadable_files_are_skipped_and_counted() {
    let root = fresh("unreadable");
    let data = bytes(300_000, 8);
    for n in ["a.bin", "b.bin", "c.bin", "d.bin"] {
        write(&root.join(n), &data);
    }
    // c.bin is open in another "app" that shares nothing; d.bin denies reading its data.
    let lock = fs::OpenOptions::new().read(true).share_mode(0).open(root.join("c.bin")).unwrap();
    let denied = root.join("d.bin");
    let deny = Command::new("icacls").arg(&denied).args(["/deny", "*S-1-1-0:(RD)"]).output().unwrap();
    assert!(deny.status.success(), "{deny:?}");
    let (r, _) = check(&root, &opts(&root));
    let _ = Command::new("icacls").arg(&denied).args(["/remove:d", "*S-1-1-0"]).output();
    drop(lock);
    assert_eq!(r.skipped, 2);
    assert!(!r.stopped);
    assert_eq!(names(&r), [vec!["a.bin", "b.bin"]], "the readable copies still group");
}

#[test]
fn the_cache_saves_reads_and_a_new_write_time_reads_again() {
    let root = fresh("cache");
    let tree = root.join("files");
    let data = bytes(400_000, 9);
    write(&tree.join("a.bin"), &data);
    write(&tree.join("b.bin"), &data);
    let mut o = opts(&tree);
    o.cache = Some(root.join("dupes-cache.json"));

    let (first, _) = check(&tree, &o);
    assert_eq!(first.groups.len(), 1);
    assert!(first.bytes_read >= 800_000);
    assert_eq!(first.from_cache, 0);
    let text = fs::read_to_string(o.cache.as_ref().unwrap()).unwrap();
    assert!(text.contains("a.bin") && text.contains("\"full\""), "{text}");

    let (again, _) = check(&tree, &o);
    assert_eq!(again.groups, first.groups);
    assert_eq!(again.from_cache, 2);
    assert_eq!(again.bytes_read, 0, "nothing changed, so nothing is read");

    // Same size, new bytes, a new write time: b.bin is read again and no longer matches.
    let mut other = data.clone();
    other[200_000] ^= 1;
    fs::write(tree.join("b.bin"), &other).unwrap();
    let later = SystemTime::now() + Duration::from_secs(120);
    File::options().write(true).open(tree.join("b.bin")).unwrap().set_times(FileTimes::new().set_modified(later)).unwrap();
    let (changed, _) = check(&tree, &o);
    assert!(changed.groups.is_empty(), "the stale hash was not used");
    assert_eq!(changed.from_cache, 1, "a.bin still came from the cache");
    assert!(changed.bytes_read > 0 && changed.bytes_read < first.bytes_read);
}

#[test]
fn git_node_modules_and_never_touch_folders_are_left_out() {
    let root = fresh("skips");
    let data = bytes(300_000, 10);
    write(&root.join("Home/project/.git/objects/pack.bin"), &data);
    write(&root.join("Home/project/node_modules/lib/x.bin"), &data);
    write(&root.join("Home/.ssh/key.bin"), &data);
    write(&root.join("System/driver.bin"), &data);
    write(&root.join("Home/Videos/clip.bin"), &data);
    let mut o = opts(&root);
    let c = stage1(&root, &o);
    assert_eq!(c.groups, 1, "{c:?}");
    assert_eq!(c.files.len(), 2, "the system folder still counts until it is named");
    o.system_dirs = vec![root.join("System")];
    let c = stage1(&root, &o);
    assert_eq!(c.groups, 0, "only Videos is left: {c:?}");
}

#[test]
fn stop_ends_the_check_with_no_groups() {
    let root = fresh("stop");
    let data = bytes(300_000, 11);
    write(&root.join("a.bin"), &data);
    write(&root.join("b.bin"), &data);
    let o = opts(&root);
    let c = stage1(&root, &o);
    let r = find_duplicates(&c, &o, |_| {}, &AtomicBool::new(true));
    assert!(r.stopped);
    assert!(r.groups.is_empty());
}

#[test]
fn an_extra_copy_moves_only_while_its_kept_copy_is_there() {
    let root = fresh("removal");
    let data = bytes(300_000, 12);
    let (a, b, c) = (root.join("a.bin"), root.join("b.bin"), root.join("c.bin"));
    for p in [&a, &b, &c] {
        write(p, &data);
    }
    let env = RuleEnv::under(&root);
    let item = |path: &Path, keep: &Path| DupeRemoval { path: path.to_path_buf(), size: 300_000, keep: keep.to_path_buf() };
    let ok = item(&b, &a);
    assert_eq!(check_duplicate_removal(&env, &ok, std::slice::from_ref(&ok)), Ok(()));
    let both = [item(&b, &a), item(&a, &c)];
    assert_eq!(check_duplicate_removal(&env, &both[0], &both), Err(Refusal::NoCopyLeft), "its keeper is moving too");
    assert_eq!(check_duplicate_removal(&env, &item(&b, &b), &[]), Err(Refusal::NoCopyLeft), "a file is not its own keeper");
    assert_eq!(check_duplicate_removal(&env, &item(&b, &root.join("gone.bin")), &[]), Err(Refusal::NoCopyLeft));
    let wrong = DupeRemoval { size: 299_999, ..item(&b, &a) };
    assert_eq!(check_duplicate_removal(&env, &wrong, &[]), Err(Refusal::Changed), "not the file that was compared");
    let ssh = root.join(r"Home\.ssh\id.bin");
    write(&ssh, &data);
    assert_eq!(check_duplicate_removal(&env, &item(&ssh, &a), &[]), Err(Refusal::NeverTouch));
    let relative = DupeRemoval { path: PathBuf::from("b.bin"), ..item(&b, &a) };
    assert_eq!(check_duplicate_removal(&env, &relative, &[]), Err(Refusal::NotNormal));
    assert!(a.exists() && b.exists() && c.exists(), "checking moves nothing");
}

#[test]
fn drive_type_has_an_answer_or_says_unknown() {
    // The dev PC's drive answers either way; a drive that is not there cannot.
    let _ = drive_kind(Path::new(env!("CARGO_TARGET_TMPDIR")));
    assert_eq!(drive_kind(Path::new(r"Q:\nowhere")), DriveKind::Unknown);
}
