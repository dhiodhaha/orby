//! Read-only survey of a folder: `cargo run --release --example survey -- <folder> [--scan-only]`.
//!
//! Scans the folder, prints totals, what was skipped and why, cloud placeholders seen, the
//! biggest folders, then (unless `--scan-only`) runs the duplicate finder with the default
//! options (no hash cache, so nothing is written) and prints what it found, and the volume's
//! space. Nothing is moved or deleted: this only reads.

use app_disk::{candidates, default_keeper, drive_space, find_duplicates, scan, DupeOptions, NodeId, ScanOptions, SkipReason, Tree};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime};

fn gb(b: u64) -> String {
    format!("{:.2} GB", b as f64 / 1e9)
}

/// Every node under `id` (inclusive), depth first.
fn walk(tree: &Tree, id: NodeId, mut each: impl FnMut(NodeId)) {
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        each(n);
        stack.extend_from_slice(tree.children(n));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let scan_only = args.next().as_deref() == Some("--scan-only");
    let never = AtomicBool::new(false);

    // ---- Scan ----
    let opts = ScanOptions::default();
    let start = Instant::now();
    let tree = scan(&root, &opts, |_| {}, &never).expect("scan failed");
    let secs = start.elapsed().as_secs_f64();
    let r = tree.node(tree.root());
    println!("== scan {}", tree.root_path().display());
    println!("status      {:?}", tree.status());
    println!("threads     {}", opts.threads);
    println!("time        {secs:.2} s");
    println!("size        {} ({} bytes), allocated {}", gb(r.size()), r.size(), gb(r.allocated()));
    println!("files       {}", r.files());
    println!("folders     {}", r.dirs());
    println!("nodes       {}", tree.len());

    let mut reasons: BTreeMap<String, Vec<&Path>> = BTreeMap::new();
    for s in tree.skipped() {
        let key = match s.reason {
            SkipReason::AccessDenied => "access denied".to_string(),
            SkipReason::Vanished => "vanished".to_string(),
            SkipReason::InCloud => "in the cloud, not downloaded".to_string(),
            SkipReason::Other(k) => format!("other: {k:?}"),
        };
        reasons.entry(key).or_default().push(&s.path);
    }
    println!("skipped     {}", tree.skipped().len());
    for (why, paths) in &reasons {
        println!("  {why}: {}", paths.len());
        for p in paths.iter().take(40) {
            println!("    {}", p.display());
        }
        if paths.len() > 40 {
            println!("    ... {} more", paths.len() - 40);
        }
    }

    let (mut placeholders, mut placeholder_bytes, mut placeholder_dirs, mut links) = (0u64, 0u64, 0u64, 0u64);
    walk(&tree, tree.root(), |id| {
        let n = tree.node(id);
        if n.not_local() {
            if n.is_dir() {
                placeholder_dirs += 1;
            } else {
                placeholders += 1;
                placeholder_bytes += n.size();
            }
        }
        if n.reparse() {
            links += 1;
        }
    });
    println!("dataless    {placeholders} files ({} logical, 0 on disk), {placeholder_dirs} folders", gb(placeholder_bytes));
    println!("links       {links} (listed, not followed)");

    println!("top 10 folders in {}", root.display());
    for c in tree.children_by_size(tree.root()).into_iter().filter(|c| tree.node(c.id).is_dir()).take(10) {
        let n = tree.node(c.id);
        println!("  {:>10}  {:5.1}%  {:>8} files  {}", gb(c.size), c.percent_of_parent, n.files(), tree.path(c.id).display());
    }

    if !scan_only {
        duplicates(&tree, &never);
    }

    // ---- Volume ----
    println!("\n== volumes");
    for p in [Path::new("/"), root.as_path()] {
        match drive_space(p) {
            Ok(s) => println!(
                "{:<12} total {} ({} bytes), free {} ({} bytes), used {} ({:.1}%)",
                p.display(),
                gb(s.total),
                s.total,
                gb(s.free),
                s.free,
                gb(s.used()),
                s.used_percent()
            ),
            Err(e) => println!("{}: {e}", p.display()),
        }
    }
}

/// The duplicate finder with the default options, but no hash cache, so nothing is written.
fn duplicates(tree: &Tree, never: &AtomicBool) {
    let d = DupeOptions::for_current_user(None, SystemTime::now());
    let start = Instant::now();
    let c = candidates(tree, &d);
    let stage1 = start.elapsed().as_secs_f64();
    let report = find_duplicates(&c, &d, |_| {}, never);
    let secs = start.elapsed().as_secs_f64();
    println!("\n== duplicates (min {} bytes, cache off)", d.min_size);
    println!("left out    {}", d.system_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    println!("candidates  {} files in {} same-size groups, at most {} ({stage1:.2} s)", c.files.len(), c.groups, gb(c.possible));
    println!("groups      {}", report.groups.len());
    println!("reclaimable {} ({} bytes)", gb(report.reclaimable()), report.reclaimable());
    // Before the file system said what each copy frees: (n - 1) x size per group.
    let logical: u64 = report.groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
    let shared: Vec<&app_disk::DupeGroup> = report.groups.iter().filter(|g| g.shares_space()).collect();
    let shared_logical: u64 = shared.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
    println!(
        "by size     {} ({} bytes) if every copy owned its blocks",
        gb(logical),
        logical
    );
    println!(
        "clones      {} groups share their blocks (APFS clones or a snapshot): {} by size, {} really freed",
        shared.len(),
        gb(shared_logical),
        gb(shared.iter().map(|g| g.reclaimable()).sum())
    );
    println!("bytes read  {} ({} bytes)", gb(report.bytes_read), report.bytes_read);
    println!("skipped     {}", report.skipped);
    println!("drive       {:?}, threads {}", report.drive, report.threads);
    println!("time        {secs:.2} s");
    let mut groups: Vec<_> = report.groups.iter().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable()));
    for (i, g) in groups.iter().take(5).enumerate() {
        let keep = default_keeper(&g.files, &d.env);
        println!("  #{} {} x {} copies, frees {}", i + 1, gb(g.size), g.files.len(), gb(g.reclaimable()));
        for (j, f) in g.files.iter().enumerate() {
            println!("     {} {}", if j == keep { "keep " } else { "extra" }, f.path.display());
        }
    }
}
