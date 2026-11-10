#![cfg(target_os = "macos")]
//! macOS behaviour on trees made under the test target folder. Nothing here reads the user's own
//! folders. Only the one #[ignore]d test touches the real Trash.

use app_disk::{
    candidates, check_duplicate_removal, default_dupes_cache, default_keeper, drive_kind, drive_space, find_duplicates, has_recycle_bin,
    measure_rules, move_to_recycle_bin, safe_to_clean, scan, CleanPlaces, DriveKind, DupeFile, DupeOptions, DupeRemoval, NodeKind,
    RecycleError, Refusal, RuleEnv, ScanOptions, SuggestionKind, RULES,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("macos").join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

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
        system_dirs: vec![root.join("Home").join("Library")],
        cache: None,
        threads: Some(2),
        now: UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    }
}

fn group_names(root: &Path, o: &DupeOptions) -> Vec<Vec<String>> {
    let tree = scan(root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap();
    let c = candidates(&tree, o);
    let r = find_duplicates(&c, o, |_| {}, &AtomicBool::new(false));
    r.groups.iter().map(|g| g.files.iter().map(|f| f.path.strip_prefix(root).unwrap().to_string_lossy().into_owned()).collect()).collect()
}

#[test]
fn the_scan_counts_packages_and_marks_dot_files_hidden() {
    let root = fresh("scan");
    write(&root.join("Foo.app/Contents/MacOS/foo"), &bytes(5000, 1));
    write(&root.join(".hidden"), b"x");
    let tree = scan(&root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(tree.node(tree.root()).size(), 5001, "a package's bytes still count toward its folder");
    let app = tree.find(&root.join("Foo.app")).unwrap();
    assert_eq!(tree.node(app).kind(), NodeKind::Dir);
    assert!(tree.node(tree.find(&root.join(".hidden")).unwrap()).hidden());
}

#[test]
fn a_symbolic_link_is_listed_but_not_followed() {
    let root = fresh("symlink");
    write(&root.join("real/big.bin"), &bytes(4096, 1));
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let tree = scan(&root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(tree.node(tree.find(&root.join("link")).unwrap()).kind(), NodeKind::Link);
    assert_eq!(tree.node(tree.root()).size(), 4096, "counted once");
}

#[test]
fn duplicates_skip_packages_the_trash_library_and_hard_links() {
    let root = fresh("dupes");
    let data = bytes(300_000, 3);
    write(&root.join("a/movie.mov"), &data);
    write(&root.join("b/movie.mov"), &data);
    write(&root.join("Foo.app/Contents/Resources/movie.mov"), &data);
    write(&root.join("Photos Library.photoslibrary/originals/movie.mov"), &data);
    write(&root.join(".Trash/movie.mov"), &data);
    write(&root.join("Home/Library/Caches/x/movie.mov"), &data);
    // Two names for one file, and a third copy of it: the hard-linked pair is left out whole,
    // so the third copy has nothing to pair with.
    let other = bytes(200_000, 9);
    write(&root.join("c/linked.bin"), &other);
    fs::create_dir_all(root.join("d")).unwrap();
    fs::hard_link(root.join("c/linked.bin"), root.join("d/linked.bin")).unwrap();
    write(&root.join("e/linked.bin"), &other);

    let groups = group_names(&root, &opts(&root));
    assert_eq!(groups, [vec!["a/movie.mov".to_string(), "b/movie.mov".to_string()]]);
}

#[test]
fn an_extra_copy_inside_a_package_or_the_trash_is_never_moved() {
    let root = fresh("removal");
    let env = RuleEnv::under(&root);
    let data = bytes(1000, 4);
    let keep = root.join("a/x.bin");
    write(&keep, &data);
    for rel in ["Foo.app/Contents/x.bin", ".Trash/x.bin", "node_modules/x.bin"] {
        let p = root.join(rel);
        write(&p, &data);
        let item = DupeRemoval { path: p, size: 1000, keep: keep.clone() };
        assert_eq!(check_duplicate_removal(&env, &item, &[]), Err(Refusal::NeverTouch), "{rel}");
    }
    let linked = root.join("b/x.bin");
    fs::create_dir_all(root.join("b")).unwrap();
    fs::hard_link(&keep, &linked).unwrap();
    let item = DupeRemoval { path: linked, size: 1000, keep: keep.clone() };
    assert_eq!(check_duplicate_removal(&env, &item, &[]), Err(Refusal::Changed), "a hard link frees nothing");
    let plain = root.join("c/x.bin");
    write(&plain, &data);
    let item = DupeRemoval { path: plain, size: 1000, keep: root.join("d/missing.bin") };
    assert_eq!(check_duplicate_removal(&env, &item, &[]), Err(Refusal::NoCopyLeft));
}

#[test]
fn unsafe_paths_are_refused_before_the_trash_sees_them() {
    let root = fresh("refused");
    let inside = root.join("Foo.app/Contents/Info.plist");
    write(&inside, b"x");
    let in_trash = root.join(".Trash/old.zip");
    write(&in_trash, b"x");
    let report = move_to_recycle_bin(&[
        inside.clone(),
        in_trash.clone(),
        PathBuf::from("/System/Library/CoreServices/SystemVersion.plist"),
        PathBuf::from("/usr/bin/true"),
        PathBuf::from("/Applications"),
        PathBuf::from("/"),
    ]);
    assert!(report.moved.is_empty(), "{report:?}");
    assert!(report.failed.iter().all(|(_, e)| matches!(e, RecycleError::Refused | RecycleError::InvalidPath)), "{report:?}");
    assert!(inside.exists() && in_trash.exists());
}

#[test]
fn volume_space_trash_and_drive_kind_answer_for_the_target_folder() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let s = drive_space(dir).unwrap();
    assert!(s.total > 0 && s.free <= s.total, "{s:?}");
    assert!(has_recycle_bin(dir), "a local, writable volume has a Trash");
    assert_ne!(drive_kind(dir), DriveKind::Hdd, "macOS never claims a hard drive");
    if cfg!(target_arch = "aarch64") {
        assert_eq!(drive_kind(Path::new("/")), DriveKind::Ssd, "an Apple Silicon Mac boots from its internal SSD");
    }
}

#[test]
fn library_caches_stand_in_for_temp_files_without_apple_or_rule_caches() {
    let root = fresh("clean");
    let caches = root.join("Library/Caches");
    write(&caches.join("com.apple.Safari/x"), &[1; 100]);
    write(&caches.join("CloudKit/x"), &[1; 100]);
    write(&caches.join("Homebrew/x.tar.gz"), &[1; 100]);
    write(&caches.join("com.spotify.client/data"), &[1; 300]);
    write(&caches.join("Google/Chrome/Default/Cache/data_0"), &[1; 200]);
    let places = CleanPlaces { downloads: None, temp: vec![caches.clone()], browser_user_data: vec![caches.join("Google/Chrome")] };
    let s = safe_to_clean(&places, SystemTime::now(), 30);
    let temp = s.iter().find(|x| x.kind == SuggestionKind::TempFiles).unwrap();
    assert_eq!(temp.items.iter().map(|c| c.path.clone()).collect::<Vec<_>>(), [caches.join("com.spotify.client")]);
    let browser = s.iter().find(|x| x.kind == SuggestionKind::BrowserCaches).unwrap();
    assert_eq!(browser.items.len(), 1);
    assert_eq!(browser.items[0].path, caches.join("Google/Chrome/Default/Cache"));
}

#[test]
fn xcode_derived_data_is_measured_from_a_fake_home() {
    let root = fresh("rules");
    let env = RuleEnv::under(&root);
    let home = env.user_profile.clone().unwrap();
    write(&home.join("Library/Developer/Xcode/DerivedData/App-abc/Build/x.o"), &[1; 1000]);
    write(&home.join("Library/Caches/Homebrew/wget.bottle.tar.gz"), &[1; 500]);
    let m = measure_rules(&env, RULES, &AtomicBool::new(false));
    assert!(m.complete);
    let ids: Vec<&str> = m.found.iter().map(|f| f.rule.id).collect();
    assert_eq!(ids, ["xcode-derived-data", "homebrew-cache"]);
}

#[test]
fn the_keeper_is_never_the_library_or_downloads_copy() {
    let root = PathBuf::from("/fake");
    let env = RuleEnv::under(&root);
    let home = env.user_profile.clone().unwrap();
    let at = |p: PathBuf, days: u64| DupeFile {
        path: p,
        size: 1,
        modified: Some(UNIX_EPOCH + Duration::from_secs(1_000_000_000 - days * 86_400)),
        frees: None,
    };
    let files = [
        at(home.join("Library/Application Support/App/a.mp4"), 900),
        at(home.join("Downloads/a.mp4"), 800),
        at(PathBuf::from("/private/tmp/a.mp4"), 700),
        at(home.join("Movies/a.mp4"), 5),
    ];
    assert_eq!(default_keeper(&files, &env), 3);
}

#[test]
fn the_cache_lives_in_application_support() {
    let p = default_dupes_cache().unwrap();
    assert!(p.ends_with("Library/Application Support/orb-proto/dupes-cache.json"), "{}", p.display());
}

#[test]
#[ignore = "moves real files into the user's Trash; run on purpose with: cargo test -p app-disk -- --ignored"]
fn moves_a_file_and_a_folder_to_the_trash_and_reports_a_missing_one() {
    let dir = fresh("trash");
    let file = dir.join("app-disk-test-file.txt");
    fs::write(&file, b"trash me").unwrap();
    let folder = dir.join("app-disk-test-folder");
    write(&folder.join("inner/x.bin"), &[1; 64]);
    let missing = dir.join("app-disk-test-missing.txt");
    let report = move_to_recycle_bin(&[file.clone(), missing.clone(), folder.clone()]);
    assert_eq!(report.moved, [file.clone(), folder.clone()]);
    assert_eq!(report.failed, [(missing, RecycleError::NotFound)]);
    assert!(!file.exists() && !folder.exists());
    assert!(dir.exists(), "only the named items move");
}

extern "C" {
    fn clonefile(src: *const std::ffi::c_char, dst: *const std::ffi::c_char, flags: u32) -> std::ffi::c_int;
}

#[test]
fn an_apfs_clone_is_found_identical_but_frees_almost_nothing() {
    use std::os::unix::ffi::OsStrExt;
    let root = fresh("clones");
    let home = root.join("Home");
    let data = bytes(3 << 20, 7);
    let (a, b, c) = (home.join("Movies/a.bin"), home.join("Movies/b.bin"), home.join("Documents/c.bin"));
    write(&a, &data);
    write(&c, &data);
    fs::create_dir_all(b.parent().unwrap()).unwrap();
    let cs = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
    // SAFETY: both paths are NUL terminated; clonefile only reads `a` and creates `b`.
    let made = unsafe { clonefile(cs(&a).as_ptr(), cs(&b).as_ptr(), 0) } == 0;
    if !made {
        eprintln!("clonefile is not available on this volume; skipping");
        return;
    }
    let tree = scan(&root, &ScanOptions::default(), |_| {}, &AtomicBool::new(false)).unwrap();
    let report = find_duplicates(&candidates(&tree, &opts(&root)), &opts(&root), |_| {}, &AtomicBool::new(false));
    assert_eq!(report.groups.len(), 1);
    let g = &report.groups[0];
    assert_eq!(g.files.len(), 3);
    let frees = |p: &Path| g.files.iter().find(|f| f.path == p).and_then(|f| f.frees).unwrap();
    // The clone pair shares every block; the plain copy owns all of its own.
    assert!(frees(&a) < 64 << 10 && frees(&b) < 64 << 10, "{} {}", frees(&a), frees(&b));
    assert!(frees(&c) >= 3 << 20);
    assert!(g.shares_space());
    // Keeping the plain copy, moving both clones frees nearly nothing; never 2 x 3 MB.
    assert!(g.reclaimable() < 128 << 10, "{}", g.reclaimable());
}
