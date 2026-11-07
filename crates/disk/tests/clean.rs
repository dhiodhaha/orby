#![cfg(windows)]
use app_disk::{safe_to_clean, CleanPlaces, SuggestionKind, Tag};
use std::fs::{self, File, FileTimes};
use std::os::windows::fs::FileTimesExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("clean").join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes]).unwrap();
}

/// Sets created and modified to `days` before `now`.
fn age(path: &Path, now: SystemTime, created_days: u64, modified_days: u64) {
    let t = |d| now - Duration::from_secs(d * DAY);
    let times = FileTimes::new().set_created(t(created_days)).set_modified(t(modified_days));
    File::options().write(true).open(path).unwrap().set_times(times).unwrap();
}

fn now() -> SystemTime {
    // A fixed clock: 2026-11-06. File times are set relative to it.
    UNIX_EPOCH + Duration::from_secs(1_793_923_200)
}

#[test]
fn downloads_older_than_cutoff_by_newer_of_created_and_modified() {
    let root = fresh("downloads");
    let dl = root.join("Downloads");
    let now = now();
    write(&dl.join("old.iso"), 5000);
    age(&dl.join("old.iso"), now, 200, 200);
    write(&dl.join("recent.zip"), 4000);
    age(&dl.join("recent.zip"), now, 10, 10);
    // Downloaded last week, but the server stamped an old modified date: not old.
    write(&dl.join("fresh-copy.pdf"), 3000);
    age(&dl.join("fresh-copy.pdf"), now, 5, 400);
    write(&dl.join("old-folder/a.bin"), 1000);
    age(&dl.join("old-folder/a.bin"), now, 120, 120);
    write(&dl.join("mixed-folder/old.bin"), 2000);
    age(&dl.join("mixed-folder/old.bin"), now, 300, 300);
    write(&dl.join("mixed-folder/new.bin"), 10);
    age(&dl.join("mixed-folder/new.bin"), now, 3, 3);
    write(&dl.join("empty-old.txt"), 0);
    age(&dl.join("empty-old.txt"), now, 300, 300);

    let places = CleanPlaces { downloads: Some(dl.clone()), ..Default::default() };
    // Folder entries carry their own created time (today), so age them too.
    for f in ["old-folder", "mixed-folder"] {
        let times = FileTimes::new().set_created(now - Duration::from_secs(400 * DAY)).set_modified(now - Duration::from_secs(400 * DAY));
        open_dir(&dl.join(f)).set_times(times).unwrap();
    }
    let s = safe_to_clean(&places, now, 90);
    assert_eq!(s.len(), 3);
    let d = &s[0];
    assert_eq!(d.kind, SuggestionKind::OldDownloads);
    assert_eq!(d.kind.tag(), Tag::CheckFirst);
    let got: Vec<_> = d.items.iter().map(|c| (c.path.file_name().unwrap().to_str().unwrap(), c.size)).collect();
    assert_eq!(got, [("old.iso", 5000), ("old-folder", 1000)]);
    assert_eq!(d.total(), 6000);
    assert!(dl.join("old.iso").exists(), "suggesting never removes");
}

/// A writable handle on a folder, so its times can be set.
fn open_dir(p: &Path) -> File {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    File::options().write(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(p).unwrap()
}
#[test]
fn temp_items_from_every_temp_folder_biggest_first() {
    let root = fresh("temp");
    write(&root.join("user-temp/a.tmp"), 300);
    write(&root.join("user-temp/sub/b.tmp"), 700);
    write(&root.join("user-temp/zero.tmp"), 0);
    write(&root.join("win-temp/c.log"), 500);
    let places = CleanPlaces { temp: vec![root.join("user-temp"), root.join("win-temp"), root.join("missing")], ..Default::default() };
    let s = safe_to_clean(&places, now(), 90);
    let t = &s[1];
    assert_eq!(t.kind, SuggestionKind::TempFiles);
    assert_eq!(t.kind.tag(), Tag::SafeToRemove);
    let got: Vec<_> = t.items.iter().map(|c| (c.path.strip_prefix(&root).unwrap().to_path_buf(), c.size)).collect();
    assert_eq!(
        got,
        [(PathBuf::from(r"user-temp\sub"), 700), (PathBuf::from(r"win-temp\c.log"), 500), (PathBuf::from(r"user-temp\a.tmp"), 300)]
    );
    assert!(s[0].items.is_empty() && s[2].items.is_empty());
}

#[test]
fn browser_caches_only_never_cookies_or_history() {
    let root = fresh("browser");
    let edge = root.join("Edge/User Data");
    let chrome = root.join("Chrome/User Data");
    write(&edge.join("Default/Cache/Cache_Data/data_1"), 4000);
    write(&edge.join("Default/Code Cache/js/index"), 1000);
    write(&edge.join("Default/Network/Cookies"), 9000);
    write(&edge.join("Default/History"), 9000);
    write(&edge.join("Default/Login Data"), 9000);
    write(&edge.join("System Profile/Cache/x"), 9000);
    write(&chrome.join("Profile 1/GPUCache/data_0"), 2000);
    write(&chrome.join("Profile 1/Bookmarks"), 9000);

    let places = CleanPlaces { browser_user_data: vec![edge.clone(), chrome.clone(), root.join("Firefox")], ..Default::default() };
    let s = safe_to_clean(&places, now(), 90);
    let b = &s[2];
    assert_eq!(b.kind, SuggestionKind::BrowserCaches);
    assert_eq!(b.kind.tag(), Tag::SafeToRemove);
    let got: Vec<_> = b.items.iter().map(|c| (c.path.clone(), c.size)).collect();
    assert_eq!(
        got,
        [
            (edge.join("Default").join("Cache"), 4000),
            (chrome.join("Profile 1").join("GPUCache"), 2000),
            (edge.join("Default").join("Code Cache"), 1000)
        ]
    );
    assert_eq!(b.total(), 7000);
}

#[test]
fn current_user_places_point_at_real_folders() {
    let p = CleanPlaces::for_current_user();
    assert!(p.downloads.is_some());
    assert!(!p.temp.is_empty());
    assert_eq!(p.browser_user_data.len(), 2);
}
