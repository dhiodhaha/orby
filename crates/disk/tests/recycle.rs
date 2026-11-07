#![cfg(windows)]
//! Only ever touches files this test creates under the crate's own target dir.

use app_disk::{has_recycle_bin, move_to_recycle_bin, recycle_bin_size, RecycleError};
use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("recycle").join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "moves real files into the user's Recycle Bin; run on purpose with: cargo test -p app-disk -- --ignored"]
fn moves_files_and_folders_and_reports_a_partial_move() {
    let dir = fresh("partial");
    assert!(has_recycle_bin(&dir), "the target dir sits on a fixed drive");
    let file = dir.join("app-disk-test-file.txt");
    fs::write(&file, b"recycle me").unwrap();
    let folder = dir.join("app-disk-test-folder");
    fs::create_dir_all(folder.join("inner")).unwrap();
    fs::write(folder.join("inner/x.bin"), [1u8; 64]).unwrap();
    let missing = dir.join("app-disk-test-missing.txt");

    let report = move_to_recycle_bin(&[file.clone(), missing.clone(), folder.clone()]);
    assert_eq!(report.moved, [file.clone(), folder.clone()]);
    assert_eq!(report.failed, [(missing, RecycleError::NotFound)]);
    assert!(!file.exists());
    assert!(!folder.exists());
    assert!(dir.exists(), "only the named items move");
}

#[test]
#[ignore = "moves real files into the user's Recycle Bin; run on purpose with: cargo test -p app-disk -- --ignored"]
fn a_file_open_in_another_app_stays_and_the_rest_still_move() {
    let dir = fresh("in-use");
    let locked = dir.join("app-disk-test-locked.iso");
    let other = dir.join("app-disk-test-other.txt");
    fs::write(&locked, b"busy").unwrap();
    fs::write(&other, b"free").unwrap();
    let handle = fs::File::options().read(true).share_mode(0).open(&locked).unwrap();

    let report = move_to_recycle_bin(&[locked.clone(), other.clone()]);
    drop(handle);
    assert_eq!(report.moved, std::slice::from_ref(&other));
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0, locked);
    assert_eq!(report.failed[0].1, RecycleError::InUse);
    assert!(locked.exists(), "nothing is lost when a move fails");
    assert!(!other.exists());
}

#[test]
fn unsafe_paths_are_refused_before_the_shell_sees_them() {
    let dir = fresh("refused");
    fs::write(dir.join("keep.txt"), b"x").unwrap();
    let report = move_to_recycle_bin(&[
        PathBuf::from("relative.txt"),
        dir.join("*.txt"),
        PathBuf::from(r"C:\"),
        PathBuf::from(format!(r"\\?\{}", dir.join("keep.txt").display())),
    ]);
    assert!(report.moved.is_empty());
    assert!(report.failed.iter().all(|(_, e)| *e == RecycleError::InvalidPath), "{report:?}");
    assert!(dir.join("keep.txt").exists());
}

#[test]
fn recycle_bin_size_is_readable() {
    assert!(recycle_bin_size(Path::new(env!("CARGO_TARGET_TMPDIR"))).is_ok());
}
