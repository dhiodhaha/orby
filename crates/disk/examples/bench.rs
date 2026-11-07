//! Read-only benchmark: `cargo run --release --example bench -- <folder>`.

use app_disk::{scan, ScanOptions};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let opts = ScanOptions::default();
    let start = Instant::now();
    let tree = scan(&root, &opts, |_| {}, &AtomicBool::new(false)).expect("scan failed");
    let secs = start.elapsed().as_secs_f64();
    let r = tree.node(tree.root());
    println!("root      {}", tree.root_path().display());
    println!("threads   {}", opts.threads);
    println!("files     {}", r.files());
    println!("folders   {}", r.dirs());
    println!("bytes     {} ({:.2} GB), allocated {:.2} GB", r.size(), r.size() as f64 / 1e9, r.allocated() as f64 / 1e9);
    println!("skipped   {}", tree.skipped().len());
    println!("status    {:?}", tree.status());
    println!("time      {secs:.3} s");
    println!("rate      {:.0} files/s, {:.0} entries/s", r.files() as f64 / secs, (tree.len() - 1) as f64 / secs);
}
