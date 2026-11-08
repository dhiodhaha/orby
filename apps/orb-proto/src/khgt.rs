//! Muhammadiyah's KHGT month starts from the official API, cached in the data folder as
//! `khgt-calendar.json` (the raw answer). Parsing lives in `app_core::khgt`.
//!
//! A worker thread asks at launch when the cache is missing or older than a week, then once a
//! day while the app runs. The request carries nothing about the user: no location, no account,
//! just the Hijri years. The built-in table stays as the offline fallback; fetched months win.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use app_core::hijri::{approx_year, HijriCalendar, Months};

const API: &str = "https://api-khgt.muhammadiyah.or.id/api/calendar";
const CACHE: &str = "khgt-calendar.json";
const STALE_AT_START: Duration = Duration::from_secs(7 * 24 * 3600);
const EVERY: Duration = Duration::from_secs(24 * 3600);

fn months() -> &'static RwLock<Months> {
    static M: OnceLock<RwLock<Months>> = OnceLock::new();
    M.get_or_init(|| RwLock::new(Months::builtin(HijriCalendar::Muhammadiyah)))
}

/// The Muhammadiyah months known now: built-in, merged with the last good fetch.
pub fn muhammadiyah() -> Months {
    months()
        .read()
        .map(|m| m.clone())
        .unwrap_or_else(|e| e.into_inner().clone())
}

fn set(text: &str) -> Result<usize, String> {
    let fetched = app_core::khgt::parse(text)?;
    let merged = Months::builtin(HijriCalendar::Muhammadiyah).merged(&fetched);
    let n = fetched.len();
    match months().write() {
        Ok(mut m) => *m = merged,
        Err(e) => *e.into_inner() = merged,
    }
    Ok(n)
}

fn log(msg: &str) {
    let path = crate::exe_dir().join("orb-proto.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{msg}");
    }
}

fn cache_age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    SystemTime::now().duration_since(modified).ok()
}

fn fetch(first: u32, last: u32) -> Result<String, String> {
    let tls = native_tls::TlsConnector::new()
        .map_err(|_| "secure connections are not available".to_string())?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent("Orby")
        .build();
    let url = format!("{API}/{first}?type=hijriah&end_year={last}");
    match agent.get(&url).call() {
        // into_string stops at 10 MB; three years are about 130 KB.
        Ok(r) => r
            .into_string()
            .map_err(|_| "the read stopped halfway".to_string()),
        Err(ureq::Error::Status(code, _)) => Err(format!("the API answered {code}")),
        Err(_) => Err("could not reach the API".to_string()),
    }
}

/// One fetch: last Hijri year, this one and the next. Writes the cache only when it parses.
fn refresh(cache: &Path) {
    let today = crate::prayer::today();
    let year = approx_year(today);
    let (first, last) = (year.saturating_sub(1), year + 1);
    let result = fetch(first, last).and_then(|text| {
        let n = set(&text)?;
        let tmp = cache.with_extension("json.tmp");
        if std::fs::write(&tmp, &text).is_ok() {
            let _ = std::fs::rename(&tmp, cache);
        }
        Ok(n)
    });
    match result {
        Ok(n) => {
            let date = muhammadiyah()
                .hijri_date(today)
                .map(|h| h.label())
                .unwrap_or_else(|| "unknown".into());
            log(&format!(
                "khgt: fetched {n} months for {first}-{last} H, today {date}"
            ));
        }
        Err(e) => log(&format!("khgt: fetch failed for {first}-{last} H: {e}")),
    }
}

/// At launch: the cached answer now, then the worker thread.
pub fn start() {
    let cache = crate::store::data_dir().join(CACHE);
    let cached = match std::fs::read_to_string(&cache) {
        Ok(text) => set(&text)
            .map_err(|e| log(&format!("khgt: cache ignored: {e}")))
            .is_ok(),
        Err(_) => false,
    };
    let _ = std::thread::Builder::new()
        .name("khgt".into())
        .spawn(move || {
            let stale = cache_age(&cache).is_none_or(|age| age > STALE_AT_START);
            if !cached || stale {
                refresh(&cache);
            }
            loop {
                std::thread::sleep(EVERY);
                refresh(&cache);
            }
        });
}
