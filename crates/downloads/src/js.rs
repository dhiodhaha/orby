//! A JavaScript runtime for yt-dlp's YouTube extraction (yt-dlp wiki, "EJS").
//!
//! Without one yt-dlp warns "No supported JavaScript runtime could be found", some formats go
//! missing and it falls back to fragile ones, so downloads fail now and then. yt-dlp enables only
//! deno by default and only finds it on PATH; we hand it the runtime we found with
//! `--js-runtimes NAME:PATH`. The official yt-dlp.exe bundles the scripts, so nothing else is fetched.

use std::env::consts::EXE_SUFFIX;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The runtimes yt-dlp supports, in its own order of priority (quickjs left out: rarely installed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsKind {
    Deno,
    /// Node 22 or later; an older one is ignored by yt-dlp with a warning.
    Node,
    /// Deprecated by yt-dlp, kept as the last resort.
    Bun,
}

impl JsKind {
    pub const ALL: [JsKind; 3] = [JsKind::Deno, JsKind::Node, JsKind::Bun];

    /// The name `--js-runtimes` takes, which is also the program's file stem.
    pub fn name(self) -> &'static str {
        match self {
            JsKind::Deno => "deno",
            JsKind::Node => "node",
            JsKind::Bun => "bun",
        }
    }

    fn file(self) -> String {
        format!("{}{EXE_SUFFIX}", self.name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsRuntime {
    pub kind: JsKind,
    pub path: PathBuf,
}

impl JsRuntime {
    /// The value for `--js-runtimes`, for example `node:C:\Program Files\nodejs\node.exe`.
    pub fn arg(&self) -> OsString {
        let mut s = OsString::from(self.kind.name());
        s.push(":");
        s.push(&self.path);
        s
    }
}

/// deno next to yt-dlp (`bin`) first, then deno, node and bun on `path_var` (the PATH value).
pub fn find_js_runtime(bin: &Path, path_var: Option<&OsStr>) -> Option<JsRuntime> {
    let ours = bin.join(JsKind::Deno.file());
    if ours.is_file() {
        return Some(JsRuntime { kind: JsKind::Deno, path: ours });
    }
    let dirs: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(p).filter(|d| !is_session_only(d)).collect())
        .unwrap_or_default();
    JsKind::ALL.into_iter().find_map(|kind| {
        dirs.iter().map(|d| d.join(kind.file())).find(|f| f.is_file()).map(|path| JsRuntime { kind, path })
    })
}

/// fnm puts a per-shell link folder on PATH that is gone once that terminal closes.
fn is_session_only(dir: &Path) -> bool {
    dir.to_string_lossy().to_ascii_lowercase().contains("fnm_multishells")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("app-downloads-js-tests").join(name);
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(d: &Path, kind: JsKind) -> PathBuf {
        let f = d.join(kind.file());
        fs::write(&f, b"").unwrap();
        f
    }

    #[test]
    fn deno_next_to_yt_dlp_wins() {
        let bin = dir("ours-bin");
        let path_dir = dir("ours-path");
        let deno = touch(&bin, JsKind::Deno);
        touch(&path_dir, JsKind::Node);
        let path = std::env::join_paths([&path_dir]).unwrap();
        assert_eq!(find_js_runtime(&bin, Some(&path)), Some(JsRuntime { kind: JsKind::Deno, path: deno }));
    }

    #[test]
    fn on_path_deno_beats_node_beats_bun_whatever_the_folder_order() {
        let bin = dir("order-bin");
        let (a, b) = (dir("order-a"), dir("order-b"));
        touch(&a, JsKind::Bun);
        let node = touch(&a, JsKind::Node);
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_js_runtime(&bin, Some(&path)).map(|r| r.path), Some(node));
        let deno = touch(&b, JsKind::Deno);
        assert_eq!(find_js_runtime(&bin, Some(&path)).map(|r| r.path), Some(deno));
    }

    #[test]
    fn nothing_found_and_session_only_folders_are_skipped() {
        let bin = dir("none-bin");
        assert_eq!(find_js_runtime(&bin, None), None);
        let fnm = dir("fnm_multishells").join("1234_5678");
        fs::create_dir_all(&fnm).unwrap();
        touch(&fnm, JsKind::Node);
        let path = std::env::join_paths([&fnm]).unwrap();
        assert_eq!(find_js_runtime(&bin, Some(&path)), None);
    }

    #[test]
    fn arg_is_name_colon_path() {
        let r = JsRuntime { kind: JsKind::Node, path: PathBuf::from(r"C:\Program Files\nodejs\node.exe") };
        assert_eq!(r.arg(), OsString::from(r"node:C:\Program Files\nodejs\node.exe"));
    }
}
