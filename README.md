# Orby

A small, quiet desktop helper for Windows: one **liquid-metal drop** that floats at the edge of your screen and grows a pill when something needs you.

Orby is built for one Indonesian Muslim power user and their family. It keeps prayer times, tasks, a calendar, focus, drinking water, downloads and disk space in one place, with almost no memory and no CPU at rest. It is a native Rust app, not a web view.

> **Status: early, working prototype.** The core is built and runs on Windows 11. Sync, sign-in and a Mac version are not done yet. See [Roadmap](#roadmap).

## The orb

- **A 48 px mercury drop** drawn with a real liquid-metal shader, always on top, never taking focus. Clicks outside the circle pass through to the app underneath.
- **Drag it** anywhere, and it snaps to the nearest screen edge. After 3 seconds it tucks half off the edge, and it slides out when your pointer comes near.
- **The highlight follows the sun.** It moves with the day's prayer times and dims after Isya. At rest it is a still frame, and nothing loops.
- **States are small marks, not decoration:** a thin ring while something runs, a dotted ring for focus time, a red dot while recording, a badge for waiting notices.
- **Pills** grow out of the drop with a springy bounce, carry one action at most, and go away on their own.
- **A half dial** opens on click or `Ctrl+Alt+Space`: Tasks, Today, Prayer, Focus, Drink, More. Aim by direction, or use the keys. Pressing the shortcut again hides the orb.

## What it does today

| Feature | What works |
|---|---|
| **Prayer** | Kemenag (Subuh 20°, Isya 18°) times calculated offline and checked against the official Bimas Islam schedule for Kota Yogyakarta. Arc with the Isya moon, live countdown, Qibla bearing and distance, reminders at −30 min, −10 min and "Memasuki waktu". Prayer is never a task: no checkbox, no streak, no history. |
| **Focus** | Pomodoro timer: 25 / 5 / 15 min, session dots, a held-notices queue, pause when prayer time arrives. |
| **Drink** | Daily goal, quick glasses, undo, 7-day bars, smart reminders that skip when you are away, catch up when you are behind, and a puasa mode (Ramadan, Senin and Kamis). |
| **Tasks** | Local task store with nested folders and lists, a quick-add line that reads Indonesian and English ("beli galon besok 9am #belanja @raka"), due pills, and an overdue badge. |
| **Today and Calendar** | One timeline of events, tasks and prayer markers. Calendar events come from `.ics` links, with repeating events and time zones. Join pills for Meet, Zoom and Teams links. |
| **Downloads** | Copy a video link and Orby offers to save it, through a yt-dlp helper (video or audio, a queue of two at a time, retry on error). |
| **Disk** | Low-space warning, a fast scanner, the biggest folders and files, file types, and a "Safe to clean" list. Removing anything only ever moves it to the Recycle Bin, after a confirm, with an undo. |
| **History** | Drink and focus only, in a contribution grid. Never prayer. |
| **Notices** | One queue for every pill, with priorities (urgent, normal, quiet), a stack, an "Earlier today" list, per-source mute, and quiet rules for full screen, presenting and focus. |
| **Settings and first run** | A short welcome, shortcut change, start with Windows, place and method, per-source mutes. |

## Design

Neutral white or graphite, one typeface, hairline edges, and real metal only on the orb. There is no colour accent: emphasis comes from ink weight. Ember red is kept for errors and recording.

- [`DESIGN.md`](DESIGN.md) is the source of truth for tokens, type, shapes, motion and the orb's behaviour.
- [`docs/ORB_SPEC.md`](docs/ORB_SPEC.md) is the product spec, with architecture and build order.
- [`docs/design/`](docs/design) holds one spec per feature.
- [`docs/research/`](docs/research) holds research notes.
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) holds the detailed per-feature build notes, tray ids and how each unit was checked.

In the design docs, `[App]` is the placeholder for the product name, which is Orby.

## Numbers

Measured on the dev PC (Windows 11, RTX 4060 Ti, i5-13600K, a 74 Hz display):

| | Result |
|---|---|
| App size | about 4.4 MB (release) |
| Memory | about 55 MB in use (a comparable native overlay used 97 MB) |
| CPU at rest | about 30 ms over 30 s, and no drawing while nothing moves |
| Animation | the display's full refresh rate (about 75 fps) |
| Tests | 389 Rust tests, 21 server tests, `clippy -D warnings` clean |

## Architecture

A Cargo workspace. The engine crates have no OS code, so a Mac or mobile app can reuse them.

```
crates/core        prayer times (Kemenag), Qibla
crates/habits      focus timer, held notices, drink pacing
crates/store       local-first tasks: SQLite, nested folders, spaces, per-field merge for sync
crates/quickadd    Indonesian and English quick-add parser
crates/calendar    ICS parsing, recurrence, time zones, the Today timeline
crates/downloads   link detection, yt-dlp arguments, progress parser, queue, runner
crates/disk        scanner, queries, safe-to-clean, move to Recycle Bin
apps/orb-proto     the Windows app: DirectComposition, Direct3D 11, Direct2D, DirectWrite
apps/server        Cloudflare Worker sync relay (TypeScript, Durable Objects): built and tested locally, not deployed
```

The orb and pills are drawn by hand on DirectComposition so they stay light, with a liquid-metal shader ported to HLSL. The app only renders while something moves and otherwise sleeps. Prayer reminders use one exact timer to the next event instead of polling.

## Build and run

You need Windows 11, [Rust](https://rustup.rs) (stable, MSVC) and the Visual Studio C++ Build Tools.

```
cargo build --release
target\release\orb-proto.exe
cargo test --workspace
```

- Build on Windows itself, not over a WSL path.
- Orby keeps its data in `%APPDATA%\orb-proto`. Set `ORB_PROTO_DATA` to use another folder, which is handy for testing.
- The server lives in `apps/server`: `npm install`, then `npm test`. It is not deployed, and nothing in this repo deploys it.
- Tray menu items let you fire test notices and open each panel.

### Safety

- Disk clean-up only moves files to the Recycle Bin, after a confirm. There is no permanent delete.
- Two disk tests move real files into the Recycle Bin, so they are opt in: `cargo test -p app-disk -- --ignored`.
- Downloads never fetch anything until you copy a link or click a button. The yt-dlp helper is downloaded only when you ask for it.

## Roadmap

**Up next**

- **Sync and sharing.** The server and the merge logic exist and are tested. Still to do: deploy, sign-in (Google or an email link), invites between PCs, and the family spaces UI.
- **Google Calendar sign-in**, read-only. Today the calendar reads `.ics` links.
- **A real-world check of Downloads** against a real yt-dlp. It has only been tested against a fake one.
- **Windows location** for prayer times, and fetching the official schedule when online to remove the last ±1 minute.
- **Adzan sound**, and pausing focus automatically at prayer time.
- **Follow-ups from verification.** About 18 small items are listed in [`docs/followups.md`](docs/followups.md), for example "starting now" firing for meetings that ended while the PC slept, scrolling for more than 8 waiting notices, and a stale percent on the hover pill.
- **The remaining settings sections** (Orb, Calendar, Focus, Drink, Downloads, Disk, Sounds) and a dark-theme pass on screen.

**Later**

- **macOS**, to run smoothly even on a MacBook Air M1 with 8 GB. The plan is in [`docs/ORB_SPEC.md`](docs/ORB_SPEC.md): OS code behind traits, and the metal shader written once and translated for Direct3D and Metal.
- **Mobile**, reusing the core and sync crates.
- **Meeting notes.** Designed, but postponed until real speech tests on Indonesian and English pass. The plan uses Groq (Whisper for speech, a small model for notes) and takes ideas from how Granola structures notes. If the tests fail, the feature is dropped.
- **Bundling the Inter typeface** instead of Segoe UI Variable.

## License

Apache License 2.0. See [`LICENSE`](LICENSE), [`NOTICE`](NOTICE) and [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

## Acknowledgements

Orby stands on other people's work. Full notices are in [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

- **[Inter](https://github.com/rsms/inter)** by Rasmus Andersson and the Inter Project Authors (SIL OFL 1.1): the typeface, bundled with the app.
- **[Paper Shaders](https://github.com/paper-design/shaders)** (Apache-2.0): the liquid-metal shader is a port of `liquidMetal`.
- **metal-fx** by Jakub Antalik (MIT): the silver preset the metal parameters follow.
- **[Libraries.dev](https://github.com/Jakubantalik/Libraries.dev)** by Jakub Antalik (MIT): the spring presets and the liquid "bud" idea, and a high bar for craft.
- **[Ovio](https://github.com/JanaSundar/ovio)** by Jana Sundar (MIT): the model for the dial menu, the knob and the contribution graph, reimplemented natively.
- **[Granola](https://www.granola.ai)**, **Poni** and **TreeSize**: inspiration for the floating orb, the notes pipeline and the disk scanner. Their behaviour was studied from the outside, and no code, prompts or assets were copied.
- **Kemenag RI / Bimas Islam**: the official prayer schedule used to check the calculation.
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)** (Unlicense): the downloader Orby drives. It is not bundled.
- **[pstack](https://github.com/backnotprop/pstack)** (MIT), a mirror of Poteto's agent skills: the workflow the build followed, with swarms of agents and independent verification.
- **[Claude Code](https://claude.com/claude-code)**: most of this code was written with it, and checked by separate agents that ran the real app.
- **[windows-rs](https://github.com/microsoft/windows-rs)**, **rusqlite**, **serde** and **Cloudflare Workers**: the foundations the app is built on.
