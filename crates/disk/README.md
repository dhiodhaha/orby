# app-disk (Arbor)

The disk scanner behind the Disk panels (`docs/design/disk.md`). It scans a folder or drive into an arena tree with a multi-threaded walk. It answers the panel queries, finds safe-to-clean candidates and moves items to the Recycle Bin. It never deletes anything by itself, has no permanent delete and never empties the Recycle Bin. There is no UI code here.

## How the scan works

- The calling thread owns the `Tree`. Worker threads (cores x 2 by default) only read folders. Each listing comes back over a channel and is inserted, and its totals are pushed up to every ancestor. The tree is therefore correct at every progress callback, and the callback can read it directly ("Biggest so far" is `children_by_size(root)` during the scan).
- No file is opened while scanning. On Windows all facts come from the directory listing, so OneDrive placeholders count at their logical size and are never downloaded.
- Junctions, symbolic links and mount points become `NodeKind::Link`. They are listed, never entered and count zero bytes. Other reparse points, such as OneDrive folders, are real folders and are entered.
- If a folder cannot be read, it goes into `Tree::skipped()` with a `SkipReason` and the scan carries on. Setting the cancel flag stops the scan within one folder per worker. The returned tree keeps what was counted and has status `Stopped`.
- Allocated size: rounded up to whole clusters, the compressed size for compressed or sparse files, and 0 for cloud placeholders whose data is not on the disk.

## Public API

| Item | What it does |
|---|---|
| `scan(root, &ScanOptions, progress: FnMut(Progress), cancel: &AtomicBool) -> Result<Tree, ScanError>` | The walk. `ScanOptions { threads, progress_every }`, `Progress { tree, current }`. |
| `ScanError` | `NotFound`, `NotReady` (drive unplugged or locked), `AccessDenied`, `NotADirectory`, `Io`. |
| `Tree` | `root`, `root_path`, `node`, `children`, `path`, `ancestry` (breadcrumb), `find(path)`, `status` (`Running` / `Complete` / `Stopped`), `skipped`, `len`. |
| `Tree::children_by_size(id) -> Vec<ChildShare>` | Biggest first, with `percent_of_parent`. |
| `Tree::largest_files(id, n) -> Vec<NodeId>` | Top N files in the subtree. |
| `Tree::type_breakdown(id) -> Vec<TypeShare>` | Video, Apps and games, Archives, Images, Documents by extension. The five are ordered biggest first, then Other last. |
| `Node` | `name`, `parent`, `kind` (`File` / `Dir` / `Link`), `size`, `allocated`, `files`, `dirs`, `modified` (newest in subtree for folders), `hidden`, `system`, `not_local` (cloud placeholder or offline), `reparse` (any reparse point). |
| `drive_space(path) -> DriveSpace { total, free }` | `used`, `used_percent`, `free_percent`, `scan_percent(counted)` (bytes counted over bytes in use). |
| `LowSpaceWatch::check(DriveSpace) -> bool` | The low-space pill rule: fires below 10% free, then again only after 5 more points, and re-arms once free space is back above 10%. |
| `safe_to_clean(&CleanPlaces, now, older_than_days) -> Vec<Suggestion>` | Three suggestions in panel order: `OldDownloads` (Check first), `TempFiles`, `BrowserCaches` (Safe to remove). It computes the lists and removes nothing. `CleanPlaces::for_current_user()` finds Downloads (known folder), `%TEMP%`, `%SystemRoot%\Temp`, and Edge and Chrome "User Data". |
| `move_to_recycle_bin(&[PathBuf]) -> RecycleReport { moved, failed }` | `SHFileOperationW` with `FOF_ALLOWUNDO`, one item at a time, so a partial move reports what did move. `RecycleError`: `NotFound`, `NoRecycleBin`, `InUse`, `AccessDenied`, `InvalidPath`, `Refused` (the profile, AppData, Windows or Program Files folder, or anything holding them; or a rule path that failed `check_removal`), `Aborted`, `Shell(code)`. |
| `RULES`, `Rule { id, group, name, paths, tier, explanation }`, `Group`, `Tier` | The cleaning-rule table (`src/rules.rs`; structure and safety lessons adapted from Purge, MIT). Paths expand `%LOCALAPPDATA%`, `%APPDATA%`, `%USERPROFILE%` through `RuleEnv` (`from_process()`, or `under(root)` for tests). |
| `measure_rules(&RuleEnv, rules, cancel) -> Measured { found: Vec<RuleFound>, complete }`, `RuleMeasure::start(env)` | Sizes each rule's folders with the scanner. Only folders that exist, pass the checks and hold bytes. `RuleMeasure` runs it on a thread: `try_take`, `cancel`, `wait`; dropping it cancels. |
| `check_removal(&RuleEnv, &Rule, path) -> Result<(), Refusal>`, `never_touch(&RuleEnv, path)` | Inside the rule's own folder, not on `NEVER_TOUCH` / `NEVER_REMOVE_ROOTS` / `NEVER_NAMES`, no junction on the way. |
| `recycle_rule_items(&RuleEnv, &[(&Rule, PathBuf)]) -> RecycleReport` | Checks each path again, then `move_to_recycle_bin`. Refused paths come back as `RecycleError::Refused` and never reach the shell. |
| `is_unfinished_download(name)`, `UNFINISHED_DOWNLOAD` | `.crdownload`, `.part`, `.tmp` -> "Unfinished download". |
| `has_recycle_bin(path)`, `recycle_bin_size(path)` | Recycle is offered only on fixed drives. The size feeds the "Not counted above" row. |

`now` is always a parameter. The only clock read is in `examples/bench.rs` (the duplicate check also times its progress reports with `Instant`).

## Duplicate files (`src/dupes.rs`)

- **Input:** the scan's own tree. Each node now keeps three flags from the listing (`system`, `not_local`, `reparse`), so stage 1 needs no second walk.
- **Stage 1, `candidates(&Tree, &DupeOptions)`:** files of `min_size` (1 MB) and up, grouped by size; groups of one are dropped. It skips `.git` and `node_modules`, the Windows, Program Files and ProgramData folders, links and other reparse points, system files, cloud placeholders, and the never-touch list (`NEVER_TOUCH`, `NEVER_REMOVE_ROOTS`, `NEVER_NAMES`, expanded once). No file is opened. `Candidates::possible` is the sum over same-size groups of (n − 1) × size: an upper bound.
- **Before reading, `find_duplicates`:** each file is opened for its attributes only (`FILE_READ_ATTRIBUTES`, `FILE_FLAG_OPEN_REPARSE_POINT`, `FILE_FLAG_OPEN_NO_RECALL`), so a placeholder is never downloaded. `GetFileInformationByHandle` gives the attributes, the size, the write time and the file id. Files with `RECALL_ON_DATA_ACCESS`, `RECALL_ON_OPEN`, `OFFLINE`, `SYSTEM` or `REPARSE_POINT` are dropped. Two names with the same volume serial and file index are a hard link and count once.
- **Stage 2:** BLAKE3 of the size plus the first and last 64 KB (the whole file up to 128 KB).
- **Stage 3:** BLAKE3 of the whole file in 1 MB reads. A full BLAKE3 match counts as identical; there is no byte compare (a collision is not a practical risk, and a compare would read every copy a second time). Data reads use `FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_NO_RECALL`.
- **Drive type:** `drive_kind(path)` opens the volume (`\\?\Volume{…}`, no access rights) and asks `IOCTL_STORAGE_QUERY_PROPERTY` for `StorageDeviceSeekPenaltyProperty`. No seek penalty is an SSD: one thread per core. A seek penalty is a hard drive, and no answer (a volume that cannot be opened, a driver that does not say) is `Unknown`: both get one thread with sequential 1 MB reads. `DupeOptions::threads` overrides it.
- **Hash cache:** a JSON file (the app keeps it at `%APPDATA%\orb-proto\dupes-cache.json`, next to `disk.json`): `{"version":1,"files":{"<path>":{"size":…,"written":<FILETIME ticks>,"ends":"<hex>","full":"<hex>","seen":<unix secs>}}}`. An entry counts only while the path, size and write time all match, so a rescan reads only files that changed. After a full check it forgets files under the checked root that the check did not see, drops entries not seen for 180 days, and keeps at most 100,000 (newest first). It is written whole (write, then rename); an unreadable cache is ignored.
- **Progress, stop, errors:** `DupeProgress { stage, files_done, files_total, bytes_done, bytes_total }` at most every 100 ms, also during one long read. Setting `cancel` stops within one 1 MB read per thread; the report then has `stopped` and no groups. A file that cannot be opened or read (in use, access denied, changed mid-read) is counted in `skipped` and left out.
- **Output:** `DupeReport { groups, skipped, stopped, drive, threads, bytes_read, from_cache }`. Groups (`DupeGroup { size, files }`, files by path) are sorted by `reclaimable()` = (n − 1) × size, biggest first.
- **Moving extra copies:** `recycle_duplicates(&RuleEnv, &[DupeRemoval { path, size, keep }])` checks each with `check_duplicate_removal` (a plain path; not on the never-touch list or in a system, `.git` or `node_modules` folder; no reparse point on the way; still a plain file of that size; its kept copy still there, the same size, not itself and not in the same batch), then calls `move_to_recycle_bin`. A refused copy is `RecycleError::Refused` (`Refusal::Changed` or `Refusal::NoCopyLeft` from the check) and never reaches the shell.

| Item | What it does |
|---|---|
| `candidates(&Tree, &DupeOptions) -> Candidates { root, files, possible, groups }` | Stage 1. |
| `find_duplicates(&Candidates, &DupeOptions, progress: FnMut(DupeProgress), cancel) -> DupeReport` | The attribute pass, stages 2 and 3, the cache. Blocks. |
| `DupeOptions { min_size, env, system_dirs, cache, threads, now }`, `DupeOptions::for_current_user(cache, now)` | 1 MB, the user's never-touch list and system folders, threads by drive type. |
| `drive_kind(path) -> DriveKind` | `Ssd`, `Hdd` or `Unknown`. |
| `check_duplicate_removal`, `recycle_duplicates`, `DupeRemoval` | See above. |

Tests (`tests/dupes.rs`, temp folders under the target folder only, nothing goes to the Recycle Bin): identical files across folders, same size with different bytes, same ends with a different middle, a hard link, empty files and an empty folder, a file locked by another handle and one with read denied (both skipped and counted), the cache (a second run reads 0 bytes; a new write time reads that file again), `.git`, `node_modules`, never-touch and system folders left out, stop, the removal checks, and drive type for a missing drive. Run: `cargo test -p app-disk` (21 unit and 11 duplicate tests passed on 2026-11-08, clippy clean).

## Verified

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\disk\target"; cd C:\Users\dopamine\dev\clippy\crates\disk; cargo test; cargo clippy --all-targets -- -D warnings
```

Final run: 26 tests passed (9 unit, 9 scan, 4 clean, 4 recycle), and clippy reported no warnings. The tests build their own trees under `target\tmp`. They cover:
- totals, percentages, top files and the type breakdown
- a real junction, which is not followed
- an `icacls` deny rule, which gives an access-denied skip
- 1 thread against 8 threads (same counts) and live progress
- stop mid-scan and stop before the scan starts
- typed root errors
- Downloads age by the newer of the created and modified dates
- browser caches, with cookies and history excluded
- a partial Recycle Bin move with one file locked

After the recycle tests, the items were confirmed in the Recycle Bin with their original locations, so they can be restored.

Benchmark (read-only, release build, warm cache, 40 threads): `cargo run --release --example bench -- C:\Users\dopamine\dev\clippy` scanned 11,277 files and 1,472 folders (1.47 GB) in 0.017 s, which is about 660,000 files/s. The file count and byte total matched `Get-ChildItem -Recurse -Force -File` exactly.

## macOS

Everything above builds for `aarch64-apple-darwin` behind `#[cfg(target_os = "macos")]`, with the same public API, so callers do not change. Windows behaviour is unchanged. Policy lives in `src/mac.rs`, OS calls in the macOS half of `src/sys.rs`.

- **Trash:** `move_to_recycle_bin` calls `NSFileManager trashItemAtURL:resultingItemURL:error:` (objc2 0.6.5, objc2-foundation 0.3.2, pinned) inside an autorelease pool. It only ever moves to the user's Trash and fails where there is none (`NSFeatureUnsupportedError` and read-only volumes map to `NoRecycleBin`); there is no delete path. `has_recycle_bin` is true for local, writable volumes (`statfs`: `MNT_LOCAL`, not `MNT_RDONLY`). `recycle_bin_size` sums `~/.Trash` (or `<volume>/.Trashes/<uid>`); without Full Disk Access it is an error, not zero. `move_to_recycle_bin` also refuses anything inside a package, inside a Trash, or under `/System`, `/bin`, `/sbin` or `/usr` (not `/usr/local`), and anything holding home, `~/Library`, its two bases or a system folder.
- **Scan:** facts come from `lstat` (`st_flags`): `UF_HIDDEN` or a leading dot is hidden, `SF_RESTRICTED` (SIP) is system, `SF_DATALESS` is a cloud placeholder (not local, 0 allocated). Allocated size is `st_blocks × 512`, which already covers compressed files. Workers run with `setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, OFF)`, so a dataless folder answers `EDEADLK` instead of downloading and goes into `skipped()` as `SkipReason::InCloud`. A folder `read_dir` refuses with `EINTR` is tried again (up to three more times). A mount point becomes `NodeKind::Link` and is not entered: a folder the mount table (`getfsstat`, read once per scan) names, or one on another device than its parent. The table is needed because the system and data volumes of an APFS volume group report the same `st_dev`, so `/System/Volumes/Data` does not look like another device; before the table, a scan of `/` counted the data volume twice (423.6 GB on a 228.9 GB-used disk). Firmlinks (`/usr/share/firmlinks`, with a built-in fallback) are entered, being the one real path to the data volume's folders.
- **Duplicates:** stage 1 also skips packages (`MAC_PACKAGE_EXTENSIONS`, by extension: `.app`, `.photoslibrary`, `.fcpbundle`, `.logicx`, ...), the Trash and volume system folders (`DUPE_SKIPPED_NAMES`), and `MAC_SYSTEM_DIRS` plus `~/Library` (`system_dirs`). The probe is `lstat` only; placeholders, SIP files, non-files and hard links (`st_nlink > 1`) are dropped before any read. Reads open with `O_NOFOLLOW` on threads that cannot download placeholders. `check_duplicate_removal` also refuses a path inside a package and a hard-linked file. `drive_kind` says `Ssd` for the boot volume (`/` or `/System/Volumes/*`, APFS, local) on Apple Silicon and `Unknown` for anything else (one thread).
- **Keeper:** `default_keeper(&[DupeFile], &RuleEnv)` and `passing_through` (all platforms): the oldest copy that is not in Downloads, Desktop, Temp or tmp, and not in the folder holding `app_data` (`~/Library` on macOS, `AppData` on Windows), when another copy exists; then the oldest; then by path.
- **Rules and Safe to clean:** `RuleEnv::from_process()` reads `$HOME`: `local_app_data` is `~/Library/Caches`, `app_data` is `~/Library/Application Support`, `user_profile` is home. Rule paths use `%CACHES%`, `%APPSUPPORT%` and `%HOME%` (aliases accepted on every platform). `RULES`, `NEVER_TOUCH` and `NEVER_REMOVE_ROOTS` are macOS tables (see `docs/design/disk.md`). `CleanPlaces::for_current_user()` uses `~/Downloads`, `~/Library/Caches` (top-level app caches, leaving out `com.apple.*`, a few Apple names and caches a rule offers) and the Chrome and Edge caches in `~/Library/Caches`; `$TMPDIR` is not offered.
- **Cache:** `default_dupes_cache()` is `~/Library/Application Support/orb-proto/dupes-cache.json` (on Windows `%APPDATA%\orb-proto\dupes-cache.json`).
- **BLAKE3:** the default build on every platform, so macOS hashes with NEON. blake3's build script compiles its NEON C file with the Mac's C compiler (Xcode's command line tools).

- **Clones and allocated size:** after the whole-file check, each copy in a group asks `getattrlist(ATTR_CMNEXT_PRIVATESIZE)` what moving it would free: blocks no APFS clone and no snapshot shares. `DupeFile::frees` carries it (None on Windows); `DupeGroup::reclaimable()` keeps the copy that frees the most and counts what the rest free, and `shares_space()` says when a copy frees less than half its size. Test: `an_apfs_clone_is_found_identical_but_frees_almost_nothing` makes a real `clonefile(2)` clone. On the home folder of the survey Mac: 1.80 GB by size, 1.64 GB really reclaimable; 34 groups share blocks (0.17 GB by size, 0.01 GB freed).
- **Put Back:** `move_to_recycle_bin` remembers where each item landed in the Trash (`resultingItemURL`); `put_back(original)` renames it back, never over something new and never making folders (`PutBackError::{NotInBin, Occupied, Failed}`). Windows answers `NotInBin` (the app restores from the Recycle Bin's records).
- **Consent prompts:** `full_disk_access()` and `protected_folders()` (macOS): without Full Disk Access the folders a scan would stop at a prompt for (Desktop, Documents, Downloads, iCloud Drive, CloudStorage, other apps' containers, the Photos, Photo Booth, Music and TV libraries). `ScanOptions::protected` leaves them out as `SkipReason::Protected` (empty by default). The walk's workers are detached, and a watchdog leaves out any folder held for 4 s (`scan_waiting` lists folders being read longer than a given time).

Built, tested and linted on a Mac (Apple Silicon, Xcode command line tools):

```sh
cargo test -p app-disk
cargo clippy -p app-disk --all-targets -- -D warnings
```

From Windows, `cargo check --target aarch64-apple-darwin` fails in blake3's build script, which wants a C compiler for the Mac. For a quick check or lint only (nothing is linked), set `CARGO_FEATURE_PURE=1` so that build script skips the C file:

```powershell
$env:CARGO_FEATURE_PURE = '1'
cargo clippy -p app-disk --target aarch64-apple-darwin --all-targets -- -D warnings
```

The integration tests `tests/scan.rs`, `tests/dupes.rs`, `tests/clean.rs` and `tests/rules.rs` run on both systems. Where Windows needs its own tools, each has a macOS twin: `chmod 000` for an `icacls` deny rule or a file held with no sharing, a symbolic link for a junction, `chflags hidden` for the hidden attribute, `std::os::macos::fs::FileTimesExt::set_created` for creation times, and a fake home laid out as macOS does for the rule profile. A hard-linked file is left out whole on macOS, and that test says so. `tests/recycle.rs` stays Windows only. On top of those, the macOS-only tests are the unit tests in `src/mac.rs`, `src/sys.rs` (`mac_tests`, including the data volume being a mount point) and `src/rules.rs`, and `tests/macos.rs` (packages, the Trash folder, hard links, refused paths, `~/Library/Caches`, Xcode DerivedData, drive type, the keeper, the cache path). Every test works on trees under the target folder; the one test that moves files into the real Trash is `#[ignore]`d. A dataless file cannot be made by a test, so that path is covered by the flag logic in `mac_tests` and the survey below.

Survey (read-only, release build, MacBook Pro M1, 8 GB, 16 threads): `cargo run --release --example survey -- ~` scans, then runs the duplicate finder with the default options and no hash cache, so it writes nothing. On a 145.6 GB home folder (1,651,388 files, 239,352 folders) the scan took 14.4 s, saw 58,524 dataless iCloud files (19.38 GB in the cloud, 0 on disk) and skipped 4 dataless folders as `InCloud` without downloading them (`du -sk ~/Library/Mobile\ Documents` was the same before and after). The duplicate finder read 3.05 GB in 2.3 s on 8 threads and found 141 groups, 1.78 GB reclaimable. Peak RSS was 313 MB. A scan of `/` (`--scan-only`) counted 223.3 GB against the 228.9 GB `statfs` says is used (the gap is likely the volumes it does not enter, such as VM swap and Preboot, local snapshots, and the 286 folders only root may read; not measured), in 25.4 s with 418 MB peak RSS.
