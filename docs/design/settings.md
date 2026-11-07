## First run and Settings (v6.2)
Doop frames "v6.2 · 21 First run (white)" (id kJ2Pp0_oBA) and "v6.2 · 22 Settings window (dark)" (id XNbeqRH48i), page "v6.2 · mercury + thinking orbs". Emails, device names and dates in them are sample content. Prayer times are the user's own (Yogyakarta, Kemenag).

### First run
- **When:** the first launch only, before the orb is shown. It can be reopened from Settings, About ("Run first-run again").
- **Window:** one small solid panel, centred on the primary monitor. In the mockup each step is 284 × 396 at 1×. Panel `#FCFCFC` / `#171717`, radius 20, the `--float` shadow, padding 20. It is a normal focusable window (unlike the orb), so keyboard users can tab through it.
- **Five steps,** one per screen, the same window size for each so nothing jumps.
- **Step marker** at the top: five 6 px dots, gap 4. Passed dots are `passed`, future dots are `track`, and the current one is a 16 × 6 ink capsule. "2 of 5" in 12/16 muted on the right. It is a marker, not a checklist.
- **Step title** 20/26, 500, −0.01em. One muted body line or two below it (13/18).
- **Footer:** Back (quiet 32 px button) on the left, the step's one primary button on the right (32 px, `#171717` with `#FAFAFA` text). Enter presses the primary, Esc goes back.

1. **Welcome**
   - A 148 px sunken well (raised fill, hairline, radius 12) holding the drop at 112 px (sprite hero at `0 0`, scaled).
   - "Welcome to [App]". Body: "A small metal drop at the edge of your screen. It grows a pill when something needs you, and opens a panel when you click it."
   - Footer: muted "About a minute" and Get started.
2. **Place and prayer method** ("Where you are")
   - Body: "Worked out on this PC, offline."
   - Two radio rows (40 px, radius 8, a 16 px ring, selected is a filled ink ring with the active fill): **Windows location** ("Yogyakarta, DIY · follows you"), selected by default; **Pick a city** ("Any kabupaten or kota") opens a search field.
   - Method row: "Kemenag RI" with "Subuh 20°, Isya 18°, +2 min" and a small Change (28 px). Change offers the kab/kota schedule too.
   - A preview of today: "Today, Jumat 6 Nov", then five columns: Subuh 03:51, Dzuhur 11:26, Ashar 14:41, Maghrib 17:38, Isya 18:50. Names 12 muted, times 13/500. These are times only: no checkboxes, no states.
   - Continue waits until a place is known.
3. **Shortcut** ("One shortcut")
   - Body: "Shows or hides the orb from any app."
   - A 124 px well with keycaps `Ctrl` + `Alt` + `Space`: 36 px tall, radius 8, panel fill with the `--ctl` lip; Space is 88 px wide. A small Change below them opens the recorder (see Settings).
   - Try it: pressing the shortcut here hides and shows the orb once. The line under the well starts as "Press it now to try." and becomes a check chip with **It works.** "You pressed it just now."
   - Row: "Start with Windows" ("Opens quietly when you sign in"), switch on by default.
4. **Family sync, optional**
   - Title with an "Optional" tag (20 px, radius 10, 12 muted on the active fill).
   - Body: "Share task spaces with family and keep your PCs in step."
   - **Continue with Google:** full width, white button with a hairline and lip, the real Google "G" at 16 px (Doop asset `https://doop.design/a/Cyiq-FDn4S.png` in the mockup; ship Google's official SVG mark).
   - An "or" rule, an email field (32 px, radius 16), and **Email me a sign-in link** (primary, full width). This is the Better Auth magic link sent through Cloudflare Email Service.
   - Fine print: "Skip it and everything still works on this PC, offline."
   - Footer: Back and **Skip for now** (quiet). After the link is sent the step shows "Check your email" with the address, Resend, and "Use a different email". Opening the link signs this PC in and moves to step 5 by itself.
5. **Done** ("You're set")
   - Three summary rows with 24 px icon chips, two-tone: **Yogyakarta** Kemenag RI, **Ctrl+Alt+Space** opens the menu, press again to hide, **Offline** sign in from Settings (or the email when signed in).
   - Finish (primary) closes the window.

**After Finish (on the orb):**
- The orb slides out at the right edge, at 70% of screen height (the spec's default spot), on `pop`.
- One pill: icon chip with the drop glyph, **[App] is ready** `Ctrl+Alt+Space`, close only, no action. Priority normal, the `open` tone, gone after 6 s.
- 3 s after the pointer leaves, the orb tucks half into the edge at 40% opacity, as usual.

**States in the steps** (shown as a list in frame 21):
| State | Step | Text | Action |
|---|---|---|---|
| Loading | 2 | **Finding your location** "Usually a few seconds; Continue waits." A 14 px short travelling arc in the chip. | Pick a city |
| Error | 2 | **Location is off in Windows** "Pick a city, or turn it on in Windows." | Pick a city |
| Error | 3 | **Ctrl+Alt+Space is taken** "Another app uses it. Try Ctrl+Alt+K." | Use Ctrl+Alt+K |
| Error | 4 | **Can't reach the sync server** "Skip for now; sign in later in Settings." | Try again |
- Error chips use ember (`#B3402F` stroke on an 7% ember tint). The text stays ink and muted.
- Keyboard focus is a 2 px ink ring with a 2 px panel-coloured gap (`0 0 0 2px #FCFCFC, 0 0 0 4px #171717`). Frame 21 shows it on Continue in step 2.

### Settings window
- **Window:** 720 × 580, solid panel (`#171717` dark / `#FCFCFC` white), radius 20, `--float` shadow. Opened from the dial (More, Settings) or the tray. Focusable, Esc or the close chip closes it.
- **Sidebar:** 200 px, hairline right edge, padding 12 × 8. A 32 px search field ("Search settings") on top. Items are 32 px rows, radius 8, a 16 px muted icon and the label 13/18. The selected row has the active fill, weight 500, and an ink icon.
- **Section order,** in groups split by a hairline:
  1. General, Orb
  2. Prayer, Tasks and sharing, Calendar, Focus, Drink
  3. Downloads, Disk, Meeting notes
  4. Notices, Sounds
  5. Account, About
- **Content pane:** padding 20 × 24. Section title 20/26 with a 28 px close chip on the right. Group labels 12/16 muted. Each group is a raised card (radius 12, hairline) of rows at least 56 px tall, padding 10 × 14, split by hairlines. A row is a label (13/18) with a muted line (12/16) and one control on the right.
- **Controls:** switch 32 × 20; segmented control 32 px with 28 px options (selected is a lifted 10% fill with a hairline); select 28 px with a chevron; small buttons 28 px; keycaps 28 px, radius 7.
- Changes save as you make them. There is no Save or Apply button, and the pane says so in one muted line at the bottom.

**General**
- Startup: **Start with Windows** ("Opens quietly as the orb when you sign in"), on. **Show the orb at start** ("Off starts it hidden; Ctrl+Alt+Space brings it back"), on.
- Appearance: **Theme**, a segmented control: Windows (default), Light, Dark. The muted line says what Windows is set to right now.
- Keyboard: **Show or hide the orb**, keycaps `Ctrl` `Alt` `Space` and Change.
- Language: **App language** (English; Bahasa Indonesia later). **Prayer and drink notices** (Bahasa Indonesia by default), with the sample "Memasuki waktu Isya", "Minum dulu".
- **Change shortcut** sheet: a small panel over the window. "Esc cancels" in the header. A 56 px capture field with a focus halo shows the keys held so far, a caret and "press a key". Hint: "Hold Ctrl, Alt or Win, then one more key." Buttons: Reset to Ctrl+Alt+Space, Cancel, Save (disabled at 40% until a full combo is held). It registers through `RegisterHotKey` and fails over to the error state if Windows refuses it.
- After Save, a quiet pill: **Shortcut** `now Ctrl+Alt+J` with Undo. No sound, gone after 6 s.

**Account**
- One lifted card (the only card in the pane): a 36 px initial avatar, the email (13/500), "Signed in with Google since 2 Nov 2026", and Sign out.
- Under it, a muted sync line with a cloud icon: "Synced just now · 3 devices · Family space with Ibu, Ayah and Raka".
- **Signed-in devices** with a count. Each row: a 24 px device chip, the name, a muted line (Windows version, place for this device, last seen), and Sign out. This device has a "This device" tag and no Sign out on its row (the card's Sign out covers it).
- **Your data:** Export your data ("Tasks, spaces, focus and drink logs in one .zip"), Export. Delete account ("Removes your account and its data from the server"), Delete.
- Footer line with a lock icon: "No password. You sign in with Google or an email link."

**Delete account confirm** (a sheet over the window)
- Title "Delete your account?", "can't be undone" in muted on the right.
- Body: "Your account, private spaces and synced copies are removed from the server. Other people keep their own tasks."
- One row per space you own, with a select to hand it over: **Family** "You own it · 3 other members", Give to Ibu. If you don't pick anyone, the delete is blocked until you do or pick "Delete this space for everyone".
- **Also clear this PC** ("Off keeps a local copy, offline"), off by default.
- A field: "Type delete to confirm". Delete account (primary) is disabled until the word matches.
- Buttons: Export first, Cancel, Delete account. Afterwards the app goes to the Not signed in state, and the orb keeps working offline.

**States**
| State | Where | Text | Action |
|---|---|---|---|
| Error | General | **Ctrl+Alt+K is taken** "Another app already uses it." | Pick another |
| Error | General | **Couldn't add to startup** "Windows blocked it. Allow [App] there." | Startup apps |
| Info | General | **Contrast theme is on** "[App] uses your Windows contrast colours." | none |
| Loading | Account | **Preparing your export** "64% · about 20 s left", a 14 px progress arc | Cancel |
| Done | Account | **Export saved** "Downloads, [App]-export-2026-11-06.zip" | Show |
| Error | Account | **Can't reach the sync server** "Device list may be old. Retrying in 30 s." | Retry now |
| Empty | Account | **Not signed in** "Everything works on this PC, offline." | Sign in (opens step 4 of first run as a sheet) |
- The device list loading state is three 56 px rows with muted bars, no spinner.

### Rules
- Signing in is never required. Every feature except family spaces works offline, and the copy says so at both places it is offered.
- No password anywhere: Google or an email magic link, through Better Auth on the Cloudflare Worker.
- Prayer settings in first run are a place and a method only. No question about whether or how you pray, no counts, no history.
- Ember is only for the error chips. Destructive actions stay neutral; the typed confirm and the plain consequences do the warning.
- Each pane and step has one primary button at most.
- Visible keyboard focus on every control (2 px ring with a 2 px gap). Tab order follows reading order; the sidebar is a list the arrow keys move through.
- Copy is English except prayer and drink notice samples. No em dashes. [App] stands in for the name.
