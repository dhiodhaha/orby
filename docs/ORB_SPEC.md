# Clippy orb: spec v0

A minimal floating orb for Windows. You can hide and show it with a shortcut, and it turns into a notification pill when it has something to say.

Sources this is based on (clean-room: behavior and numbers only, no copied code or assets):
- Granola "nub": `~/granolarea/research/05-nub-floating-orb.md` (window config, docking, hit-testing, prompt timings)
- Granola meeting detection: `~/granolarea/research/01-audio-capture-and-meeting-detection.md` §3
- Oyenza Studio / Poni (installer strings only; `~/oyennotech-rea` is empty): prayer times, yt-dlp downloader, clipboard link offer, full-screen silence, notification mirroring
- Arbor disk scanner: `~/treesize-rea/docs/BUILD_PLAN.md`

## 1. Window
- Native Rust (`windows-rs`). The orb is a layered window composed with **DirectComposition** and drawn with Direct2D/DirectWrite (see §9):
  - Window styles:
    - `WS_POPUP`, so there is no frame.
    - `WS_EX_TOPMOST`, so it stays on top.
    - `WS_EX_TOOLWINDOW`, so it has no taskbar button.
    - `WS_EX_NOREDIRECTIONBITMAP`, so DirectComposition draws the window and per-pixel alpha comes for free.
  - It is shown with `SW_SHOWNOACTIVATE`, plus `WS_EX_NOACTIVATE`, so it never steals focus.
  - Blur behind the orb and pill uses the real Windows backdrop:
    - Windows 11: `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)`, which is Acrylic.
    - Windows 10: the `SetWindowCompositionAttribute` accent fallback.
  - Click-through: toggle `WS_EX_TRANSPARENT` from the 33 ms hit-test loop (§4), or return `HTTRANSPARENT` from `WM_NCHITTEST` outside the circle.
- Collapsed orb: 48 px circle (v6.1) drawn inside a 68×68 transparent window, leaving room for the shadow and badge.
- The pill and panel are a **second** window anchored to the orb, the same pattern Granola uses for its popovers. That way the orb window never resizes, and resizing transparent windows is what causes flicker.
- Clamp the orb to the monitor's **work area**, not its full bounds. Granola clamps to full bounds and can end up covering the taskbar; we avoid that.

## 2. States
| State | Look | Entered by |
|---|---|---|
| Hidden | nothing on screen (tray icon only) | shortcut, tray "Hide", or full-screen app |
| Idle | orb at 40% opacity, half tucked into the edge | 3 s after the pointer leaves |
| Hover | slides out, 100% opacity | pointer within 20 px of the orb's edge |
| Menu | **dial menu** (ported from Ovio): a half-dial facing the screen with Tasks · Today · Prayer · Focus · Drink · More. Branches fan out (Focus 15/25/50 min, Drink 150/250/500 ml, Prayer: mute next adzan / Qibla / reminders, More: downloads, disk, meeting notes, settings). Pick by direction or flick. See `DESIGN.md` → Components. | click the orb, or `Ctrl+Alt+Space` |
| Notify | orb grows into a pill: `[● icon  text ·  action  ✕]` | a notice arrives |

- **Orb body (v6.2):** a 48 px liquid-metal drop. While a task runs, a thin progress ring sits around it. No other orb library is drawn on the drop. The full state list (dragging, waiting, focus, recording, done, error, the prayer flow and the sun-following highlight) is in `DESIGN.md` under "Orb body: the mercury drop".
- **Shortcut (decided 2026-11-06):** `Ctrl+Alt+Space` opens the dial at the orb in keyboard mode, and shows the orb first if it was hidden. Pressing it again while the dial is open hides the orb. Esc closes only the dial. It is configurable and registered directly with Win32 `RegisterHotKey`.
- **While Hidden:** urgent notices (prayer, meeting starting) show the orb plus pill, then hide it again when dismissed. Normal notices wait in a queue and show as a badge count when you bring the orb back.

## 3. Drag and dock (Granola dock mode, adapted)
- Click versus drag: movement under **3 px** counts as a click.
- On drop, snap to the **left or right edge**, whichever side of the monitor's centre the pointer is on. Vertical position stays free, with an **8 px** margin.
- Tuck: when idle, the orb sits partly off the edge. Within **20 px** of the pointer it slides out with a spring (stiff on the way out, soft on the way back).
- Save the position **250 ms** after the drag ends, keyed by monitor ID **and** bounds. If that monitor is gone, fall back to the right edge at 70% of screen height (Granola's default).
- Handle DPI changes (`WM_DPICHANGED`) and monitor plug/unplug (`WM_DISPLAYCHANGE`).

## 4. Hit-testing (transparent window)
- The page reports the orb's circle rectangle to Rust.
- A **33 ms** loop compares the cursor position with that rectangle and calls `set_ignore_cursor_events(true/false)`.
- Pixels outside the circle pass clicks through to the app underneath.
- This runs on Windows too. Granola's classic hover is macOS-only, which is a gap we close.

## 5. Notification pill
- Grows out of the orb toward the centre of the screen. Width ≤ 360 px, 48 px tall (v6.1). Morph spring about 0.3 s.
- Each notice has `{id, source, priority: urgent|normal|quiet, title, body?, action?, ttl}`.
- **Auto-dismiss after 6 s** of visible time without hover. Hovering pauses the timer, the same as Granola's prompt (theirs uses 20 s).
- **Queue:** one pill at a time, ordered by priority then age. While collapsed, the orb's badge shows how many are waiting.
- **Per-source cooldown** (meeting prompt: 2 min, as in Granola) and **per-source mute** ("don't show downloads again").
- **Stays silent** when `SHQueryUserNotificationState` reports full screen, presentation mode or Focus mode, or when the active window is full screen. Notices queue instead (Poni does the same).
- An attention shake (±3 px, at most once every 5.5 s) is used only for urgent notices.

## 6. Notice sources (each one a plug-in module)
| Module | Trigger | Pill |
|---|---|---|
| prayer | custom offsets per prayer, default −30 min, −10 min and 0 s (see §6.1) | "30 menit lagi Dzuhur · 11:26" / "Memasuki waktu Dzuhur" |
| tasks | a to-do's due time is reached | "Due now: Kirim invoice · Done · Snooze 10m" |
| calendar | 10 min before an event, and at its start | "Standup in 10 min · Join" |
| focus | pomodoro phase ends | "Focus done · Break 5 min" |
| drink | interval while you're active (see §6.5) | "Minum dulu · 750 / 2000 ml · +250 ml" |
| download | clipboard listener (`AddClipboardFormatListener`) sees a media URL | "Download this?" → yt-dlp |
| disk | C: free space drops below 10% (checked every 10 min) | "C: is 92% full · See what's big" → Arbor quick scan |
| meeting | mic session active for a meeting app (`IAudioSessionManager2`); appears after 2 s, with a 2 min cooldown | "Meeting detected · Take notes" → minutes |
| windows-mirror (later) | `UserNotificationListener`; **needs an MSIX package identity** | WhatsApp/Telegram messages |

### 6.1 Prayer
- Times are calculated offline with Kemenag settings (Subuh 20°, Isya 18°, Ashar shadow factor 1). Per-prayer offsets [Subuh 2.3, Dzuhur 3.0, Ashar 2.0, Maghrib 2.5, Isya 1.9 min] cover the ihtiyat and the gap to the official sun model, then each time is rounded up to the next minute. Built in `crates/core` (`app-core`, 2026-11-06). Checked against the Bimas Islam schedule for Kota Yogyakarta: 20 of 20 exact on the four tuned November days, and 19 of 25 exact (the rest ±1 min) on held-out June and March days. When online, the app should fetch the official kab/kota schedule and use the calculation only offline.
- The location comes from Windows location or a city you pick.
- Also shows Imsak (Subuh −10 min), Terbit (sunrise) and the Qibla bearing (great-circle bearing to the Kaaba, 21.4225°N 39.8262°E).
- **Reminders are a list of offsets per prayer, each with its own on/off switch:**

  | Offset | Default text | Priority |
  |---|---|---|
  | −30 min | `30 menit lagi {Prayer} · {time}` | normal |
  | −10 min | `10 menit lagi {Prayer} · {time}` | normal |
  | 0 s | `Memasuki waktu {Prayer}` (+ optional adhan sound; Subuh can use its own sound) | urgent (shows even while the orb is hidden) |
  | custom | the user adds any offset and text (e.g. −5 min "Siap-siap wudhu"). Reminders inform; they never ask whether you prayed (DESIGN.md, "Prayer is not a task") | user picks |

- Imsak and Terbit reminders are optional (off by default; Imsak on automatically in Ramadan).
- **Prayer panel**, opened from the menu or by clicking a prayer pill:
  - Header row: `Imsak 03:41 · Terbit 05:07 · Qibla 295° WNW`.
  - A dotted arc (the sun's path) with one dot per prayer: Subuh, Dzuhur, Ashar, Maghrib, Isya.
  - The current or next prayer is a ringed dot with its label and time in bold. Isya is shown with a moon icon.
  - A small marker for "now" moves along the arc.
  - Below it: a countdown to the next prayer.
- **Focus integration:** a pomodoro session that runs into "Memasuki waktu" pauses itself and resumes after you dismiss the pill.

### 6.2 Tasks (to-do)
- **Folders and lists:** nested folders of any depth, each holding lists or subfolders. Example:
  ```
  📁 Family
     ├─ 📁 Rumah  → Belanja, Tagihan
     └─ 📋 Liburan
  📁 Work
     ├─ 📁 Client A → Sprint, Invoice
     └─ 📋 Admin
  📥 Inbox (default when a quick add has no list)
  ```
  - Folders and lists can be renamed, recoloured, given an emoji, dragged to reorder or move, and archived.
  - In quick add, `#` picks a list: `beli galon besok #belanja`. `/` picks a folder: `/work/client-a`.
  - Any folder can be opened as a "smart view" that shows the tasks from all lists inside it.
- **Sync across devices: local-first.**
  - Every device keeps a full SQLite copy, so the orb works offline and opens instantly.
  - Changes are written to a local outbox and pushed to a small sync server when online. The server sends changes to other devices in real time (WebSocket).
  - Each row has a stable UUID. Every field carries a hybrid-logical-clock timestamp, and the **last write wins per field**. That way, editing a title on the PC and ticking the task done on the phone don't overwrite each other.
  - Deletes are tombstones, purged after 30 days.
  - Server: **Cloudflare** (decided). See §9.
- **Sharing with family (decided):**
  - v1 shares **top-level folders** (Family, Work), and a shared folder is called a "space". Sharing individual subfolders comes later.
  - A member is an owner, an editor or a viewer of a space and everything inside it.
  - People are invited by link or email.
  - Each task shows who added it and who completed it. Tasks can be assigned to a member, and that person's orb gets the due pill.
- Each task has a title, optional notes, an optional due date and time, an optional repeat (daily/weekly), an optional assignee, and done/undone.
- **Quick add:** press the shortcut, then type, e.g. `beli galon besok 9am`. Natural-language dates are parsed in both Indonesian and English (`besok`, `lusa`, `jam 3`, `tomorrow`).
- The panel shows Today, Upcoming and Done. Tasks can be checked off right from the pill.
- At the due time a pill appears with Done and Snooze 10m. Overdue tasks add to the orb's badge count.

### 6.3 Calendar
- v1 (decided 2026-11-06): **Google sign-in** (OAuth, read-only) for Google Calendar, plus **ICS subscription URLs** for Outlook's published calendar and anything else. Polled every 10 min.
- Later: Microsoft sign-in, and writing events back.
- Calendars are told apart by fill, not colour: your own are filled, calendars shared with you are outlined (accepted 2026-11-06; revisit if it feels too plain in use).
- Reminder at −10 min and at the start. A **Join** button appears when the event has a Meet, Zoom or Teams link.
- An event in progress feeds the meeting helper (§6, meeting row), so the prompt can say "Standup · Take notes?"
- The Today panel shows one combined timeline: calendar events, tasks due, and the prayer times.

### 6.4 Focus (pomodoro)
- Defaults: 25 min focus, 5 min break, 15 min long break after every 4 focus sessions. All are configurable.
- Each next session can start by itself or wait for you.
- While running, the orb shows a progress ring and the minutes left. It keeps showing even in tuck mode.
- Focus can be started on a task ("Focus on: Kirim invoice"). Finished sessions are logged against that task.
- During focus, **normal** notices are held until the break. **Urgent** ones (prayer at 0 s, a meeting starting) still come through.

### 6.5 Drink (hydration)
- **Daily goal:** 2000 ml by default. You can change it, or have it suggested from body weight (30–35 ml/kg).
- **Glass sizes:** quick buttons for 150 / 250 / 500 ml, plus your own bottle size (e.g. "Tumbler 600 ml").
- **Reminders:** every 60 min during active hours (default 08:00–21:00). Interval and hours are configurable.
  - **Smart skip:** no reminder while you're away. Idle time over 5 min (`GetLastInputInfo`) pauses the clock, and the screen lock (`WTSRegisterSessionNotification`) resets it.
  - **Catch-up:** if you're behind pace (e.g. 600 ml by 14:00 against a 1150 ml target), the interval shortens to 45 min. If you're ahead, reminders become quieter.
  - **Respects focus and meetings:** a reminder that falls during a focus session waits for the break, and the break pill shows "Break 5 min · minum dulu?". During a recorded meeting it waits until the meeting ends.
  - **After prayer:** optionally ask 10 min after each prayer's 0 s pill ("Habis sholat, minum dulu"). Pairing a habit with something you already do makes it stick better than a fixed timer.
- **Puasa mode:**
  - Turns on automatically in Ramadan, or manually for Senin–Kamis and other sunnah fasts.
  - No reminders between Imsak and Maghrib.
  - At Maghrib it suggests buka water. During the night it spaces reminders between Maghrib and Imsak.
  - At sahur there's a −30 min pill: "Sahur: minum 500 ml sebelum Imsak 03:41".
  - The goal is split across the night window.
- **The pill:**
  - Text: `Minum dulu · 750 / 2000 ml`.
  - Actions: **+250 ml** (your default glass), **Snooze 15m**, and a dismiss button.
  - Priority: normal. It is never urgent and never shows while the orb is hidden.
- **Orb:** shows nothing extra by default, so it doesn't add clutter. The quick menu row shows `Drink 750/2000`.
- **Panel:**
  - A water level fill (moonlight white, not blue, to keep the single accent).
  - Today's log with times, with tap-to-undo.
  - A 7-day bar strip.
  - Settings.
- **Sync:** the log syncs like tasks, so a glass logged on the phone counts on the PC. It stays personal (your own space); it is never shared with Family.

## 7. Visuals
- **Source of truth: [`../DESIGN.md`](../DESIGN.md) (v2).** It covers tokens for both themes, type, icons, radius, the accent rule and the dotted time track. Designs are on the Doop canvas, page "v2 · antislop pass".
- Theme follows the Windows app setting (light or dark), and both themes ship.
- Fajr gold means prayer time only. Recording and errors use ember red. Everything else is neutral.
- Recording shows the mic level, updated every 100 ms and smoothed with a fast-decay peak follower.
- An original design: no Clippy, Granola or Apple artwork, and no "Clippy" name (it belongs to Microsoft). The app name is still to be decided.
- Respect `prefers-reduced-motion`: crossfade instead of springs.
- Every data view ships with empty, loading and error states that name the cause and the next action (Doop frame "v2 · 05").

## 9. Architecture (decided 2026-11-06: native Rust desktop, hybrid UI; mobile later; Cloudflare server)
```
crates/core        pure Rust, no OS code: prayer times, ID/EN date parsing, task/folder/space model,
                   notice queue + priority rules, drink pacing, focus timer. Exposes a UniFFI-friendly API
                   so a mobile app can reuse it later.
crates/sync        Rust sync client: rusqlite store, outbox, HLC clock, per-field last-write-wins merge,
                   WebSocket (tokio-tungstenite) to the space's Durable Object
crates/platform    OS services behind traits; platform/windows (windows-rs) now, platform/macos later: hotkey, tray, clipboard listener, foreground/fullscreen
                   watch, idle and lock, mic sessions (IAudioSessionManager2), location, toasts fallback
crates/orb         the orb + pill renderer: DirectComposition visual tree, Direct2D shapes, DirectWrite text,
                   DWM Acrylic backdrop, spring animations, hit-testing, edge docking
crates/panels      panels (menu, prayer, tasks, today, focus, drink, settings) built with Slint
apps/desktop       the exe: wires core + sync + win + orb + panels; one tokio runtime, one UI thread
apps/server        Cloudflare Worker (TypeScript): auth, invites, sync relay; one Durable Object per space;
                   D1 for accounts. It only stores and relays the change log; merging happens in crates/sync.
(later) apps/mobile  UI to be decided, calling crates/core + crates/sync through UniFFI
```
- **Why hybrid:**
  - The orb and pill float over other apps all day, so they need the lowest RAM, real Acrylic, per-pixel click-through and 60–120 fps morphing. Drawing them by hand on DirectComposition gives all of that, the way Oyenza does.
  - Panels are lists, forms and settings. Slint gives layout, text input, scrolling and accessibility (UI Automation), which would otherwise take months to write by hand.
  - Both sides read the same tokens from `DESIGN.md`. Inter (v6.1) is bundled as TTF and loaded into both DirectWrite and Slint.
- **Panel windows:** Slint with the winit backend and the Skia renderer, frameless and topmost. The raw HWND is taken through `raw-window-handle`, so panels get the same DWM corner rounding and no-activate behaviour as the orb. Panels are solid (no Acrylic), as `DESIGN.md` says.
- **Icons:** Fluent System Icons shipped as SVG. They're drawn by `resvg` into Direct2D bitmaps for the orb, and by Slint's own SVG support for panels.
- **Prior art already on disk:**
  - `minutes` (`~/granolarea/minutes`) is a Rust core with UniFFI. Its `capture`, `asr` and `store` crates become the meeting feature (step 10).
  - Arbor (`~/treesize-rea`) gives the disk scanner (step 9).
- **Local DB:** `rusqlite` with bundled SQLite in `crates/sync`. The same schema will serve a future mobile app.
- **Sync:**
  - Each device holds one WebSocket per space to that space's Durable Object.
  - The DO stores the space's change log in its own SQLite storage and sends changes to everyone connected.
  - A device that comes back online sends its outbox, then asks for everything since its last seen sequence number.
- **Auth:**
  - Better Auth on the Worker, using D1 for storage.
  - Sign in with Google, or with an email magic link sent through Cloudflare Email Service.
  - Family members join through an invite link: `/invite/<token>`, valid for 7 days, with a role.
- **Mobile (later):**
  - Family members on phones will need the app before shared spaces are useful to them.
  - Until then, sharing works between Windows PCs.
  - When mobile starts:
    - Prayer, focus and drink reminders become local notifications.
    - Assigned and changed tasks arrive as push (FCM/APNs) from the space's Durable Object.
    - A reminder dismissed on one device is marked seen everywhere.
- **macOS later (decided 2026-11-06): it must run smoothly on a MacBook Air M1 with 8 GB.** Windows ships first. These choices are made now, so the Mac port only adds code and rewrites nothing:
  - **Platform crate behind traits.** `crates/win` becomes `crates/platform` with traits: `Hotkey`, `Tray`, `Clipboard`, `Foreground` (full screen, presenting), `Idle` (idle and lock), `MicSessions`, `AudioCapture` and `Location`. `platform/windows` ships now; `platform/macos` is added later. `crates/core`, `crates/sync`, `crates/panels` and `apps/server` stay OS-free.
  - **One metal shader.** The liquid-metal shader is written once, in WGSL. `naga` turns it into HLSL for Windows and MSL for Metal at build time, so both platforms render the same drop.
  - **The orb renderer gets a small backend seam.** Windows uses DirectComposition with Direct3D. The Mac version will use a non-activating `NSPanel` (floating, on all Spaces) with a `CAMetalLayer`, and Core Animation springs.
  - **Mac equivalents, for planning:**
    - Hotkey: Carbon `RegisterEventHotKey`. Tray: `NSStatusItem`. Clipboard: `NSPasteboard` change count.
    - Idle: `CGEventSourceSecondsSinceLastEventType`. Full screen: `NSWorkspace` and `CGWindowList`.
    - Mic in use: Core Audio "device is running somewhere", plus the front app.
    - Meeting audio: Core Audio process taps on macOS 14.2 or later, or ScreenCaptureKit as a fallback.
    - Location: Core Location.
  - **Budget on M1 8 GB:** the app itself stays under 100 MB.
    - Meeting notes use Groq only (no local models), so they cost almost no RAM on the Mac.
  - Slint panels already run on macOS (winit plus Skia on Metal).
- **Repo:** a Cargo workspace on the Windows side (`C:\Users\dopamine\dev\clippy`), with `apps/server` as a small pnpm package deployed with `wrangler`. Toolchain: stable Rust with the MSVC target, plus the Windows SDK.

## 8. Build order
1. Orb shell (`crates/orb` + `crates/platform`):
   - DirectComposition window, Acrylic, the orb, and the pill morph
   - shortcut, tray, drag and dock, hit-testing
   - notice queue, with a "test notice" item in the tray menu
2. Prayer: calculation in `crates/core` with golden tests against the Kemenag schedule, reminders, and the Slint arc panel.
3. Focus and drink reminders. They're built together because both are timers.
4. Tasks, local only: folders, subfolders, lists, quick add (Slint panel, `rusqlite`).
5. Sync: `crates/sync` and the Worker with Durable Objects and auth, then family spaces and invites between PCs.
6. Calendar (Google sign-in, plus ICS links)
7. Download (yt-dlp sidecar)
8. Disk (Arbor scanner)
9. Meeting notes: **postponed** (user, 2026-11-06). The design is kept (frames 15 and 16, `docs/design/meetings.md`), but nothing is built until the user tests Groq on real speech. If that test fails, the feature is dropped.
10. Mobile app, UI to be decided, reusing `crates/core` and `crates/sync` through UniFFI.

Build and run on the **Windows side** (for example `C:\Users\dopamine\dev\clippy`). This spec lives in WSL. Cargo over `\\wsl.localhost` is slow, and building inside WSL produces a Linux binary.
