//! Runs yt-dlp as a child process and streams its output through a [`Tracker`].
//!
//! This is the only module with I/O. Each download uses three threads: one per pipe reading
//! lines, and one that owns the tracker, calls back with each [`Update`], and reports the
//! outcome once the process has exited.

use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::args::{build_args, Request};
use crate::js::JsRuntime;
use crate::parse::{Outcome, Tracker, Update};

/// Where the bundled tools live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sidecar {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
    /// Handed to yt-dlp for YouTube ([`crate::find_js_runtime`]); skipped if it has gone since.
    pub js: Option<JsRuntime>,
}

/// A running download. Dropping it does not stop the process; call [`Download::stop`].
#[derive(Debug)]
pub struct Download {
    child: Arc<Mutex<Child>>,
}

impl Download {
    /// Kills the process (pause and cancel both do this). The part file stays on disk, and the
    /// callback still receives a final `Finished`, which the queue ignores for paused jobs.
    pub fn stop(&self) -> io::Result<()> {
        let mut child = lock(&self.child);
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        // The PyInstaller yt-dlp.exe is a bootloader with a Python child, and yt-dlp runs ffmpeg
        // as another child, so only killing the whole tree stops the download.
        #[cfg(windows)]
        {
            let killed = no_window(Command::new("taskkill").args(["/T", "/F", "/PID", &child.id().to_string()]))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if killed.is_ok_and(|s| s.success()) {
                return Ok(());
            }
        }
        child.kill()
    }
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Spawns yt-dlp for `req` and calls `on_update` from a background thread for every update,
/// ending with exactly one [`Update::Finished`]. Fails only when the process cannot start
/// (for example the sidecar is missing).
pub fn start<F>(sidecar: &Sidecar, req: &Request, mut on_update: F) -> io::Result<Download>
where
    F: FnMut(Update) + Send + 'static,
{
    let mut cmd = Command::new(&sidecar.yt_dlp);
    let js = sidecar.js.as_ref().filter(|j| j.path.is_file());
    cmd.args(build_args(req, &sidecar.ffmpeg, js))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // yt-dlp is Python; without these it writes in the ANSI code page when piped on Windows.
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    let mut child = no_window(&mut cmd).spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let child = Arc::new(Mutex::new(child));

    let (tx, rx) = mpsc::channel::<String>();
    read_lines(stdout, tx.clone());
    read_lines(stderr, tx);

    let waiter = Arc::clone(&child);
    thread::spawn(move || {
        let mut tracker = Tracker::new();
        let mut exited = None;
        // Lines until both pipes close. A leftover grandchild can hold a pipe open after the
        // process exits, so a quiet pipe after exit also ends the stream.
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(line) => tracker.feed(&line).into_iter().for_each(&mut on_update),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    exited = lock(&waiter).try_wait().ok().flatten();
                    if exited.is_some() {
                        break;
                    }
                }
            }
        }
        let status = match exited {
            Some(s) => Ok(s),
            None => wait(&waiter),
        };
        let exit_ok = status.as_ref().is_ok_and(|s| s.success());
        let outcome = tracker.finish(exit_ok, |p| fs::metadata(p).ok().map(|m| m.len()));
        if let Outcome::Failed(_) = outcome {
            let text = tracker.error_text().unwrap_or_else(|| match &status {
                Ok(s) => format!("yt-dlp ended with {s} and no error text"),
                Err(e) => format!("yt-dlp ended, exit status unknown ({e})"),
            });
            on_update(Update::ErrorText(text));
        }
        on_update(Update::Finished(outcome));
    });
    Ok(Download { child })
}

fn read_lines(pipe: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buf = Vec::new();
        while let Ok(n) = reader.read_until(b'\n', &mut buf) {
            if n == 0 {
                break;
            }
            // `\r` also ends a line in case a progress bar slips through without --newline.
            for part in String::from_utf8_lossy(&buf).split(['\r', '\n']).filter(|s| !s.is_empty()) {
                if tx.send(part.to_string()).is_err() {
                    return;
                }
            }
            buf.clear();
        }
    });
}

/// Polls instead of a blocking wait so [`Download::stop`] never waits on the lock.
fn wait(child: &Mutex<Child>) -> io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = lock(child).try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn lock(child: &Mutex<Child>) -> std::sync::MutexGuard<'_, Child> {
    child.lock().unwrap_or_else(|e| e.into_inner())
}

/// Deletes what a cancelled job left behind: each file it created plus its `.part` and `.ytdl`
/// companions. Files that are already gone are fine.
pub fn delete_partial(files: &[PathBuf]) -> io::Result<()> {
    for file in files {
        for path in [file.clone(), with_suffix(file, ".part"), with_suffix(file, ".ytdl")] {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}
