//! Cleaning rules: one static table of cache folders that a tool or app rebuilds by itself,
//! each with its group, its honesty tier and one plain line on what removing it costs.
//!
//! A rule only proposes paths. Every proposal, when it is measured and again when it is
//! moved, goes through [`check_removal`]: the path must sit at or below one of the rule's own
//! roots, must not touch the never-touch list and must not pass through a junction or other
//! reparse point. The never-touch list is separate from the rules, so a wrong rule can only
//! narrow what is offered, never widen it. Removal is [`move_to_recycle_bin`] only.
//!
//! The rule structure and the safety lessons (one table as the source of truth; a policy that
//! can only narrow a rule; Check first is never pre-selected; a tool's home folder such as
//! `.gradle` or `.cache` is never offered whole; browser Service Worker, IndexedDB and Local
//! Storage folders are never offered) are adapted from Purge,
//! <https://github.com/jithin-sabu/purge-app>, MIT, copyright (c) 2026 Jithin Sabu. The Windows
//! rules below are Orby's own. See `THIRD-PARTY-NOTICES.md`.

use crate::clean::{Candidate, Tag};
use crate::recycle::{move_to_recycle_bin, RecycleError, RecycleReport};
use crate::scan::{scan, ScanOptions};
use crate::sys;
use crate::tree::{ScanStatus, Tree};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

/// The honesty tier of a rule. The same two words as the Safe to clean tags.
pub type Tier = Tag;

/// Where a rule's row sits in the window. Panel order is [`Group::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    DeveloperCaches,
    AppCaches,
    BrowserCaches,
    AiModels,
    CrashReports,
}

impl Group {
    pub const ALL: [Group; 5] = [Self::DeveloperCaches, Self::AppCaches, Self::BrowserCaches, Self::AiModels, Self::CrashReports];

    pub fn label(self) -> &'static str {
        match self {
            Self::DeveloperCaches => "Developer caches",
            Self::AppCaches => "App caches",
            Self::BrowserCaches => "Browser caches",
            Self::AiModels => "AI models",
            Self::CrashReports => "Crash reports",
        }
    }
}

/// One cleaning rule. Paths start with `%LOCALAPPDATA%`, `%APPDATA%` or `%USERPROFILE%`
/// (on macOS `%CACHES%`, `%APPSUPPORT%` or `%HOME%`), use `\` or `/` between parts, and may
/// have a `*` part that matches any one real folder (never a junction or link).
#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    /// Stable id, for selection and logs.
    pub id: &'static str,
    pub group: Group,
    pub name: &'static str,
    pub paths: &'static [&'static str],
    pub tier: Tier,
    /// What removing it costs and how it comes back, in one line.
    pub explanation: &'static str,
}

#[cfg(not(target_os = "macos"))]
use Group::*;
#[cfg(not(target_os = "macos"))]
use Tag::{CheckFirst, SafeToRemove};

// Rule structure and safety lessons adapted from Purge (MIT, (c) 2026 Jithin Sabu,
// https://github.com/jithin-sabu/purge-app). The rows are Orby's own Windows rules.
/// Every cleaning rule, in panel order within each group.
#[cfg(not(target_os = "macos"))]
pub static RULES: &[Rule] = &[
    // ---- Developer caches ----
    Rule {
        id: "npm-cache",
        group: DeveloperCaches,
        name: "npm cache",
        paths: &[r"%LOCALAPPDATA%\npm-cache\_cacache", r"%LOCALAPPDATA%\npm-cache\_npx"],
        tier: SafeToRemove,
        explanation: "Packages npm already downloaded. Re-downloaded on your next install.",
    },
    Rule {
        id: "yarn-cache",
        group: DeveloperCaches,
        name: "Yarn cache",
        paths: &[r"%LOCALAPPDATA%\Yarn\Cache", r"%LOCALAPPDATA%\Yarn\Berry\cache"],
        tier: SafeToRemove,
        explanation: "Packages Yarn already downloaded. Re-downloaded on your next install.",
    },
    Rule {
        id: "pip-cache",
        group: DeveloperCaches,
        name: "pip cache",
        paths: &[r"%LOCALAPPDATA%\pip\Cache"],
        tier: SafeToRemove,
        explanation: "Python packages pip already downloaded. Re-downloaded the next time you install them.",
    },
    Rule {
        id: "uv-cache",
        group: DeveloperCaches,
        name: "uv cache",
        paths: &[r"%LOCALAPPDATA%\uv\cache"],
        tier: SafeToRemove,
        explanation: "Python packages uv already downloaded. Re-downloaded on your next uv sync.",
    },
    Rule {
        id: "cargo-registry",
        group: DeveloperCaches,
        name: "Cargo registry",
        paths: &[r"%USERPROFILE%\.cargo\registry\cache", r"%USERPROFILE%\.cargo\registry\src"],
        tier: SafeToRemove,
        explanation: "Crates Cargo downloaded and unpacked. Re-downloaded on your next build. Installed tools stay.",
    },
    Rule {
        id: "cargo-git",
        group: DeveloperCaches,
        name: "Cargo git checkouts",
        paths: &[r"%USERPROFILE%\.cargo\git"],
        tier: SafeToRemove,
        explanation: "Git dependencies Cargo checked out. Fetched again on your next build.",
    },
    Rule {
        id: "gradle-wrapper",
        group: DeveloperCaches,
        name: "Gradle wrapper downloads",
        paths: &[r"%USERPROFILE%\.gradle\wrapper\dists"],
        tier: SafeToRemove,
        explanation: "Gradle versions the wrapper downloaded. Re-downloaded on your next build.",
    },
    Rule {
        id: "gradle-caches",
        group: DeveloperCaches,
        name: "Gradle caches",
        paths: &[r"%USERPROFILE%\.gradle\caches"],
        tier: CheckFirst,
        explanation: "Offline builds need it. The next online build downloads every dependency again.",
    },
    Rule {
        id: "nuget-http-cache",
        group: DeveloperCaches,
        name: "NuGet download cache",
        paths: &[r"%LOCALAPPDATA%\NuGet\v3-cache"],
        tier: SafeToRemove,
        explanation: "Package downloads NuGet keeps for a while. Re-downloaded on your next restore.",
    },
    Rule {
        id: "go-build",
        group: DeveloperCaches,
        name: "Go build cache",
        paths: &[r"%LOCALAPPDATA%\go-build"],
        tier: SafeToRemove,
        explanation: "Compiled Go packages. Rebuilt on your next build, which is slower once.",
    },
    Rule {
        id: "nuget-packages",
        group: DeveloperCaches,
        name: "NuGet packages",
        paths: &[r"%USERPROFILE%\.nuget\packages"],
        tier: CheckFirst,
        explanation: "Every package your .NET projects use. Restored on the next build, which needs the internet and takes a while.",
    },
    Rule {
        id: "maven-repository",
        group: DeveloperCaches,
        name: "Maven repository",
        paths: &[r"%USERPROFILE%\.m2\repository"],
        tier: CheckFirst,
        explanation: "Libraries your Java builds use. Downloaded ones come back on the next build; ones you built with mvn install do not.",
    },
    Rule {
        id: "pnpm-store",
        group: DeveloperCaches,
        name: "pnpm store",
        paths: &[r"%LOCALAPPDATA%\pnpm\store", r"%USERPROFILE%\.pnpm-store"],
        tier: CheckFirst,
        explanation:
            "Packages your pnpm projects share through hard links, so it may free less than shown. Re-downloaded on your next install.",
    },
    // ---- App caches ----
    Rule {
        id: "vscode-cache",
        group: AppCaches,
        name: "VS Code caches",
        paths: &[r"%APPDATA%\Code\Cache", r"%APPDATA%\Code\CachedData", r"%APPDATA%\Code\CachedExtensionVSIXs"],
        tier: SafeToRemove,
        explanation: "Cached web content, compiled code and extension downloads. VS Code rebuilds them; its next start is a little slower.",
    },
    Rule {
        id: "discord-cache",
        group: AppCaches,
        name: "Discord cache",
        paths: &[r"%APPDATA%\discord\Cache", r"%APPDATA%\discord\Code Cache"],
        tier: SafeToRemove,
        explanation: "Images and code Discord saved. Loaded again as you use it. You stay signed in.",
    },
    Rule {
        id: "slack-cache",
        group: AppCaches,
        name: "Slack cache",
        paths: &[r"%APPDATA%\Slack\Cache", r"%APPDATA%\Slack\Code Cache"],
        tier: SafeToRemove,
        explanation: "Images and code Slack saved. Loaded again as you use it. You stay signed in.",
    },
    Rule {
        id: "teams-cache",
        group: AppCaches,
        name: "Teams cache",
        paths: &[
            r"%APPDATA%\Microsoft\Teams\Cache",
            r"%APPDATA%\Microsoft\Teams\Code Cache",
            r"%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\Microsoft\MSTeams\EBWebView\*\Cache",
            r"%LOCALAPPDATA%\Packages\MSTeams_8wekyb3d8bbwe\LocalCache\Microsoft\MSTeams\EBWebView\*\Code Cache",
        ],
        tier: SafeToRemove,
        explanation: "Images and code Teams saved. Loaded again as you use it. You stay signed in.",
    },
    // ---- Browser caches ----
    Rule {
        id: "firefox-cache",
        group: BrowserCaches,
        name: "Firefox cache",
        paths: &[r"%LOCALAPPDATA%\Mozilla\Firefox\Profiles\*\cache2"],
        tier: SafeToRemove,
        explanation: "Pages Firefox saved to load faster. Sites load a bit slower for a day. You stay signed in.",
    },
    // ---- AI models ----
    Rule {
        id: "huggingface-hub",
        group: AiModels,
        name: "Hugging Face downloads",
        paths: &[r"%USERPROFILE%\.cache\huggingface\hub"],
        tier: CheckFirst,
        explanation: "Models and datasets you downloaded. Getting them again can mean many gigabytes. Your sign-in token stays.",
    },
    Rule {
        id: "ollama-models",
        group: AiModels,
        name: "Ollama models",
        paths: &[r"%USERPROFILE%\.ollama\models"],
        tier: CheckFirst,
        explanation: "Local models you pulled. Pull them again to use them, which can take a long time.",
    },
    // ---- Crash reports ----
    Rule {
        id: "crash-dumps",
        group: CrashReports,
        name: "App crash dumps",
        paths: &[r"%LOCALAPPDATA%\CrashDumps"],
        tier: SafeToRemove,
        explanation: "Memory saved when apps crashed, only useful for debugging those crashes. Nothing comes back.",
    },
    Rule {
        id: "wer-archive",
        group: CrashReports,
        name: "Windows error reports",
        paths: &[r"%LOCALAPPDATA%\Microsoft\Windows\WER\ReportArchive"],
        tier: SafeToRemove,
        explanation: "Problem reports Windows already sent. Nothing comes back, and nothing needs to.",
    },
];

/// Never removed, nor anything inside them, nor any folder that holds them: tools you
/// installed, keys, tokens and passwords.
#[cfg(not(target_os = "macos"))]
pub static NEVER_TOUCH: &[&str] = &[
    r"%USERPROFILE%\.cargo\bin",
    r"%USERPROFILE%\.cargo\credentials",
    r"%USERPROFILE%\.cargo\credentials.toml",
    r"%USERPROFILE%\.ssh",
    r"%USERPROFILE%\.gnupg",
    r"%USERPROFILE%\.aws",
    r"%USERPROFILE%\.azure",
    r"%USERPROFILE%\.kube",
    r"%USERPROFILE%\.docker",
    r"%USERPROFILE%\.git-credentials",
    r"%USERPROFILE%\.npmrc",
    r"%USERPROFILE%\.gradle\gradle.properties",
    r"%USERPROFILE%\.m2\settings.xml",
    r"%USERPROFILE%\.m2\settings-security.xml",
    r"%USERPROFILE%\.cache\huggingface\token",
    r"%USERPROFILE%\.cache\huggingface\stored_tokens",
    r"%USERPROFILE%\.ollama\id_ed25519",
    r"%USERPROFILE%\.ollama\id_ed25519.pub",
    r"%APPDATA%\NuGet",
    r"%APPDATA%\Microsoft\Credentials",
    r"%APPDATA%\Microsoft\Protect",
    r"%LOCALAPPDATA%\Microsoft\Credentials",
    r"%LOCALAPPDATA%\Microsoft\Vault",
];

/// Folders that are never removed themselves, although a rule may offer something inside
/// them: the profile and AppData roots, and each tool's home folder (`.gradle` also holds
/// installed JDKs and settings, `.cache` holds models and runtimes).
#[cfg(not(target_os = "macos"))]
pub static NEVER_REMOVE_ROOTS: &[&str] = &[
    r"%USERPROFILE%",
    r"%APPDATA%",
    r"%LOCALAPPDATA%",
    r"%USERPROFILE%\.gradle",
    r"%USERPROFILE%\.gradle\wrapper",
    r"%USERPROFILE%\.cache",
    r"%USERPROFILE%\.cache\huggingface",
    r"%USERPROFILE%\.cargo",
    r"%USERPROFILE%\.cargo\registry",
    r"%USERPROFILE%\.nuget",
    r"%USERPROFILE%\.m2",
    r"%USERPROFILE%\.ollama",
    r"%LOCALAPPDATA%\npm-cache",
    r"%LOCALAPPDATA%\Yarn",
    r"%LOCALAPPDATA%\pip",
    r"%LOCALAPPDATA%\uv",
    r"%LOCALAPPDATA%\pnpm",
    r"%LOCALAPPDATA%\NuGet",
    r"%LOCALAPPDATA%\Packages",
    r"%LOCALAPPDATA%\Mozilla\Firefox\Profiles",
    r"%LOCALAPPDATA%\Microsoft\Windows\WER",
    r"%APPDATA%\Code",
    r"%APPDATA%\discord",
    r"%APPDATA%\Slack",
    r"%APPDATA%\Microsoft\Teams",
];

/// Browser and app state, never a cache. A path through one of these is refused, and a
/// measured folder that holds one is not offered at all.
pub static NEVER_NAMES: &[&str] = &["Service Worker", "IndexedDB", "Local Storage", "Session Storage"];

/// Names of credential stores, refused anywhere in a path that would be removed.
#[cfg(not(target_os = "macos"))]
const SECRET_NAMES: &[&str] = &["Cookies", "Login Data", "Web Data", "Local State", "Credentials", "Protect", "Vault", ".ssh", ".gnupg"];
#[cfg(target_os = "macos")]
const SECRET_NAMES: &[&str] =
    &["Cookies", "Login Data", "Web Data", "Local State", "Credentials", "Keychains", ".ssh", ".gnupg", "MobileSync"];

#[cfg(target_os = "macos")]
pub use crate::mac::{NEVER_REMOVE_ROOTS, NEVER_TOUCH, RULES};

/// Why a path was not offered or not moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Relative, with `.` or `..`, a `\\?\` path, or a drive root.
    NotNormal,
    /// On the never-touch list, inside it, holding it, or a profile root.
    NeverTouch,
    /// Not at or below one of the rule's own folders.
    OutsideRule,
    /// The path, or a folder on the way to it, is a junction, symbolic link or other
    /// reparse point.
    ReparsePoint,
    /// Holds browser state ([`NEVER_NAMES`]).
    HoldsAppState,
    /// A duplicate that is no longer the file that was compared: gone, a folder, a different
    /// size, or a system file or reparse point now.
    Changed,
    /// A duplicate whose kept copy is missing, changed, or is being moved in the same batch.
    NoCopyLeft,
}

/// The three folders rule paths start from. Built from the process environment, or by hand
/// in tests so nothing real is touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleEnv {
    pub local_app_data: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub user_profile: Option<PathBuf>,
}

impl RuleEnv {
    #[cfg(not(target_os = "macos"))]
    pub fn from_process() -> Self {
        let var = |n: &str| std::env::var_os(n).filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
        RuleEnv { local_app_data: var("LOCALAPPDATA"), app_data: var("APPDATA"), user_profile: var("USERPROFILE") }
    }

    /// On macOS the three folders are `~/Library/Caches` (local, rebuilt), `~/Library/Application
    /// Support` (the app's own data) and the home folder, all from `$HOME`.
    #[cfg(target_os = "macos")]
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute());
        match home {
            Some(h) => Self::mac(h),
            None => RuleEnv::default(),
        }
    }

    #[cfg(target_os = "macos")]
    fn mac(home: PathBuf) -> Self {
        let library = home.join("Library");
        RuleEnv {
            local_app_data: Some(library.join("Caches")),
            app_data: Some(library.join("Application Support")),
            user_profile: Some(home),
        }
    }

    /// A fake profile at `root\Home`, laid out as Windows does (`Home\AppData\Local` and
    /// `Home\AppData\Roaming`), for tests and test drives. Nothing real is touched.
    #[cfg(not(target_os = "macos"))]
    pub fn under(root: &Path) -> Self {
        let home = root.join("Home");
        RuleEnv {
            local_app_data: Some(home.join("AppData").join("Local")),
            app_data: Some(home.join("AppData").join("Roaming")),
            user_profile: Some(home),
        }
    }

    /// A fake home at `root/Home`, laid out as macOS does (`Home/Library/Caches` and
    /// `Home/Library/Application Support`), for tests. Nothing real is touched.
    #[cfg(target_os = "macos")]
    pub fn under(root: &Path) -> Self {
        Self::mac(root.join("Home"))
    }

    /// `HOME`, `CACHES` and `APPSUPPORT` are the macOS names for the same three folders.
    fn var(&self, name: &str) -> Option<&Path> {
        match name.to_ascii_uppercase().as_str() {
            "LOCALAPPDATA" | "CACHES" => self.local_app_data.as_deref(),
            "APPDATA" | "APPSUPPORT" => self.app_data.as_deref(),
            "USERPROFILE" | "HOME" => self.user_profile.as_deref(),
            _ => None,
        }
    }

    fn bases(&self) -> impl Iterator<Item = &Path> {
        [&self.local_app_data, &self.app_data, &self.user_profile].into_iter().flatten().map(PathBuf::as_path)
    }

    /// The base and the parts after it, or None if the variable is unknown or unset.
    fn split<'p>(&self, pattern: &'p str) -> Option<(PathBuf, Vec<&'p str>)> {
        let rest = pattern.strip_prefix('%')?;
        let (name, rest) = rest.split_once('%')?;
        let base = self.var(name)?.to_path_buf();
        let parts = rest.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
        Some((base, parts))
    }

    /// A pattern with no `*`, expanded without touching the disk.
    pub fn expand_plain(&self, pattern: &str) -> Option<PathBuf> {
        let (base, parts) = self.split(pattern)?;
        if parts.contains(&"*") {
            return None;
        }
        Some(parts.into_iter().fold(base, |p, s| p.join(s)))
    }

    /// Every folder a pattern names that exists now. A `*` part matches each real folder at
    /// that level; junctions and links are never matched.
    pub fn expand(&self, pattern: &str) -> Vec<PathBuf> {
        let Some((base, parts)) = self.split(pattern) else { return Vec::new() };
        let mut at = vec![base];
        for part in parts {
            if part == "*" {
                at = at
                    .iter()
                    .filter_map(|d| fs::read_dir(d).ok())
                    .flat_map(|rd| rd.flatten())
                    .filter(|e| e.metadata().is_ok_and(|m| m.is_dir() && !sys::is_reparse(&m)))
                    .map(|e| e.path())
                    .collect();
                at.sort();
            } else {
                at = at.into_iter().map(|p| p.join(part)).collect();
            }
        }
        at.retain(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()));
        at
    }

    /// The rule's folders that exist now, with any folder inside another one dropped.
    pub fn roots(&self, rule: &Rule) -> Vec<PathBuf> {
        let mut all: Vec<PathBuf> = rule.paths.iter().flat_map(|p| self.expand(p)).collect();
        all.sort_by_key(|p| key(p));
        all.dedup_by_key(|p| key(p));
        let keep: Vec<bool> = all.iter().map(|p| !all.iter().any(|q| !same(p, q) && is_within(p, q))).collect();
        all.into_iter().zip(keep).filter_map(|(p, k)| k.then_some(p)).collect()
    }
}

/// Lowercased parts, so comparisons ignore case and separators as Windows does.
pub(crate) fn key(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()
}

pub(crate) fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

/// `p` is `ancestor` or below it.
pub(crate) fn is_within(p: &Path, ancestor: &Path) -> bool {
    let (p, a) = (key(p), key(ancestor));
    p.len() >= a.len() && p[..a.len()] == a[..]
}

fn name_is(name: &OsStr, list: &[&str]) -> bool {
    let n = name.to_string_lossy();
    list.iter().any(|x| x.eq_ignore_ascii_case(&n))
}

/// A browser or app profile folder: Chromium `User Data`, `Default`, `Profile 2`,
/// `Guest Profile`, `System Profile`, WebView2 `WV2Profile_*`, or a Firefox profile.
fn is_profile_root(p: &Path) -> bool {
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { return false };
    let parent = p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase());
    matches!(name.as_str(), "user data" | "default" | "guest profile" | "system profile")
        || name.starts_with("profile ")
        || name.starts_with("wv2profile")
        || parent.as_deref() == Some("profiles")
}

pub(crate) fn is_normal(p: &Path) -> bool {
    let s = p.as_os_str().to_string_lossy();
    p.is_absolute()
        && p.parent().is_some()
        && !s.starts_with(r"\\?\")
        && !s.contains(['*', '?'])
        && !p.components().any(|c| matches!(c, Component::CurDir | Component::ParentDir))
}

/// Whether `path` is on the never-touch list: inside or holding a [`NEVER_TOUCH`] entry,
/// one of the [`NEVER_REMOVE_ROOTS`], a profile root, or through a folder named in
/// [`NEVER_NAMES`] or a credential store. Pure: reads no disk.
pub fn never_touch(env: &RuleEnv, path: &Path) -> bool {
    NeverTouch::new(env).holds(path)
}

/// The never-touch lists expanded once, for checking many paths (the duplicate finder's stage
/// 1). [`never_touch`] is this, built for one path.
pub(crate) struct NeverTouch {
    roots: Vec<Vec<String>>,
    touch: Vec<Vec<String>>,
    bases: Vec<Vec<String>>,
}

impl NeverTouch {
    pub(crate) fn new(env: &RuleEnv) -> Self {
        let expand = |list: &[&str]| list.iter().filter_map(|p| env.expand_plain(p)).map(|p| key(&p)).collect();
        NeverTouch { roots: expand(NEVER_REMOVE_ROOTS), touch: expand(NEVER_TOUCH), bases: env.bases().map(key).collect() }
    }

    pub(crate) fn holds(&self, path: &Path) -> bool {
        if !is_normal(path) {
            return true;
        }
        if path.components().any(|c| name_is(c.as_os_str(), NEVER_NAMES) || name_is(c.as_os_str(), SECRET_NAMES)) {
            return true;
        }
        if is_profile_root(path) {
            return true;
        }
        let k = key(path);
        let within = |p: &[String], a: &[String]| p.len() >= a.len() && p[..a.len()] == a[..];
        if self.roots.contains(&k) {
            return true;
        }
        // Holding a never-touch entry counts too: removing `.cargo` would take `.cargo\bin`.
        if self.touch.iter().any(|n| within(&k, n) || within(n, &k)) {
            return true;
        }
        // Anything holding a base folder (C:\Users, C:\Users\you) is never removed.
        self.bases.iter().any(|b| within(b, &k))
    }
}

/// The deepest base folder that holds `path`.
fn base_of<'e>(env: &'e RuleEnv, path: &Path) -> Option<&'e Path> {
    env.bases().filter(|b| is_within(path, b)).max_by_key(|b| b.components().count())
}

/// Whether the path, or any folder between its base folder and it, is a reparse point.
/// Parts that do not exist are not reparse points.
fn through_reparse(env: &RuleEnv, path: &Path) -> bool {
    let Some(base) = base_of(env, path) else { return true };
    let depth = base.components().count();
    let mut at = base.to_path_buf();
    for c in path.components().skip(depth) {
        at.push(c.as_os_str());
        match at.symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() || sys::is_reparse(&m) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Whether `path` may be offered or moved for `rule`. Checked when measuring and again just
/// before the move, because a folder can turn into a junction in between.
pub fn check_removal(env: &RuleEnv, rule: &Rule, path: &Path) -> Result<(), Refusal> {
    if !is_normal(path) {
        return Err(Refusal::NotNormal);
    }
    if never_touch(env, path) {
        return Err(Refusal::NeverTouch);
    }
    if !env.roots(rule).iter().any(|r| is_within(path, r)) {
        return Err(Refusal::OutsideRule);
    }
    if through_reparse(env, path) {
        return Err(Refusal::ReparsePoint);
    }
    Ok(())
}

/// One rule that has something to offer: its folders that exist, are not empty and passed
/// [`check_removal`], biggest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleFound {
    pub rule: &'static Rule,
    pub items: Vec<Candidate>,
}

impl RuleFound {
    pub fn total(&self) -> u64 {
        self.items.iter().map(|c| c.size).sum()
    }
}

/// What [`measure_rules`] found. `complete` is false when it was cancelled; `found` then holds
/// the rules finished before that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub found: Vec<RuleFound>,
    pub complete: bool,
}

fn holds_app_state(tree: &Tree) -> bool {
    let mut stack = vec![tree.root()];
    while let Some(id) = stack.pop() {
        if id != tree.root() && name_is(tree.node(id).name(), NEVER_NAMES) {
            return true;
        }
        stack.extend_from_slice(tree.children(id));
    }
    false
}

/// Measures each rule's folders with the scanner, in table order. Rules with nothing to
/// offer (missing, empty, refused) are left out. Blocks; [`RuleMeasure`] runs it on a thread.
pub fn measure_rules(env: &RuleEnv, rules: &'static [Rule], cancel: &AtomicBool) -> Measured {
    let options = ScanOptions { progress_every: usize::MAX, ..ScanOptions::default() };
    let mut found = Vec::new();
    for rule in rules {
        let mut items = Vec::new();
        for root in env.roots(rule) {
            if cancel.load(Ordering::Relaxed) {
                return Measured { found, complete: false };
            }
            if check_removal(env, rule, &root).is_err() {
                continue;
            }
            let Ok(tree) = scan(&root, &options, |_| {}, cancel) else { continue };
            if tree.status() == ScanStatus::Stopped && cancel.load(Ordering::Relaxed) {
                return Measured { found, complete: false };
            }
            if holds_app_state(&tree) {
                continue;
            }
            let node = tree.node(tree.root());
            if node.size() > 0 {
                items.push(Candidate { path: root, size: node.size(), newest: node.modified() });
            }
        }
        if !items.is_empty() {
            items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
            found.push(RuleFound { rule, items });
        }
    }
    Measured { found, complete: true }
}

/// [`measure_rules`] on its own thread. Dropping the handle cancels it.
pub struct RuleMeasure {
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Measured>,
}

impl RuleMeasure {
    /// Measures [`RULES`].
    pub fn start(env: RuleEnv) -> Self {
        Self::start_with(env, RULES)
    }

    pub fn start_with(env: RuleEnv, rules: &'static [Rule]) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let flag = cancel.clone();
        std::thread::spawn(move || {
            let _ = tx.send(measure_rules(&env, rules, &flag));
        });
        RuleMeasure { cancel, rx }
    }

    /// Stops promptly; the result then has `complete: false`.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The result once it is ready. Never blocks.
    pub fn try_take(&self) -> Option<Measured> {
        self.rx.try_recv().ok()
    }

    /// Blocks until done.
    pub fn wait(self) -> Option<Measured> {
        self.rx.recv().ok()
    }
}

impl Drop for RuleMeasure {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Whether `p` is, or holds, the user's profile or AppData folders or the Windows and
/// Program Files folders, read from the process environment. [`move_to_recycle_bin`] refuses
/// these for any caller, as a last line of defence.
pub(crate) fn holds_system_folder(p: &Path) -> bool {
    let env = RuleEnv::from_process();
    let extra = system_dirs();
    let held = env.bases().chain(extra.iter().map(PathBuf::as_path)).any(|b| is_within(b, p));
    #[cfg(target_os = "macos")]
    let held = held || crate::mac::refused_on_mac(p);
    held
}

/// The Windows, Program Files and ProgramData folders, read from the process environment.
#[cfg(not(target_os = "macos"))]
pub(crate) fn system_dirs() -> Vec<PathBuf> {
    ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

/// macOS: the system folders ([`crate::MAC_SYSTEM_DIRS`]) and `~/Library`, which is treated
/// like AppData.
#[cfg(target_os = "macos")]
pub(crate) fn system_dirs() -> Vec<PathBuf> {
    crate::mac::system_dirs(&RuleEnv::from_process())
}

/// Moves rule items to the Recycle Bin. Each path is checked again with [`check_removal`]
/// first; a refused path is reported as [`RecycleError::Refused`] and never reaches the shell.
pub fn recycle_rule_items(env: &RuleEnv, items: &[(&Rule, PathBuf)]) -> RecycleReport {
    let mut refused = Vec::new();
    let mut allowed = Vec::new();
    for (rule, path) in items {
        match check_removal(env, rule, path) {
            Ok(()) => allowed.push(path.clone()),
            Err(_) => refused.push((path.clone(), RecycleError::Refused)),
        }
    }
    let mut report = if allowed.is_empty() { RecycleReport::default() } else { move_to_recycle_bin(&allowed) };
    report.failed.extend(refused);
    report
}

/// Large files whose name says they are unfinished browser or app downloads.
pub fn is_unfinished_download(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|e| ["crdownload", "part", "tmp"].iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The label the Files list shows for those.
pub const UNFINISHED_DOWNLOAD: &str = "Unfinished download";

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(windows, target_os = "macos"))]
    fn env() -> RuleEnv {
        RuleEnv::under(&std::env::temp_dir().join("orby-rules-fake"))
    }

    #[test]
    fn every_rule_has_a_unique_id_and_plain_words() {
        let mut ids: Vec<_> = RULES.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), RULES.len());
        for r in RULES {
            assert!(!r.paths.is_empty(), "{}", r.id);
            assert!(r.explanation.ends_with('.'), "{}", r.id);
            assert!(r.explanation.len() <= 140, "{} explanation is one line", r.id);
            assert!(!r.explanation.to_lowercase().contains("junk"));
            for p in r.paths {
                #[cfg(not(target_os = "macos"))]
                assert!(p.starts_with("%LOCALAPPDATA%\\") || p.starts_with("%APPDATA%\\") || p.starts_with("%USERPROFILE%\\"), "{p}");
                #[cfg(target_os = "macos")]
                assert!(p.starts_with("%CACHES%/") || p.starts_with("%APPSUPPORT%/") || p.starts_with("%HOME%/"), "{p}");
            }
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    fn no_rule_path_is_on_the_never_touch_list() {
        let e = env();
        for r in RULES {
            for p in r.paths {
                let probe = p.replace('*', "SomeProfile");
                let path = e.expand_plain(&probe).unwrap();
                assert!(!never_touch(&e, &path), "{} offers a never-touch path {p}", r.id);
            }
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    fn rule_paths_never_overlap() {
        let e = &env();
        let all: Vec<(&str, PathBuf)> =
            RULES.iter().flat_map(|r| r.paths.iter().map(move |p| (r.id, e.expand_plain(&p.replace('*', "X")).unwrap()))).collect();
        for (i, (a, pa)) in all.iter().enumerate() {
            for (b, pb) in &all[i + 1..] {
                assert!(!is_within(pa, pb) && !is_within(pb, pa), "{a} and {b} overlap");
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn the_spec_never_list_is_refused() {
        let e = env();
        let h = e.user_profile.clone().unwrap();
        let l = e.local_app_data.clone().unwrap();
        let a = e.app_data.clone().unwrap();
        for p in [
            h.clone(),
            l.clone(),
            a.clone(),
            h.join(".gradle"),
            h.join(".cache"),
            h.join(".cargo"),
            h.join(r".cargo\bin"),
            h.join(r".cargo\bin\cargo.exe"),
            h.join(r".cargo\credentials.toml"),
            h.join(r".cache\huggingface\token"),
            h.join(".ssh"),
            l.join(r"Google\Chrome\User Data"),
            l.join(r"Google\Chrome\User Data\Default"),
            l.join(r"Google\Chrome\User Data\Profile 2"),
            l.join(r"Google\Chrome\User Data\Default\Service Worker"),
            l.join(r"Google\Chrome\User Data\Default\Service Worker\CacheStorage"),
            l.join(r"Google\Chrome\User Data\Default\IndexedDB"),
            l.join(r"Google\Chrome\User Data\Default\Local Storage"),
            l.join(r"Google\Chrome\User Data\Default\Network\Cookies"),
            l.join(r"Mozilla\Firefox\Profiles\abcd.default-release"),
            a.join(r"Microsoft\Credentials"),
            a.join("NuGet"),
            h.parent().unwrap().to_path_buf(),
            h.join("AppData"),
        ] {
            assert!(never_touch(&e, &p), "{} should be refused", p.display());
        }
        assert!(!never_touch(&e, &h.join(r".cargo\registry\cache")));
        assert!(!never_touch(&e, &h.join(r".gradle\caches")));
        assert!(!never_touch(&e, &l.join(r"Mozilla\Firefox\Profiles\abcd.default-release\cache2")));
    }

    #[test]
    #[cfg(windows)]
    fn odd_paths_are_refused() {
        let e = env();
        let h = e.user_profile.clone().unwrap();
        for p in [PathBuf::from(r"relative\x"), h.join(r".cargo\registry\cache\..\..\bin"), h.join(r".cargo\registry\*")] {
            assert!(never_touch(&e, &p), "{}", p.display());
        }
    }

    #[test]
    #[cfg(windows)]
    fn unknown_or_unset_variables_expand_to_nothing() {
        let e = RuleEnv { user_profile: None, ..env() };
        assert_eq!(e.expand_plain(r"%USERPROFILE%\.cargo\git"), None);
        assert_eq!(e.expand_plain(r"%WINDIR%\Temp"), None);
        assert_eq!(e.expand_plain(r"no-var\x"), None);
        assert!(e.expand(r"%USERPROFILE%\.cargo\git").is_empty());
    }

    #[test]
    fn unfinished_downloads_are_named() {
        for n in ["Win11.iso.crdownload", "movie.mkv.part", "setup.TMP"] {
            assert!(is_unfinished_download(OsStr::new(n)), "{n}");
        }
        for n in ["movie.mkv", "partial.zip", "tmp", "notes.txt"] {
            assert!(!is_unfinished_download(OsStr::new(n)), "{n}");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn the_mac_never_list_is_refused() {
        let e = env();
        let h = e.user_profile.clone().unwrap();
        let c = e.local_app_data.clone().unwrap();
        let a = e.app_data.clone().unwrap();
        assert_eq!(c, h.join("Library/Caches"));
        assert_eq!(a, h.join("Library/Application Support"));
        for p in [
            h.clone(),
            c.clone(),
            a.clone(),
            h.join("Library"),
            h.join("Desktop"),
            h.join("Documents"),
            h.join(".Trash"),
            h.join(".Trash/old.zip"),
            h.join("Library/Keychains/login.keychain-db"),
            h.join("Library/Mobile Documents/com~apple~CloudDocs/a.pdf"),
            h.join("Library/CloudStorage/Dropbox/x"),
            a.join("MobileSync/Backup/1234"),
            h.join(".ssh"),
            h.join(".cargo/bin/cargo"),
            h.join("Library/Developer/Xcode"),
            a.join("Google/Chrome/Default"),
            a.join("Google/Chrome/Default/Cookies"),
            a.join("Google/Chrome/Default/IndexedDB"),
            c.join("Firefox/Profiles/abcd.default-release"),
            h.parent().unwrap().to_path_buf(),
        ] {
            assert!(never_touch(&e, &p), "{} should be refused", p.display());
        }
        assert!(!never_touch(&e, &h.join("Library/Developer/Xcode/DerivedData")));
        assert!(!never_touch(&e, &c.join("Homebrew")));
        assert!(!never_touch(&e, &c.join("Firefox/Profiles/abcd.default-release/cache2")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn mac_system_dirs_hold_library_and_applications() {
        let d = system_dirs();
        for p in ["/System", "/Library", "/usr", "/private", "/Applications"] {
            assert!(d.iter().any(|x| x == Path::new(p)), "{p}");
        }
    }
}
