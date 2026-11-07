//! A stand-in for yt-dlp in tests. It checks the arguments the engine builds, then replays
//! output recorded from real yt-dlp runs, writing small real files into the `-P` folder.
//! The scenario comes from the YouTube video id in the URL. Nothing touches the network.
//!
//! - `success0001`: video and audio streams, then a merge (exit 0)
//! - `private0001`: a private video (exit 1)
//! - `diskfull001`: disk full part way through (exit 1)
//! - `slowslow001`: a long download for pause and cancel tests (exit 0 if left alone)

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::thread::sleep;
use std::time::Duration;

struct Args {
    folder: PathBuf,
    output: String,
    template: String,
    url: String,
}

fn main() {
    let args = parse_args().unwrap_or_else(|missing| {
        eprintln!("ERROR: fake-yt-dlp: missing or bad argument: {missing}");
        exit(2);
    });
    let id = args.url.rsplit("v=").next().unwrap_or_default().to_string();
    let title = "Sample clip";
    println!("[youtube] Extracting URL: {}", args.url);
    println!("[youtube] {id}: Downloading webpage");
    println!("[youtube] {id}: Downloading tv client config");
    println!("[youtube] {id}: Downloading player 0b866fa6");
    match id.as_str() {
        "success0001" => {
            println!("[info] {id}: Downloading 1 format(s): 137+140");
            let video = download(&args, title, &id, "f137.mp4", 40_000, 40_000, 5);
            let audio = download(&args, title, &id, "f140.m4a", 10_000, 10_000, 5);
            let merged = args.folder.join(render(&args.output, title, &id, "mp4"));
            println!("[Merger] Merging formats into \"{}\"", merged.display());
            let mut bytes = fs::read(&video).unwrap();
            bytes.extend(fs::read(&audio).unwrap());
            fs::write(&merged, bytes).unwrap();
            for f in [&video, &audio] {
                println!("Deleting original file {} (pass -k to keep)", f.display());
                fs::remove_file(f).unwrap();
            }
        }
        "private0001" => {
            eprintln!("ERROR: [youtube] {id}: Private video. Sign in if you've been granted access to this video. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies. Also see  https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies  for tips on effectively exporting YouTube cookies");
            exit(1);
        }
        "diskfull001" => {
            println!("[info] {id}: Downloading 1 format(s): 18");
            download(&args, title, &id, "mp4", 25_000, 41_200, 5);
            eprintln!("ERROR: unable to write data: [Errno 28] No space left on device");
            exit(1);
        }
        "slowslow001" => {
            println!("[info] {id}: Downloading 1 format(s): 18");
            download(&args, title, &id, "mp4", 100_000, 100_000, 300);
        }
        _ => {
            eprintln!("ERROR: [youtube] {id}: Video unavailable");
            exit(1);
        }
    }
}

fn parse_args() -> Result<Args, &'static str> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &'static str| argv.iter().position(|a| a == flag).and_then(|i| argv.get(i + 1)).cloned().ok_or(flag);
    for flag in ["--newline", "--no-playlist"] {
        if !argv.iter().any(|a| a == flag) {
            return Err(flag);
        }
    }
    let ffmpeg = value("--ffmpeg-location")?;
    if ffmpeg.is_empty() {
        return Err("--ffmpeg-location");
    }
    value("-f")?;
    let template = value("--progress-template")?;
    if !template.starts_with("download:") || !template.contains("%(progress)j") {
        return Err("--progress-template");
    }
    let url = match argv.iter().position(|a| a == "--") {
        Some(i) if i + 2 == argv.len() => argv[i + 1].clone(),
        _ => return Err("-- URL"),
    };
    Ok(Args { folder: PathBuf::from(value("-P")?), output: value("-o")?, template, url })
}

/// The subset of yt-dlp's output template our engine uses.
fn render(output: &str, title: &str, id: &str, ext: &str) -> String {
    let name = output.replace("%(title).150B", title).replace("%(id)s", id).replace("%(ext)s", ext);
    if name.contains('%') {
        eprintln!("ERROR: fake-yt-dlp: unknown output template field in {output}");
        exit(2);
    }
    name
}

/// Downloads `stop_at` of `total` bytes into `<name>.part` in `steps` progress lines, then renames
/// it when complete. Returns the final path.
fn download(args: &Args, title: &str, id: &str, ext: &str, stop_at: u64, total: u64, steps: u64) -> PathBuf {
    let dest = args.folder.join(render(&args.output, title, id, ext));
    let part = PathBuf::from(format!("{}.part", dest.display()));
    println!("[download] Destination: {}", dest.display());
    let delay = if steps > 50 { 50 } else { 5 };
    let mut done = 0;
    for step in 1..=steps {
        let next = stop_at * step / steps;
        append(&part, next - done);
        done = next;
        let left = total - done;
        progress(args, title, &dest, &part, "downloading", done, total, left);
        sleep(Duration::from_millis(delay));
    }
    if done == total {
        fs::rename(&part, &dest).unwrap();
        progress(args, title, &dest, &part, "finished", done, total, 0);
        println!("[download] 100% of   {:.2}KiB in 00:00:00 at 2.50MiB/s", total as f64 / 1024.0);
    }
    dest
}

fn append(path: &Path, n: u64) {
    let mut f = OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(&vec![b'x'; n as usize]).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn progress(args: &Args, title: &str, dest: &Path, part: &Path, status: &str, done: u64, total: u64, left: u64) {
    let speed = 2_621_440.0_f64;
    let eta = (left as f64 / speed).ceil() as u64;
    let p = format!(
        "{{\"status\":{},\"downloaded_bytes\":{done},\"total_bytes\":{total},\"tmpfilename\":{},\"filename\":{},\"eta\":{eta},\"speed\":{speed},\"elapsed\":0.25,\"ctx_id\":null,\"_eta_str\":\"00:00\",\"_speed_str\":\"2.50MiB/s\"}}",
        json(status),
        json(&part.display().to_string()),
        json(&dest.display().to_string()),
    );
    let line = args.template.trim_start_matches("download:").replace("%(info.title)j", &json(title)).replace("%(progress)j", &p);
    println!("{line}");
}

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
