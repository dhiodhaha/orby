//! Undo for Move to Recycle Bin: puts items back where they were.
//!
//! The engine only moves items in. Windows keeps each recycled item as a pair in
//! `<drive>\$Recycle.Bin\<user SID>`: `$R…` is the item itself and `$I…` records where it came
//! from and when. Restoring renames `$R…` back to that place and drops its `$I…` record, which is
//! what Explorer's Restore does. Nothing is ever overwritten: a place that is taken again stays
//! as it is and the item stays in the bin.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::core::HSTRING;
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEDIR, SHCNF_PATHW};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// No record of it in the bin (emptied, or restored from Explorer already).
    NotInBin,
    /// Something new is at the old place.
    Occupied,
    Failed,
}

/// Seconds from 1601 (FILETIME) to 1970.
const FILETIME_TO_UNIX: u64 = 11_644_473_600;

fn filetime(ft: u64) -> SystemTime {
    let secs = ft / 10_000_000;
    let nanos = (ft % 10_000_000) * 100;
    UNIX_EPOCH
        + Duration::from_secs(secs.saturating_sub(FILETIME_TO_UNIX))
        + Duration::from_nanos(nanos)
}

/// A `$I` record: the original path and when it was recycled. Version 2 (Windows 10 and later)
/// stores the path's length first; version 1 a fixed 260 characters.
pub fn parse_info(b: &[u8]) -> Option<(PathBuf, SystemTime)> {
    let u64_at = |o: usize| {
        b.get(o..o + 8)
            .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    };
    let version = u64_at(0)?;
    let deleted = filetime(u64_at(16)?);
    let (start, chars) = match version {
        2 => (
            28,
            u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize,
        ),
        1 => (24, 260),
        _ => return None,
    };
    let raw = b.get(start..start + chars * 2)?;
    let wide: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&c| c != 0)
        .collect();
    (!wide.is_empty()).then(|| (PathBuf::from(String::from_utf16_lossy(&wide)), deleted))
}

struct Entry {
    info: PathBuf,
    item: PathBuf,
    original: PathBuf,
    deleted: SystemTime,
}

fn bin_root(p: &Path) -> Option<PathBuf> {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 2 && b[1] == b':')
        .then(|| PathBuf::from(format!("{}:\\$Recycle.Bin", b[0] as char)))
}

/// Every record this user can read on the drive (other users' folders refuse to list).
fn entries(bin: &Path) -> Vec<Entry> {
    let Ok(users) = fs::read_dir(bin) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for user in users.flatten() {
        let Ok(items) = fs::read_dir(user.path()) else {
            continue;
        };
        for e in items.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("$I") else {
                continue;
            };
            let Ok(bytes) = fs::read(e.path()) else {
                continue;
            };
            let Some((original, deleted)) = parse_info(&bytes) else {
                continue;
            };
            out.push(Entry {
                info: e.path(),
                item: user.path().join(format!("$R{rest}")),
                original,
                deleted,
            });
        }
    }
    out
}

fn same(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches('\\'))
}

/// Puts each path back from the bin. Only records written since `since` (minus a little clock
/// slack) count, so an older copy recycled from the same place is never the one restored.
pub fn restore(paths: &[PathBuf], since: SystemTime) -> Vec<(PathBuf, Result<(), RestoreError>)> {
    let cutoff = since.checked_sub(Duration::from_secs(5)).unwrap_or(since);
    let mut by_bin: Vec<(PathBuf, Vec<Entry>)> = Vec::new();
    paths
        .iter()
        .map(|p| {
            let Some(bin) = bin_root(p) else {
                return (p.clone(), Err(RestoreError::NotInBin));
            };
            if !by_bin.iter().any(|(b, _)| *b == bin) {
                let list = entries(&bin);
                by_bin.push((bin.clone(), list));
            }
            let list = &by_bin.iter().find(|(b, _)| *b == bin).unwrap().1;
            let found = list
                .iter()
                .filter(|e| {
                    same(&e.original, p) && e.deleted >= cutoff && e.item.symlink_metadata().is_ok()
                })
                .max_by_key(|e| e.deleted);
            let Some(e) = found else {
                return (p.clone(), Err(RestoreError::NotInBin));
            };
            if p.symlink_metadata().is_ok() {
                return (p.clone(), Err(RestoreError::Occupied));
            }
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if fs::rename(&e.item, p).is_err() {
                return (p.clone(), Err(RestoreError::Failed));
            }
            let _ = fs::remove_file(&e.info);
            if let Some(parent) = p.parent() {
                unsafe {
                    SHChangeNotify(
                        SHCNE_UPDATEDIR,
                        SHCNF_PATHW,
                        Some(HSTRING::from(parent.as_os_str()).as_ptr() as *const _),
                        None,
                    )
                };
            }
            (p.clone(), Ok(()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(version: u64, path: &str, ft: u64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&10u64.to_le_bytes());
        b.extend_from_slice(&ft.to_le_bytes());
        let w: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        if version == 2 {
            b.extend_from_slice(&(w.len() as u32).to_le_bytes());
        }
        let mut w = w;
        if version == 1 {
            w.resize(260, 0);
        }
        b.extend(w.iter().flat_map(|c| c.to_le_bytes()));
        b
    }

    #[test]
    fn both_record_versions_give_the_place_and_the_time() {
        let ft = (FILETIME_TO_UNIX + 1_800_000_000) * 10_000_000;
        for v in [1, 2] {
            let (p, t) =
                parse_info(&record(v, r"C:\Users\You\Downloads\Win11_24H2_x64.iso", ft)).unwrap();
            assert_eq!(
                p,
                PathBuf::from(r"C:\Users\You\Downloads\Win11_24H2_x64.iso")
            );
            assert_eq!(t, UNIX_EPOCH + Duration::from_secs(1_800_000_000));
        }
        assert_eq!(parse_info(&[0; 10]), None);
        assert_eq!(parse_info(&record(3, "x", ft)), None);
    }

    /// Only touches what it creates under the user's temp folder.
    #[test]
    fn undo_puts_a_recycled_file_and_folder_back() {
        let root = std::env::temp_dir().join(format!("orb-proto-undo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("folder").join("inner")).unwrap();
        let file = root.join("orb-proto-undo-test.txt");
        let folder = root.join("folder");
        fs::write(&file, b"put me back").unwrap();
        fs::write(folder.join("inner").join("x.bin"), [7u8; 32]).unwrap();
        let since = SystemTime::now();
        let report = app_disk::move_to_recycle_bin(&[file.clone(), folder.clone()]);
        assert_eq!(report.moved.len(), 2, "{report:?}");
        assert!(!file.exists() && !folder.exists());

        // Something new at the old place is never overwritten.
        fs::write(&file, b"newer").unwrap();
        let r = restore(std::slice::from_ref(&file), since);
        assert_eq!(r[0].1, Err(RestoreError::Occupied));
        assert_eq!(fs::read(&file).unwrap(), b"newer");
        fs::remove_file(&file).unwrap();

        let r = restore(&[file.clone(), folder.clone()], since);
        assert!(r.iter().all(|(_, x)| x.is_ok()), "{r:?}");
        assert_eq!(fs::read(&file).unwrap(), b"put me back");
        assert_eq!(
            fs::read(folder.join("inner").join("x.bin")).unwrap(),
            [7u8; 32]
        );
        assert_eq!(
            restore(std::slice::from_ref(&file), since)[0].1,
            Err(RestoreError::NotInBin),
            "already put back"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
