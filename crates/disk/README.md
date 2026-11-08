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
| `Node` | `name`, `parent`, `kind` (`File` / `Dir` / `Link`), `size`, `allocated`, `files`, `dirs`, `modified` (newest in subtree for folders), `hidden`. |
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

`now` is always a parameter. The only clock read is in `examples/bench.rs`.

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
