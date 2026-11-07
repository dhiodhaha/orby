# app-downloads

The downloads engine for the orb, built around a yt-dlp sidecar with ffmpeg (ORB_SPEC §6, build
step 7; design in `docs/design/downloads.md`). It has no UI code. The desktop app wires it to the
clipboard listener, the offer pill, the Downloads panel and the orb ring.

Everything is pure logic except `runner`, which spawns the process. The only time input is the
`now: SystemTime` passed to `Queue::apply`, which stamps Done rows ("10:42", "Done today").

## Modules and public API

**link**: which links are offered, in canonical form.
- `MediaLink::parse(text) -> Option<MediaLink>`, `is_media_link(text) -> bool`
- `MediaLink::site()`, `MediaLink::url()` (normalised, also the "Already saved" key)
- `Site { YouTube, YouTubeMusic, Instagram, TikTok, X, SoundCloud }`, `Site::name()`, `Site::default_kind()`
- Accepted: YouTube `watch?v=`, `youtu.be/`, `shorts/` (all become `https://www.youtube.com/watch?v=ID`);
  YouTube Music `watch?v=`; Instagram `reel/`, `reels/`, `p/`; TikTok `@user/video/ID`, `t/CODE`,
  `vm.` and `vt.` short links; X and Twitter `user/status/ID` (become `x.com`); SoundCloud
  `artist/track` (plus secret `s-` links and `on.soundcloud.com`). Tracking params, playlists, `t=`
  and fragments are dropped. Playlists, sets, profiles and other sites are not offered.

**format**: what to save.
- `Format::Video { max_height: Option<u32> }` (default `Some(1080)`, `None` is "Best"), `Format::Audio { codec: AudioCodec }`
- `AudioCodec { M4a (default), Mp3, Opus }`, `MediaKind { Video, Audio }`
- `Format::label()` gives "1080p MP4", "Best MP4", "M4A", "MP3", "Opus"
- `FormatPrefs { save_as, max_height, audio }` with `for_link(&link)` (music sites always start as audio), `video()`, `audio()`

**args**: the yt-dlp command line.
- `Request { link, format, folder }`, `build_args(&Request, ffmpeg: &Path) -> Vec<OsString>`
- Format selectors with `height<=N` (falls back to the next lower height), MP4 merge and remux, or
  `-x --audio-format`; `--newline`, `--no-playlist`, `--no-colors`, a JSON `--progress-template`
  (`[orb] {"title":..,"progress":..}`), `--ffmpeg-location`, `-P folder`,
  `-o "%(title).150B [%(id)s].%(ext)s"`, then `--` and the URL.

**parse**: reading yt-dlp output.
- `parse_line(&str) -> Line { Progress, Destination, AlreadyDownloaded, Error(ErrorKind), Other }`
- `classify_error(&str) -> ErrorKind { Private, Removed, Unsupported, DiskFull, Offline, FfmpegMissing, Other }`
- `Tracker::feed(line) -> Vec<Update>` adds up a merge's separate video and audio files into job
  progress; `Tracker::finish(exit_ok, size_of) -> Outcome { Done { path, size }, Failed(kind) }`
- `Update { Title, Progress(Progress { downloaded, total, speed, eta }), Destination, Finished(Outcome) }`

**queue**: the state machine.
- `JobState { Waiting, Reading, Running(Progress), Paused { .. }, Done { path, size, at }, Failed { kind, downloaded, total } }`
- `Queue::add`, `start_ready() -> Vec<(JobId, Request)>` (at most `MAX_RUNNING = 2` active),
  `apply(id, Update, now)`, `pause`, `resume`, `retry`, `retry_offline`, `cancel -> Cancelled { stop_process, delete }`,
  `set_format` (waiting only), `set_folder` (disk full: Change folder), `clear_done`, `saved(&link)`,
  `counts()`, `overall() -> Overall { files_done, files, bytes_done, bytes_total }` with `fraction()`,
  and `ring() -> Ring { Hidden, Reading, Progress { fraction, tone: Ink | Grey | Ember } }`.
- Updates for jobs that are no longer active are ignored, so a killed process cannot undo a pause.

**runner**: the only I/O.
- `Sidecar { yt_dlp, ffmpeg }`, `start(&Sidecar, &Request, on_update) -> io::Result<Download>`;
  `on_update` runs on a background thread and always ends with one `Update::Finished`.
- `Download::stop()` kills the process tree (`taskkill /T` on Windows, since the PyInstaller exe and
  ffmpeg are child processes). `delete_partial(&files)` removes a cancelled job's files and their `.part` and `.ytdl`.

## Typical wiring
1. Clipboard text: `MediaLink::parse`; if `queue.saved(&link)` is some, show "Already saved".
2. Save: `queue.add(link, prefs.for_link(&link), folder)`, then for each `queue.start_ready()` call
   `runner::start`, sending `(id, update)` to the UI thread, which calls `queue.apply(id, update, now)`
   and then `start_ready()` again.
3. Pause: `queue.pause(id)` then `download.stop()`. Cancel: `queue.cancel(id)`, stop if
   `stop_process`, then `delete_partial(&delete)`.

## Verified
`src/bin/fake-yt-dlp.rs` checks the arguments and replays recorded yt-dlp output (success with a
merge, private video, disk full, and a slow download for pause and cancel). The integration tests in
`tests/runner.rs` run it as a real child process through `runner::start` and the queue. Nothing real
is downloaded; test files live under `target/tmp`.

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"; $env:CARGO_TARGET_DIR = "C:\Users\dopamine\dev\clippy\crates\downloads\target"; Set-Location C:\Users\dopamine\dev\clippy\crates\downloads; cargo test; cargo clippy --all-targets -- -D warnings
```

Result: 40 tests pass (34 unit, 6 end to end), clippy clean with `-D warnings`.
