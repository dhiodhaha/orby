//! macOS policy: the cleaning-rule table, the never-touch lists, the system folders, packages
//! and the Trash. Only built on macOS; [`crate::rules`] re-exports the tables under the same
//! names as the Windows ones, so callers do not change.
//!
//! The rows are Orby's own, written for macOS. Conservative beats clever: a folder is listed only
//! when its owner documents it as rebuilt by itself.

use crate::clean::Tag::{CheckFirst, SafeToRemove};
use crate::rules::{is_within, Group::*, Rule, RuleEnv};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Every cleaning rule, in panel order within each group.
pub static RULES: &[Rule] = &[
    // ---- Developer caches ----
    Rule {
        id: "npm-cache",
        group: DeveloperCaches,
        name: "npm cache",
        paths: &["%HOME%/.npm/_cacache", "%HOME%/.npm/_npx"],
        tier: SafeToRemove,
        explanation: "Packages npm already downloaded. Re-downloaded on your next install.",
    },
    Rule {
        id: "yarn-cache",
        group: DeveloperCaches,
        name: "Yarn cache",
        paths: &["%CACHES%/Yarn", "%HOME%/.yarn/berry/cache"],
        tier: SafeToRemove,
        explanation: "Packages Yarn already downloaded. Re-downloaded on your next install.",
    },
    Rule {
        id: "pip-cache",
        group: DeveloperCaches,
        name: "pip cache",
        paths: &["%CACHES%/pip"],
        tier: SafeToRemove,
        explanation: "Python packages pip already downloaded. Re-downloaded the next time you install them.",
    },
    Rule {
        id: "uv-cache",
        group: DeveloperCaches,
        name: "uv cache",
        paths: &["%HOME%/.cache/uv"],
        tier: SafeToRemove,
        explanation: "Python packages uv already downloaded. Re-downloaded on your next uv sync.",
    },
    Rule {
        id: "cargo-registry",
        group: DeveloperCaches,
        name: "Cargo registry",
        paths: &["%HOME%/.cargo/registry/cache", "%HOME%/.cargo/registry/src"],
        tier: SafeToRemove,
        explanation: "Crates Cargo downloaded and unpacked. Re-downloaded on your next build. Installed tools stay.",
    },
    Rule {
        id: "cargo-git",
        group: DeveloperCaches,
        name: "Cargo git checkouts",
        paths: &["%HOME%/.cargo/git"],
        tier: SafeToRemove,
        explanation: "Git dependencies Cargo checked out. Fetched again on your next build.",
    },
    Rule {
        id: "xcode-derived-data",
        group: DeveloperCaches,
        name: "Xcode DerivedData",
        paths: &["%HOME%/Library/Developer/Xcode/DerivedData"],
        tier: SafeToRemove,
        explanation: "Build products and indexes Xcode made. Rebuilt on your next build, which is slower once.",
    },
    Rule {
        id: "homebrew-cache",
        group: DeveloperCaches,
        name: "Homebrew downloads",
        paths: &["%CACHES%/Homebrew"],
        tier: SafeToRemove,
        explanation: "Downloads Homebrew kept after installing. Fetched again only if you reinstall something.",
    },
    Rule {
        id: "gradle-wrapper",
        group: DeveloperCaches,
        name: "Gradle wrapper downloads",
        paths: &["%HOME%/.gradle/wrapper/dists"],
        tier: SafeToRemove,
        explanation: "Gradle versions the wrapper downloaded. Re-downloaded on your next build.",
    },
    Rule {
        id: "gradle-caches",
        group: DeveloperCaches,
        name: "Gradle caches",
        paths: &["%HOME%/.gradle/caches"],
        tier: CheckFirst,
        explanation: "Offline builds need it. The next online build downloads every dependency again.",
    },
    Rule {
        id: "go-build",
        group: DeveloperCaches,
        name: "Go build cache",
        paths: &["%CACHES%/go-build"],
        tier: SafeToRemove,
        explanation: "Compiled Go packages. Rebuilt on your next build, which is slower once.",
    },
    Rule {
        id: "nuget-packages",
        group: DeveloperCaches,
        name: "NuGet packages",
        paths: &["%HOME%/.nuget/packages"],
        tier: CheckFirst,
        explanation: "Every package your .NET projects use. Restored on the next build, which needs the internet and takes a while.",
    },
    Rule {
        id: "maven-repository",
        group: DeveloperCaches,
        name: "Maven repository",
        paths: &["%HOME%/.m2/repository"],
        tier: CheckFirst,
        explanation: "Libraries your Java builds use. Downloaded ones come back on the next build; ones you built with mvn install do not.",
    },
    Rule {
        id: "pnpm-store",
        group: DeveloperCaches,
        name: "pnpm store",
        paths: &["%HOME%/Library/pnpm/store", "%HOME%/.pnpm-store"],
        tier: CheckFirst,
        explanation:
            "Packages your pnpm projects share through hard links, so it may free less than shown. Re-downloaded on your next install.",
    },
    // ---- App caches ----
    Rule {
        id: "vscode-cache",
        group: AppCaches,
        name: "VS Code caches",
        paths: &["%APPSUPPORT%/Code/Cache", "%APPSUPPORT%/Code/CachedData", "%APPSUPPORT%/Code/CachedExtensionVSIXs"],
        tier: SafeToRemove,
        explanation: "Cached web content, compiled code and extension downloads. VS Code rebuilds them; its next start is a little slower.",
    },
    Rule {
        id: "discord-cache",
        group: AppCaches,
        name: "Discord cache",
        paths: &["%APPSUPPORT%/discord/Cache", "%APPSUPPORT%/discord/Code Cache"],
        tier: SafeToRemove,
        explanation: "Images and code Discord saved. Loaded again as you use it. You stay signed in.",
    },
    Rule {
        id: "slack-cache",
        group: AppCaches,
        name: "Slack cache",
        paths: &["%APPSUPPORT%/Slack/Cache", "%APPSUPPORT%/Slack/Code Cache"],
        tier: SafeToRemove,
        explanation: "Images and code Slack saved. Loaded again as you use it. You stay signed in.",
    },
    // ---- Browser caches ----
    Rule {
        id: "firefox-cache",
        group: BrowserCaches,
        name: "Firefox cache",
        paths: &["%CACHES%/Firefox/Profiles/*/cache2"],
        tier: SafeToRemove,
        explanation: "Pages Firefox saved to load faster. Sites load a bit slower for a day. You stay signed in.",
    },
    // ---- AI models ----
    Rule {
        id: "huggingface-hub",
        group: AiModels,
        name: "Hugging Face downloads",
        paths: &["%HOME%/.cache/huggingface/hub"],
        tier: CheckFirst,
        explanation: "Models and datasets you downloaded. Getting them again can mean many gigabytes. Your sign-in token stays.",
    },
    Rule {
        id: "ollama-models",
        group: AiModels,
        name: "Ollama models",
        paths: &["%HOME%/.ollama/models"],
        tier: CheckFirst,
        explanation: "Local models you pulled. Pull them again to use them, which can take a long time.",
    },
    // ---- Crash reports ----
    Rule {
        id: "crash-reports",
        group: CrashReports,
        name: "App crash reports",
        paths: &["%HOME%/Library/Logs/DiagnosticReports"],
        tier: SafeToRemove,
        explanation: "Reports macOS saved when apps crashed, only useful for debugging those crashes. Nothing comes back.",
    },
];

/// Never removed, nor anything inside them, nor any folder that holds them: tools you
/// installed, keys, tokens, passwords, phone backups, cloud folders and the Trash itself.
pub static NEVER_TOUCH: &[&str] = &[
    "%HOME%/.cargo/bin",
    "%HOME%/.cargo/credentials",
    "%HOME%/.cargo/credentials.toml",
    "%HOME%/.ssh",
    "%HOME%/.gnupg",
    "%HOME%/.aws",
    "%HOME%/.azure",
    "%HOME%/.kube",
    "%HOME%/.docker",
    "%HOME%/.git-credentials",
    "%HOME%/.npmrc",
    "%HOME%/.gradle/gradle.properties",
    "%HOME%/.m2/settings.xml",
    "%HOME%/.m2/settings-security.xml",
    "%HOME%/.cache/huggingface/token",
    "%HOME%/.cache/huggingface/stored_tokens",
    "%HOME%/.ollama/id_ed25519",
    "%HOME%/.ollama/id_ed25519.pub",
    "%HOME%/.Trash",
    "%HOME%/Library/Keychains",
    "%HOME%/Library/Mobile Documents",
    "%HOME%/Library/CloudStorage",
    "%HOME%/Library/Mail",
    "%HOME%/Library/Messages",
    "%APPSUPPORT%/MobileSync",
];

/// Folders that are never removed themselves, although a rule may offer something inside
/// them: home, `~/Library` and its two bases, the user's own top folders, and each tool's home
/// folder.
pub static NEVER_REMOVE_ROOTS: &[&str] = &[
    "%HOME%",
    "%APPSUPPORT%",
    "%CACHES%",
    "%HOME%/Library",
    "%HOME%/Library/Developer",
    "%HOME%/Library/Developer/Xcode",
    "%HOME%/Library/Logs",
    "%HOME%/Library/pnpm",
    "%HOME%/Applications",
    "%HOME%/Desktop",
    "%HOME%/Documents",
    "%HOME%/Downloads",
    "%HOME%/Movies",
    "%HOME%/Music",
    "%HOME%/Pictures",
    "%HOME%/Public",
    "%HOME%/.npm",
    "%HOME%/.yarn",
    "%HOME%/.yarn/berry",
    "%HOME%/.gradle",
    "%HOME%/.gradle/wrapper",
    "%HOME%/.cache",
    "%HOME%/.cache/huggingface",
    "%HOME%/.cargo",
    "%HOME%/.cargo/registry",
    "%HOME%/.nuget",
    "%HOME%/.m2",
    "%HOME%/.ollama",
    "%CACHES%/Firefox",
    "%CACHES%/Firefox/Profiles",
    "%APPSUPPORT%/Code",
    "%APPSUPPORT%/discord",
    "%APPSUPPORT%/Slack",
];

/// Folders macOS owns. The duplicate finder never looks inside them, and nothing that holds
/// one is ever moved. `/Applications` is here because an app bundle is not a set of user files
/// to pick apart.
pub const MAC_SYSTEM_DIRS: &[&str] = &["/System", "/Library", "/usr", "/bin", "/sbin", "/private", "/Applications", "/opt", "/cores"];

/// Protected by System Integrity Protection: nothing inside is ever offered to the Trash.
/// `/usr/local` is the one user-writable part of `/usr`.
const SIP_DIRS: &[&str] = &["/System", "/bin", "/sbin", "/usr"];

/// Folder extensions macOS shows as one item (apps, bundles, libraries and documents saved as
/// folders). The duplicate finder never looks inside one, and nothing inside one is moved.
pub const MAC_PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "xpc",
    "prefpane",
    "saver",
    "qlgenerator",
    "mdimporter",
    "dsym",
    "docset",
    "pkg",
    "mpkg",
    "photoslibrary",
    "photolibrary",
    "migratedphotolibrary",
    "aplibrary",
    "imovielibrary",
    "tvlibrary",
    "musiclibrary",
    "theater",
    "fcpbundle",
    "logicx",
    "band",
    "lrdata",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "xcodeproj",
    "xcworkspace",
    "xcarchive",
    "playground",
    "sparsebundle",
    "scptd",
];

/// Caches in `~/Library/Caches` that macOS itself keeps, left out of Safe to clean beside every
/// `com.apple.*` one.
const APPLE_CACHES: &[&str] = &["CloudKit", "GeoServices", "Metadata", "PassKit", "FamilyCircle", "SiriTTS", "Apple"];

/// The macOS system folders and `~/Library`.
pub fn system_dirs(env: &RuleEnv) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = MAC_SYSTEM_DIRS.iter().map(PathBuf::from).collect();
    v.extend(env.user_profile.as_ref().map(|h| h.join("Library")));
    v
}

/// Whether a folder with this name is a package, by extension (case does not matter).
pub fn is_package_name(name: &OsStr) -> bool {
    Path::new(name).extension().and_then(OsStr::to_str).is_some_and(|e| MAC_PACKAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` sits inside a package (a folder on the way has a package extension). The
/// package itself is not inside one.
pub fn inside_package(path: &Path) -> bool {
    path.parent().is_some_and(|p| p.components().any(|c| matches!(c, Component::Normal(n) if is_package_name(n))))
}

fn in_trash(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(n) if n == ".Trash" || n == ".Trashes"))
}

/// The macOS last line of defence in `move_to_recycle_bin`, beside holding a system folder:
/// `.` or `..` in the path, inside a package, in a Trash already, or under a SIP folder.
pub fn refused_on_mac(p: &Path) -> bool {
    if p.components().any(|c| matches!(c, Component::CurDir | Component::ParentDir)) {
        return true;
    }
    if inside_package(p) || in_trash(p) {
        return true;
    }
    SIP_DIRS.iter().any(|d| is_within(p, Path::new(d))) && !is_within(p, Path::new("/usr/local"))
}

/// Whether a top-level item of `~/Library/Caches` is offered under Safe to clean: not Apple's
/// own, and not a cache a cleaning rule already offers (so nothing is counted twice).
pub fn cache_item_offered(name: &OsStr) -> bool {
    let n = name.to_string_lossy();
    if n.starts_with("com.apple.") || n.starts_with('.') || APPLE_CACHES.iter().any(|a| a.eq_ignore_ascii_case(&n)) {
        return false;
    }
    !RULES.iter().flat_map(|r| r.paths.iter()).filter_map(|p| p.strip_prefix("%CACHES%/")).any(|rest| {
        let first = rest.split('/').next().unwrap_or(rest);
        first.eq_ignore_ascii_case(&n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_are_found_by_extension_and_only_their_insides_count() {
        for n in ["Safari.app", "Photos Library.photoslibrary", "Movie.fcpbundle", "Song.logicx", "X.APP", "Report.pages"] {
            assert!(is_package_name(OsStr::new(n)), "{n}");
        }
        for n in ["notes.txt", "app", "Applications", "photos"] {
            assert!(!is_package_name(OsStr::new(n)), "{n}");
        }
        assert!(inside_package(Path::new("/Applications/Safari.app/Contents/MacOS/Safari")));
        assert!(inside_package(Path::new("/Users/me/Pictures/Photos Library.photoslibrary/originals/a.heic")));
        assert!(!inside_package(Path::new("/Users/me/Downloads/Foo.app")), "the package itself is one item");
        assert!(!inside_package(Path::new("/Users/me/Downloads/report.pages")));
    }

    #[test]
    fn the_last_line_of_defence_refuses_packages_trash_and_sip() {
        for p in [
            "/Applications/Safari.app/Contents/Info.plist",
            "/Users/me/.Trash/old.zip",
            "/Users/me/.Trash",
            "/Volumes/USB/.Trashes/501/x",
            "/System/Library/Fonts/x.ttf",
            "/usr/lib/libz.dylib",
            "/bin/ls",
            "/Users/me/Downloads/../Documents/x",
        ] {
            assert!(refused_on_mac(Path::new(p)), "{p}");
        }
        for p in ["/Users/me/Downloads/x.dmg", "/usr/local/Cellar/x", "/Applications/Foo.app", "/Users/me/Library/Caches/Foo"] {
            assert!(!refused_on_mac(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn library_caches_offers_app_caches_but_not_apple_or_rule_caches() {
        for n in ["com.spotify.client", "com.tinyspeck.slackmacgap", "JetBrains"] {
            assert!(cache_item_offered(OsStr::new(n)), "{n}");
        }
        for n in ["com.apple.Safari", "CloudKit", "Homebrew", "pip", "go-build", "Yarn", "Firefox", ".DS_Store"] {
            assert!(!cache_item_offered(OsStr::new(n)), "{n}");
        }
    }

    #[test]
    fn system_dirs_include_home_library() {
        let env = RuleEnv::under(Path::new("/tmp/fake"));
        let d = system_dirs(&env);
        assert!(d.contains(&PathBuf::from("/tmp/fake/Home/Library")));
        assert!(d.contains(&PathBuf::from("/Applications")));
    }
}
