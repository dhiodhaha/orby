//! "Safe to clean": finds candidates and their sizes. Never removes anything; the user
//! reviews a list and confirms, and the app then calls `move_to_recycle_bin`.

use crate::sys;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Top-level items in Downloads whose newest created or modified date is older than the cutoff.
    OldDownloads,
    /// Top-level items in the user and Windows temp folders.
    TempFiles,
    /// Edge and Chrome cache folders, never cookies, history or passwords.
    BrowserCaches,
}

/// The honesty tag on a suggestion. Words in the UI, never colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// The user's own files: look first.
    CheckFirst,
    /// Windows or the app rebuilds it.
    SafeToRemove,
}

impl SuggestionKind {
    pub fn tag(self) -> Tag {
        match self {
            SuggestionKind::OldDownloads => Tag::CheckFirst,
            SuggestionKind::TempFiles | SuggestionKind::BrowserCaches => Tag::SafeToRemove,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    /// Logical bytes, the whole subtree for a folder.
    pub size: u64,
    /// Newest created or modified time of the item and everything inside it.
    pub newest: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    /// Biggest first. Zero-byte items are left out because removing them frees nothing.
    pub items: Vec<Candidate>,
}

impl Suggestion {
    pub fn total(&self) -> u64 {
        self.items.iter().map(|c| c.size).sum()
    }
}

/// Where to look. Built from the environment by [`CleanPlaces::for_current_user`], or by
/// hand in tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanPlaces {
    pub downloads: Option<PathBuf>,
    pub temp: Vec<PathBuf>,
    /// Chromium "User Data" folders; profiles inside them are found by name.
    pub browser_user_data: Vec<PathBuf>,
}

const CACHE_FOLDERS: [&str; 3] = ["Cache", "Code Cache", "GPUCache"];

impl CleanPlaces {
    pub fn for_current_user() -> Self {
        let mut temp = vec![std::env::temp_dir()];
        if let Some(root) = std::env::var_os("SystemRoot") {
            let win = PathBuf::from(root).join("Temp");
            if !temp.iter().any(|t| same_path(t, &win)) {
                temp.push(win);
            }
        }
        let browser_user_data = std::env::var_os("LOCALAPPDATA")
            .map(|l| {
                let l = PathBuf::from(l);
                vec![l.join(r"Microsoft\Edge\User Data"), l.join(r"Google\Chrome\User Data")]
            })
            .unwrap_or_default();
        CleanPlaces { downloads: sys::downloads_dir(), temp, browser_user_data }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// The three suggestions, in panel order. Folders that do not exist or cannot be read
/// just contribute nothing.
pub fn safe_to_clean(places: &CleanPlaces, now: SystemTime, older_than_days: u32) -> Vec<Suggestion> {
    let cutoff = now.checked_sub(Duration::from_secs(u64::from(older_than_days) * 86_400));
    let downloads = places
        .downloads
        .iter()
        .flat_map(|d| top_level(d))
        .filter(|c| matches!((c.newest, cutoff), (Some(n), Some(cut)) if n < cut))
        .collect();
    let temp = places.temp.iter().flat_map(|d| top_level(d)).collect();
    let caches = places
        .browser_user_data
        .iter()
        .flat_map(|ud| profiles(ud))
        .flat_map(|profile| CACHE_FOLDERS.iter().map(move |c| profile.join(c)))
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()))
        .map(|p| measure(&p))
        .collect();
    [(SuggestionKind::OldDownloads, downloads), (SuggestionKind::TempFiles, temp), (SuggestionKind::BrowserCaches, caches)]
        .into_iter()
        .map(|(kind, items)| Suggestion { kind, items: tidy(items) })
        .collect()
}

fn tidy(mut items: Vec<Candidate>) -> Vec<Candidate> {
    items.retain(|c| c.size > 0);
    items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    items
}

fn profiles(user_data: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(user_data) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n == "Default" || n.starts_with("Profile ")
        })
        .map(|e| e.path())
        .collect()
}

fn top_level(dir: &Path) -> Vec<Candidate> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    rd.flatten().map(|e| measure(&e.path())).collect()
}

/// Size and newest date of a file or folder tree, never following links.
fn measure(path: &Path) -> Candidate {
    let mut size = 0;
    let mut newest = None;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(md) = p.symlink_metadata() else { continue };
        newest = newest.max(md.created().ok()).max(md.modified().ok());
        if md.is_dir() {
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        } else if md.is_file() {
            size += md.len();
        }
    }
    Candidate { path: path.to_path_buf(), size, newest }
}
