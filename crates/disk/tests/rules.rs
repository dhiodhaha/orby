#![cfg(windows)]
//! Cleaning rules against a fake profile under the crate's own target dir. Nothing here reads
//! the real profile or reaches the real Recycle Bin: every path handed to
//! `recycle_rule_items` is one the checks refuse.

use app_disk::{
    check_removal, measure_rules, never_touch, recycle_rule_items, Group, RecycleError, Refusal, Rule, RuleEnv, RuleMeasure, Tier, RULES,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rules").join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; bytes]).unwrap();
}

fn junction(link: &Path, target: &Path) {
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    let ok = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
    assert!(ok, "mklink /J failed");
}

fn rule(id: &str) -> &'static Rule {
    RULES.iter().find(|r| r.id == id).unwrap()
}

/// A profile with some caches, things that must never be offered, and an empty cache.
fn profile(name: &str) -> (PathBuf, RuleEnv) {
    let root = fresh(name);
    let env = RuleEnv::under(&root);
    let (h, l, a) = (env.user_profile.clone().unwrap(), env.local_app_data.clone().unwrap(), env.app_data.clone().unwrap());
    write(&l.join(r"npm-cache\_cacache\content-v2\sha512\aa"), 3000);
    write(&l.join(r"npm-cache\_npx\1234\package.json"), 1000);
    write(&l.join(r"npm-cache\_logs\debug.log"), 50_000);
    write(&h.join(r".cargo\registry\cache\index.crates.io\serde.crate"), 5000);
    write(&h.join(r".cargo\bin\cargo.exe"), 99_999);
    write(&h.join(r".cargo\credentials.toml"), 10);
    write(&h.join(r".gradle\caches\modules-2\x.jar"), 2000);
    write(&h.join(r".gradle\gradle.properties"), 10);
    fs::create_dir_all(l.join(r"pip\Cache")).unwrap();
    write(&l.join(r"Mozilla\Firefox\Profiles\abcd.default-release\cache2\entries\E1"), 700);
    write(&l.join(r"Mozilla\Firefox\Profiles\abcd.default-release\cookies.sqlite"), 4000);
    write(&a.join(r"discord\Cache\Cache_Data\f_000001"), 800);
    // A cache folder that turns out to hold app state is not offered at all.
    write(&a.join(r"Slack\Cache\Service Worker\CacheStorage\x"), 900);
    write(&h.join(r".cache\huggingface\hub\models--x\blob"), 6000);
    write(&h.join(r".cache\huggingface\token"), 10);
    (root, env)
}

#[test]
fn measures_only_rules_that_exist_and_are_not_empty() {
    let (_root, env) = profile("measure");
    let m = measure_rules(&env, RULES, &AtomicBool::new(false));
    assert!(m.complete);
    let got: Vec<(&str, u64, Tier, Group)> = m.found.iter().map(|f| (f.rule.id, f.total(), f.rule.tier, f.rule.group)).collect();
    assert_eq!(
        got,
        [
            ("npm-cache", 4000, Tier::SafeToRemove, Group::DeveloperCaches),
            ("cargo-registry", 5000, Tier::SafeToRemove, Group::DeveloperCaches),
            ("gradle-caches", 2000, Tier::CheckFirst, Group::DeveloperCaches),
            ("discord-cache", 800, Tier::SafeToRemove, Group::AppCaches),
            ("firefox-cache", 700, Tier::SafeToRemove, Group::BrowserCaches),
            ("huggingface-hub", 6000, Tier::CheckFirst, Group::AiModels),
        ]
    );
    let npm = &m.found[0];
    let l = env.local_app_data.clone().unwrap();
    assert_eq!(
        npm.items.iter().map(|c| (c.path.clone(), c.size)).collect::<Vec<_>>(),
        [(l.join(r"npm-cache\_cacache"), 3000), (l.join(r"npm-cache\_npx"), 1000)],
        "biggest first, and npm's logs are not part of the cache"
    );
    // Nothing offered is on the never-touch list.
    for f in &m.found {
        for c in &f.items {
            assert!(!never_touch(&env, &c.path), "{}", c.path.display());
            assert_eq!(check_removal(&env, f.rule, &c.path), Ok(()));
        }
    }
    assert!(env.user_profile.unwrap().join(r".cargo\bin\cargo.exe").exists(), "measuring never removes");
}

#[test]
fn removal_checks_refuse_never_touch_outside_root_and_odd_paths() {
    let (_root, env) = profile("refuse");
    let h = env.user_profile.clone().unwrap();
    let l = env.local_app_data.clone().unwrap();
    let cargo = rule("cargo-registry");
    assert_eq!(check_removal(&env, cargo, &h.join(r".cargo\registry\cache")), Ok(()));
    assert_eq!(check_removal(&env, cargo, &h.join(r".cargo\registry\cache\index.crates.io")), Ok(()), "inside the root is fine");
    assert_eq!(check_removal(&env, cargo, &h.join(r".cargo\bin")), Err(Refusal::NeverTouch));
    assert_eq!(check_removal(&env, cargo, &h.join(".cargo")), Err(Refusal::NeverTouch));
    assert_eq!(check_removal(&env, cargo, &h), Err(Refusal::NeverTouch));
    assert_eq!(check_removal(&env, cargo, &l.join(r"npm-cache\_cacache")), Err(Refusal::OutsideRule), "another rule's folder");
    assert_eq!(check_removal(&env, cargo, &h.join(r".cargo\registry\cache\..\..\bin")), Err(Refusal::NotNormal));
    assert_eq!(check_removal(&env, cargo, &h.join(r".cargo\registry\*")), Err(Refusal::NotNormal));
    let hf = rule("huggingface-hub");
    assert_eq!(check_removal(&env, hf, &h.join(r".cache\huggingface\token")), Err(Refusal::NeverTouch));
    assert_eq!(check_removal(&env, hf, &h.join(r".cache\huggingface")), Err(Refusal::NeverTouch));
    let ff = rule("firefox-cache");
    let profile = l.join(r"Mozilla\Firefox\Profiles\abcd.default-release");
    assert_eq!(check_removal(&env, ff, &profile.join("cache2")), Ok(()));
    assert_eq!(check_removal(&env, ff, &profile), Err(Refusal::NeverTouch), "a profile root");
}

/// A rule table with a mistake in it: the never-touch list still wins.
static BAD_RULES: &[Rule] = &[
    Rule {
        id: "whole-gradle",
        group: Group::DeveloperCaches,
        name: "Gradle home",
        paths: &[r"%USERPROFILE%\.gradle"],
        tier: Tier::SafeToRemove,
        explanation: "A wrong rule.",
    },
    Rule {
        id: "whole-cache",
        group: Group::DeveloperCaches,
        name: "Cache home",
        paths: &[r"%USERPROFILE%\.cache"],
        tier: Tier::SafeToRemove,
        explanation: "A wrong rule.",
    },
    Rule {
        id: "cargo-home",
        group: Group::DeveloperCaches,
        name: "Cargo home",
        paths: &[r"%USERPROFILE%\.cargo"],
        tier: Tier::SafeToRemove,
        explanation: "A wrong rule.",
    },
    Rule {
        id: "local-storage",
        group: Group::BrowserCaches,
        name: "Local Storage",
        paths: &[r"%LOCALAPPDATA%\Google\Chrome\User Data\Default\Local Storage"],
        tier: Tier::SafeToRemove,
        explanation: "A wrong rule.",
    },
];

#[test]
fn a_wrong_rule_cannot_widen_what_is_offered_or_moved() {
    let (_root, env) = profile("bad-rules");
    let l = env.local_app_data.clone().unwrap();
    write(&l.join(r"Google\Chrome\User Data\Default\Local Storage\leveldb\000003.log"), 500);
    let m = measure_rules(&env, BAD_RULES, &AtomicBool::new(false));
    assert!(m.complete);
    assert!(m.found.is_empty(), "{:?}", m.found.iter().map(|f| f.rule.id).collect::<Vec<_>>());
    let h = env.user_profile.clone().unwrap();
    let report = recycle_rule_items(
        &env,
        &[
            (&BAD_RULES[0], h.join(".gradle")),
            (&BAD_RULES[1], h.join(".cache")),
            (&BAD_RULES[2], h.join(r".cargo\bin")),
            (&BAD_RULES[3], l.join(r"Google\Chrome\User Data\Default\Local Storage")),
            (rule("npm-cache"), h.join(r".cargo\registry\cache")),
        ],
    );
    assert!(report.moved.is_empty());
    assert_eq!(report.failed.len(), 5);
    assert!(report.failed.iter().all(|(_, e)| *e == RecycleError::Refused), "{report:?}");
    assert!(h.join(r".gradle\gradle.properties").exists() && h.join(r".cargo\bin\cargo.exe").exists());
}

#[test]
fn junctions_are_never_measured_or_moved() {
    let (root, env) = profile("junction");
    let elsewhere = root.join("Elsewhere");
    write(&elsewhere.join(r"go-build\00\x-d"), 4000);
    write(&elsewhere.join(r"WER\ReportArchive\AppCrash_x\Report.wer"), 3000);
    let l = env.local_app_data.clone().unwrap();
    // The cache folder itself is a junction.
    junction(&l.join("go-build"), &elsewhere.join("go-build"));
    // A folder on the way is a junction: WER -> Elsewhere\WER, so WER\ReportArchive is a real
    // folder, but reached through it.
    junction(&l.join(r"Microsoft\Windows\WER"), &elsewhere.join("WER"));
    let wer = l.join(r"Microsoft\Windows\WER\ReportArchive");
    assert!(wer.symlink_metadata().unwrap().is_dir());
    let m = measure_rules(&env, RULES, &AtomicBool::new(false));
    let ids: Vec<&str> = m.found.iter().map(|f| f.rule.id).collect();
    assert!(!ids.contains(&"go-build") && !ids.contains(&"wer-archive"), "{ids:?}");
    assert!(check_removal(&env, rule("go-build"), &l.join("go-build")).is_err());
    assert_eq!(check_removal(&env, rule("wer-archive"), &wer), Err(Refusal::ReparsePoint));
    let report = recycle_rule_items(&env, &[(rule("go-build"), l.join("go-build")), (rule("wer-archive"), wer)]);
    assert!(report.moved.is_empty());
    assert_eq!(report.failed.len(), 2);
    assert!(report.failed.iter().all(|(_, e)| *e == RecycleError::Refused));
    assert!(elsewhere.join(r"go-build\00\x-d").exists() && elsewhere.join(r"WER\ReportArchive\AppCrash_x\Report.wer").exists());
}

#[test]
fn a_cancelled_measure_says_so_and_the_thread_gives_the_same_answer() {
    let (_root, env) = profile("cancel");
    let m = measure_rules(&env, RULES, &AtomicBool::new(true));
    assert!(!m.complete);
    assert!(m.found.is_empty());
    let sync = measure_rules(&env, RULES, &AtomicBool::new(false));
    let threaded = RuleMeasure::start(env.clone()).wait().unwrap();
    assert_eq!(threaded, sync);
    let h = RuleMeasure::start(env);
    h.cancel();
    let r = h.wait().unwrap();
    assert!(r.found.len() <= sync.found.len());
}

#[test]
fn the_process_environment_gives_absolute_bases() {
    let e = RuleEnv::from_process();
    for p in [&e.local_app_data, &e.app_data, &e.user_profile].into_iter().flatten() {
        assert!(p.is_absolute());
    }
}
