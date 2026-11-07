# DESIGN.md: orb direction (v6.1)

> **v6 (2026-11-06, the user's call):** "don't use gold, I prefer metal" and "use something more neutral, maybe white or dark".
> - The warm cream and stone palette is replaced by a **neutral zinc** palette, sampled from the user's reference screenshot (`#F9F9F9` sidebar, `#FCFCFC` page, `#E4E4E7` divider).
> - **All gold is removed.** The prayer moment is **liquid chrome**.
> - There is **no colour accent** at all. Emphasis comes from ink weight, filled versus ring markers, and the one metal moment.
> - The warm sun-core tints are dropped. The orb core is plain neutral.
>
> Sections below are updated to v6. Where an older section still names a warm or gold value, v6 wins. Doop page: "v6 · neutral + liquid metal".

v3.1 added eight patterns from the deep passes (`docs/research/orenji-deep.md`, `docs/research/shujaat-deep.md`):
1. surface morph
2. grow vs shrink timing
3. notice stack
4. press-in
5. row ripple
6. three surface levels
7. two-tone labels
8. concentric radii as a formula

Dial: ENERGY 1 / RHYTHM 2 / MOTION 2

Design Read: an ambient Windows utility (an always-on-top orb, notification pills and small panels) for one Indonesian Muslim power user and their family. Quiet, neutral utility language (white or dark, like native Windows 11 apps), with a little play in the motion and sound, and one moment of liquid metal.
- ENERGY 1 because it sits on top of other people's work all day.
- RHYTHM 2 because each panel has its own job, so panels differ in shape while sharing the same parts.
- MOTION 2 because the springs and sounds are functional: they say where a notice came from and that an action landed.

v3 mixes two studied references (`docs/research/style-orenji-shujaat.md`). We take principles, not their look:
- **From orenji.studio:** softer radii and a playful overshoot on small things. Its warm cream ground was used in v3 to v5 and dropped in v6.
- **From shujaat.info:** layered micro-shadows, a near-critical spring for panels, tight display tracking, and a synthesized UI sound palette.

## Identity
- **Mood (v6):** a clean instrument: white or graphite, hairline edges, and one piece of polished metal that comes alive at prayer time.
- **Identity motif 1, the dotted time track.** One dotted line style (1.5 px dots, 6 px gaps) stands for time still to come:
  - the prayer arc
  - the unfilled part of the focus ring
  - the drink vessel's empty part
  - the Today rail

  Solid means passed or done; dotted means still ahead.
- **Identity motif 2, the mercury drop (v6.2).** The whole orb is one liquid-metal drop, replacing the v6 plain dot. It changes in only three ways:
  - a running task draws a thin progress ring around it
  - the prayer moment makes the whole drop flow once
  - its highlight follows the sun through the day (the v3 sun clock, rebuilt without colour)

  See "Orb body: mercury drop × thinking orbs" below.
- **No colour accent (v6).** Prayer emphasis uses ink, never hue:
  - the next prayer is a filled ink dot inside a ring, with its name and time in bold
  - other prayers are hollow ink rings
  - urgent prayer pills carry a filled ink dot
  - the only "special" material is the liquid chrome at *Memasuki waktu*

  Ember red stays for errors and recording, because those are states that need a warning colour.

## Theme
Follows the Windows app theme. Both themes ship and both are checked.

**v6.1 (current):** **true neutral** greys, replacing zinc's blue cast (audit finding 5, `docs/research/audit-why-v6-feels-cheap.md`). Dark surfaces are layers of **white at low alpha** over one base, the way Libraries.dev builds them.

| Role (v6.1, true neutral) | Dark | White | Text contrast |
|---|---|---|---|
| desk / window behind | `#0F0F0F` | `#F5F5F5` | |
| panel (base) | `#171717` | `#FCFCFC` (sidebar `#F9F9F9`) | |
| raised (rows, cards, inputs) | `rgba(255,255,255,.05)` | `rgba(0,0,0,.035)` | |
| active (buttons, chips, selected) | `rgba(255,255,255,.08)` | `rgba(0,0,0,.05)` | |
| hairline | `rgba(255,255,255,.07)` | `rgba(0,0,0,.07)` | non-text |
| text | `#FAFAFA` | `#171717` | 17.4:1 / 17.4:1 |
| muted text | `#9E9E9E` | `#6E6E6E` | 6.7:1 / 5.0:1 (4.7:1 on `#F5F5F5`) |
| dotted track and future dots (non-text) | `#3A3A3A` | `#D4D4D4` | never used for text |
| passed (arc, rails) | `#6B6B6B` | `#A3A3A3` | non-text |
| ember (recording, error) | `#E2614F` | `#B3402F` | |
| orb/pill fill over Acrylic | `#1C1C1C` at 92% | `#FCFCFC` at 92% | |
| primary button | `#FAFAFA` on `#171717` text | `#171717` with `#FAFAFA` text | |

v1's `#55585E` stays banned for text (2.5:1).

## Surfaces
- **Glass** (Acrylic): orb and pill only.
  - In the app this is DWM Acrylic (`DWMSBT_TRANSIENTWINDOW`) under a DirectComposition tint. The `backdrop-filter` in the Doop mockups only stands in for it.
  - Panels are solid.
- **Elevation and controls:** see "Surface depth" (v6 values) below. The v3 warm micro-shadow and lift are retired.
- **Glow:** none. The prayer moment uses liquid chrome instead.
- **Radius (v6.1),** concentric where things nest. Every tappable control is **fully round**, and content is rectangular (audit finding 6):
  - orb: 48 px circle, the same height as the pill
  - pill: 48 px tall, radius 24, with a **36 px round icon chip** (6 px inset) and at most **one** action
  - buttons and chips: fully round (32 px buttons, 28 px chips)
  - list rows: 36 px, radius 8
  - cards: radius 12
  - panels: radius 20
- **Icons sit in chips (v6.1):** 36 px circle in pills, 24 px rounded square (radius 7) in lists, each with its own `active` fill and hairline. No loose icons.
- **Dark shadows (v6.1):** nearly flat. Use a hairline plus `inset 0 1px 0 rgba(255,255,255,.05)` and at most `0 16px 40px rgba(0,0,0,.35)` for real floating surfaces. Light keeps the soft `0 4px 42px rgba(0,0,0,.07), 0 2px 6px rgba(0,0,0,.05)`.
- **Three surface levels (v3.1):** a sunken **well** holds lists, the **panel** is the base, and one white **card** lifts the single item that matters now: the next prayer, the running focus task, or the task due soonest.
  - Only one card per panel. If everything is a card, nothing is.

## Prayer is not a task (v6.1, the user's rule)
Shalat is a kewajiban, so the app never treats a prayer as something to complete.
- **Never checkable:** prayers have no checkbox, no icon chip shaped like a checkbox, and no "done" or "missed" state.
- **No tracking:** no streaks, no history, no counts. The v4 opt-in prayer history is removed. The History panel covers drink and focus only.
- **In lists, prayers are markers on the day, not rows:**
  - a dashed rule with the name (500) and time (muted), centred, 30 px tall
  - the next prayer is a quiet banner at the top of Today ("Next prayer · Dzuhur in 30 min · 11:26")
- **Reminders, the prayer arc, Qibla and the liquid-metal moment stay.** They inform; they don't grade.

Doop frame: "v6.1 · 04 Today: prayers are not tasks".

### Prayer panel, reminders and Qibla (v6.2)
Doop frames "v6.2 · 05 Prayer: panel, reminders, place (white)" and "v6.2 · 06 Prayer: evening, orb pills, Qibla (dark)".
- **Arc:** a dotted track for what is still ahead and a solid line for the day so far.
  - Past prayers are filled `passed` dots; future ones are `track` dots.
  - The next prayer is a ringed dot with its name and time in 500.
  - **Isya is always a moon.** When Isya is next, the filled moon sits inside the ring.
- **Countdown card:** "Isya in 19:04", with the active reminder chips.
- **Buttons:** Reminders, Place, Qibla.
- **Reminders editor:**
  - A segmented control picks the prayer (Subuh to Isya).
  - Each offset is a row with its message preview and a switch: −30 min, −10 min, at the time (with adzan select), and custom (e.g. −5 min "Siap-siap wudhu").
  - Add a reminder.
  - "Use these for every prayer" (Subuh can keep its own adzan).
  - Imsak and Terbit are off by default, and Imsak turns on by itself in Ramadan.
  - Custom texts inform. They never ask whether you prayed.
- **Place and method:**
  - Windows location (follows travel).
  - Kemenag RI (Subuh 20°, Isya 18°, +2 min ihtiyat), or the kab/kota schedule.
  - An optional "Show Dzuhur as Jumat" on Friday.
  - Adzan has its own volume and mute. It is silent in meetings and full screen, and the pill still shows.
- **Pills:**
  - −30 and −10 min: normal, a bell chip, no action, gone after 6 s.
  - At the time: urgent, a filled ink dot chip, "Memasuki waktu Isya" and one action, Mute adzan. It shows even while the orb is hidden, and the drop flows once.
  - A focus session that runs into prayer time pauses itself ("Focus paused for Isya · 12:40 left", Resume).
- **Qibla:** a static compass. North is ink and the other points are muted. The Qibla line ends in a small square for the Kaaba.
  - The bearing is shown big: "295° WNW".
  - Instruction: "Face north, then turn left until you face the line".
  - Distance by great circle: Yogyakarta to the Kaaba is about 8,350 km.

## Orb body: the mercury drop (v6.2, chosen)
Chosen: option A, kept plain. B (metal bezel) and C (bead in glass) are dropped. Doop page "v6.2 · mercury + thinking orbs", frames 01 and 02.

**The user's rule: no other orb library is drawn on the drop.** A v6.2 draft broke the drop into Thinking Orbs beads during tasks. It was rejected as too much and has been removed.

- **At rest:** the whole 48 px orb is one polished liquid-metal drop (`shape` circle, `fit` contain, shift R/B .15, softness .12, tint `#ffffff40` white / `#ffffff66` dark). The metal is a still frame. Nothing loops.
- **While a task runs:** a thin progress ring around the drop, 6 px out (radius 30 at 1×).
  - Track: hairline 1.5 px, `rgba(255,255,255,.12)` dark / `rgba(0,0,0,.1)` white.
  - Arc: 2 px with round caps, `#FAFAFA` dark / `#171717` white, filling clockwise from 12 o'clock.
  - Tasks with no percentage (sync, finding location): a short arc, a quarter of the ring, that travels only while the work runs.
  - Focus is different on purpose: it uses the dotted ring (time), not the hairline track (progress).
  - Recording has no ring: an ember dot (`#E2614F` dark / `#B3402F` white) at the top right. The mic level shows in the pill.
- **When the task ends:** the ring closes and fades out as one ripple on Gooey's bouncy spring (320 / 17, about 380 ms), and the done tone plays.
- **Prayer time:** never a ring. The whole drop flows once, slowly (about 5 s), then goes still.
- **Light through the day** (the v3 sun core, rebuilt without colour): the shader `angle` follows the sun. Subuh about 28°, Dzuhur 90°, Ashar 128°, Maghrib 160°. After Isya the metal dims (brightness .7).
- **Hover:** the highlight leans toward the pointer (about 62° when the pointer is to the left).
- **Reduced motion:** the travelling arc holds still, and the task name shows in the tooltip.

### Orb states (v6.2, carried over from v1 to v5)
| State | Look | First designed |
|---|---|---|
| Hidden | nothing on screen, tray icon only; `Ctrl+Alt+Space` toggles | v1 |
| Idle, tucked | half off the edge at 40% after 3 s; still metal | v1 |
| Hover | slides out within 20 px; highlight leans toward the pointer | v1 |
| Dragging | stretches along its path (about 1.14 × 0.88), snaps to the nearest edge | v6.2 |
| Waiting notices | ink-white badge (`#FAFAFA`, text `#171717`) with the count | v1 |
| Notify | a pill buds out of the drop, then pinches off | v5 |
| Menu | the dial fans out from the drop | v4 |
| Task running | thin progress ring around the drop; a short travelling arc when there is no % | v6.1 |
| Focus running | ring around the drop: solid is time spent, dotted is time left; minutes on hover | v1 |
| Recording | ember dot at the top right, no ring; mic level in the pill | v1 |
| Done | the ring closes, one ripple, done tone | v5 |
| Error | ember badge; the pill names what failed and the next step | v2 |

## Tasks panel and sharing (v6.2)
Doop frames "v6.2 · 03 Tasks: folders, spaces, quick add (white)" and "v6.2 · 04 Sharing, orb pills, sync states (dark)". Names and items in them are sample content.

- **Layout:** one panel, 780 × 560. A 228 px sidebar (`#F9F9F9` white, hairline right edge) holds search (`Ctrl+K`), Inbox, Today and Upcoming, then **Spaces**. The list sits on the right.
- **Spaces:** a top-level folder is a space.
  - A shared space shows a people chip and its member avatars.
  - A private one shows a lock chip and the word "private".
  - Folders nest to any depth. Each level indents 20 px, with a chevron, a folder or list icon and a muted count.
- **List header:** breadcrumb (12 px, muted), list name 20/500, member avatar stack, and a Share pill.
- **Quick add:** a 40 px field with a focus halo.
  - Parsed parts turn into 24 px token chips as you type (a date with a calendar icon, a person). Dates are read in Indonesian and English.
  - One muted line restates what it understood, plus the grammar: `#` list, `/` folder, `@` person.
- **Rows:** 40 px, radius 8.
  - Left to right: an 18 px rounded checkbox (radius 6), the title, the due date (muted; overdue is ink 500 "Overdue · Tue", never red), and the assignee avatar (20 px initials). An empty dashed avatar means unassigned.
  - A second muted line shows "Added by Ibu on her phone" on tasks someone else made.
  - On hover and keyboard focus: Assign, Due and More icon buttons (28 px, round).
- **Tabs:** To do and Done (Liquid Tabs). A collapsed footer reads "3 done today · last: Ayah finished Gula 1 kg at 08:40".
- **Sync status** sits at the bottom of the sidebar: "Synced just now · 3 devices". There is no conflict screen, because merging is per field.
- **Share sheet:**
  - Invite by email or phone, with a role select (Editor by default).
  - An invite link that joins as Editor and expires in 7 days.
  - Members with role pills, and pending invites with Resend.
  - Rules line: editors add, edit and finish; viewers only see; only the owner invites or removes people.
- **On the orb (pills):**
  - **Assigned to you:** person chip, "Gas elpiji 3 kg" plus muted "Ibu · besok 09:00", action Open. It shows once, then waits in Today.
  - **Due now:** checkbox chip with one action, Done. Snooze 10 min is `S` or right-click.
  - **Family activity:** quiet (no sound, gone after 6 s), with no action. Mutable per space.
  - **Overdue:** no pop-up again. It joins the badge count silently.
- **States:**
  - offline (changes waiting)
  - can't reach the sync server (auto-retry in 30 s, plus Retry now)
  - a member left (their tasks become unassigned, plus Review tasks)
  - empty list (Add task, plus the `#list` tip)

## Focus (v6.2)
Doop frame "v6.2 · 07 Focus: running, setup, on the orb (white)".
- **Running panel:**
  - A "Focus on" task chip.
  - A 168 px ring: solid is time spent, dotted is time left. In the centre, the minutes left (34/500) with "left of 25:00" below.
  - Session dots: done ones are filled and the current one is half filled.
  - A held-notices line: "2 notices wait for your break. Prayer time and a meeting starting still come through."
  - Buttons: Pause (primary), Skip to break, Stop.
- **Setup panel:**
  - The Ovio knob for length (5 to 90 min, 5 min detents, 270° sweep). Ticks up to the value are ink 2 px; the rest are muted 1.5 px. The knob face shows the value with "min" below it.
  - Presets: 15, 25, 50, Custom.
  - Optional task (sessions are logged against it).
  - Breaks: 5 min, then 15 min after every 4 sessions.
  - "Start the next session by itself" is off by default.
  - A full-width "Start 25 min" button.
- **On the orb:**
  - The dotted focus ring; hovering shows "Focus · 18 min left · Kirim invoice". The ring stays visible while tucked.
  - "Focus done · 25 min" with Start break, the done tone and one ripple.
  - "Break 5 min · minum dulu?" with +250 ml, when a drink reminder waited for the break.
- **This week:** a 7-bar strip of focus time with today in ink. It is part of History, which covers focus and drink only.

## Drink (v6.2)
Doop frame "v6.2 · 08 Drink: panel, settings, on the orb (dark)".
- **Panel:**
  - A glass: the empty part is a dotted outline (still to come), and the water is moonlight white `#EDEDED`, never blue, with a white meniscus line.
  - "750 / 2000 ml" with pace text ("Ahead of pace for 11:00").
  - Quick buttons: +150, **+250 ml** (the default glass, primary), +500, and your own bottle.
  - Today's log: time, amount and source ("from your phone"). Undo is only on the latest entry.
  - Last 7 days as bars: met days are grey `#9E9E9E`, short days are track, today is ink. The goal is a dashed line.
  - "Next reminder 11:42 · every 60 min".
- **Settings:**
  - A goal knob (1000 to 4000 ml, 250 ml detents), or a suggestion from weight (30 to 35 ml per kg).
  - Glass-size chips; the filled one is the pill's quick button.
  - Remind every 60 min, 08:00 to 21:00.
  - Skip while away: 5 idle minutes pauses the clock, and locking the PC resets it.
  - Catch up when behind (45 min).
  - After prayer ("Habis sholat, minum dulu", 10 min after).
  - Puasa mode: on by itself in Ramadan; you can add Senin and Kamis.
- **Pills:**
  - "Minum dulu · 750 / 2000 ml" with +250 ml and close. Snooze 15 min is `S`.
  - "Habis sholat, minum dulu" with +250 ml.
  - "Sahur, minum dulu · Imsak 03:41" with +500 ml.
  - All drink pills are normal priority, never urgent, and never shown while the orb is hidden.
- **Privacy:** the log syncs to your own devices only. It is never shared with Family.

## More features (v6.2, one spec file each)
Eight features were designed in parallel, one designer each, on the same tokens and rules. Each spec is its own file. Frames 09 to 24 are on the Doop page "v6.2 · mercury + thinking orbs".

| Feature | Spec | Frames |
|---|---|---|
| Calendar + Today | [`docs/design/calendar.md`](docs/design/calendar.md) | 09 Calendar + Today (white), 10 week, event pills, accounts (dark) |
| Downloads | [`docs/design/downloads.md`](docs/design/downloads.md) | 11 offer, queue, settings (white), 12 on the orb, errors (dark) |
| Disk | [`docs/design/disk.md`](docs/design/disk.md) | 13 low space, scan, results (white), 14 big folders, clean up safely (dark) |
| Meeting notes | [`docs/design/meetings.md`](docs/design/meetings.md) | 15 detect, record, notes (white), 16 on the orb, summary, privacy (dark) |
| Quick menu dial | [`docs/design/dial.md`](docs/design/dial.md) | 17 dial (white), 18 branches, keyboard (dark) |
| Notices | [`docs/design/notices.md`](docs/design/notices.md) | 19 stack and queue (white), 20 quiet rules (dark) |
| First run + Settings | [`docs/design/settings.md`](docs/design/settings.md) | 21 first run (white), 22 settings window (dark) |
| History | [`docs/design/history.md`](docs/design/history.md) | 23 drink and focus (white), 24 year view, empty (dark) |

**Decisions from the user (2026-11-06):**
1. **Shortcut:** `Ctrl+Alt+Space` opens the dial (it shows the orb first if hidden). Pressed again while the dial is open, it hides the orb.
2. **Calendar:** Google sign-in in v1. Outlook comes through an ICS link.
3. **Meeting notes AI:** Groq only, no local models: Whisper large-v3-turbo for speech and `qwen3.8-27b` for notes. **The whole feature is postponed** until the user tests real speech. If that test fails, it is dropped. See `docs/research/meeting-notes-ai.md`.
4. **Calendars:** filled (yours) and outlined (shared with you) is fine for now.
5. **Presenting full screen:** "Memasuki waktu" still shows, silently. The current design stays.
6. **Disk:** never a permanent delete. Everything goes to the Recycle Bin.

Reference from the user: Granola's pill "Meeting detected · Discord · [logo] Take notes ⌄". It is a **split button**: one main action plus a chevron for its options.
- We allow this as the one exception to "at most one action". The chevron is part of the same action, not a second one.
- Use it for Take notes (pick a template or language) and for Save in Downloads (pick the format).
- Discord is on the meeting detection list.

Shared rules carried into all eight:
- prayer is never a task
- true-neutral tokens
- ember only for errors and recording
- pills with at most one action
- the plain mercury drop with rings, dots and badges only
- `[App]` as the name placeholder
- sample content labelled

## Labels (v3.1)
- **Two-tone labels:** the noun in text colour, the time or state in muted, on one line. For example **Ashar** `in 12 min`, **Kirim invoice** `due 14:00`, **Minum dulu** `750 / 2000 ml`.
- Pills, menu rows and the Today timeline all use this, so a glance reads the noun first.

## Notice stack (v3.1)
- **Collapsed:** the front pill shows the newest notice. Up to two older ones peek out behind it:
  - offset 6 px and 12 px
  - scale .96 and .92
  - opacity .7 and .45
- **Hover the stack:** it spreads into a column with 8 px gaps on `settle` (400 ms). At most **3 pills** are visible; more collapse into a "+2 more" row that opens the full list in the menu.
- **Pacing:**
  - a new pill waits at least **1.5 s** after the previous one appeared
  - notices from the same source merge ("3 downloads done")
  - urgent prayer notices jump to the front and never merge
- The stack is a column, never a fan: no rotation.

## Type
- **v6.1: Inter** (variable, optical sizes), with `font-feature-settings: "tnum", "cv11"` so times and amounts stay aligned. **No monospace anywhere.**
  - Reason: every reference the user picked uses a quiet grotesk at 13 px and 500 weight (Libraries.dev is all Inter).
  - Atkinson's low-vision shapes and its mono digits were the biggest reason v6 felt cheap (audit finding 1). Inter is OFL and bundled with the app, so it renders the same in Doop and in DirectWrite.
- **Scale:**
  - controls 13/16
  - body 13/18
  - secondary 12/16
  - panel titles 14/18
  - card numbers 26/30
  - focus timer 34 to 44
- **Weight:** 500 for titles, labels and buttons; 400 for body and muted values. Never 600 or 700 below 28 px (audit finding 2).
- **Tracking:** −0.005em at 14 px, −0.02em at 26 px, −0.03em at 34 px and up.
- **Labels:** sentence case, 12 px, muted. Two-tone labels use the **same size** for noun and value; only colour and weight differ.

## Motion
| Token | Curve | Duration | Used for |
|---|---|---|---|
| `grow` | `cubic-bezier(.3,1.25,.5,1)` (about 3% overshoot) | 360 ms | surface morphs: orb → pill, pill → panel |
| `shrink` | `cubic-bezier(0,0,.2,1)` (ease-out, no overshoot) | 200 ms | the reverse morphs. Closing is always faster than opening. |
| `pop` | `cubic-bezier(.34,1.56,.64,1)` (bigger overshoot) | 300 ms | tiny things only: the orb sliding out of the edge, the badge appearing |
| `settle` | `cubic-bezier(.2,.85,.25,1.05)` (slight overshoot) | 250 ms (hover), 400 ms (stack, panel layout) | stack spread, list reorder, row collapse |
| `press` | scale 1 → .98 | 120 ms down, 180 ms back on `settle` | buttons, rows and chips. Pressed things sink rather than lift (after Shujaat). |
| `ripple` | y 6 px → 0, opacity 0 → 1, with `settle` | 200 ms each, 30 ms stagger, at most 8 rows (240 ms total) | panel rows entering, starting from the row nearest the orb |
| `fade` | ease-out | 150 ms | content in and out, every exit |

**Surface morph (v3.1, after Shujaat):** orb, pill and panel are one object changing shape. Size and corner radius animate together (radius from 22 → 22 → 20), and the content is never stretched.

| Time | Open (`grow`, 360 ms) | Close (`shrink`, 200 ms) |
|---|---|---|
| 0 ms | The shape starts growing toward the screen centre. Content is hidden, with its text width already fixed at the final size. The `open` tone starts. | Content fades out (80 ms). |
| ~200 ms | The shape passes its final size by about 3%. | The shape shrinks back to the orb. |
| 250 ms | Content starts fading in (`fade`, 120 ms), so text never reflows. | |
| 360–370 ms | Settled and ready. The auto-dismiss clock starts. | Done at 280 ms, after the content fade-out. |

- No endless loops. The prayer pill shakes once (±3 px) when it appears, and repeats only if it is still unread after 5.5 s, at most twice.
- Under `prefers-reduced-motion` (Windows: "Animation effects" off), every movement becomes a 150 ms crossfade.
- In Rust these curves are evaluated by `crates/orb` on its own frame clock. Slint panels use the same curves through its `animate` blocks.

## Components ported from Ovio (v4)
Ovio (https://github.com/JanaSundar/ovio, MIT) is React + Tailwind + Motion, so it can't run in the native app. We port its logic and constants to Rust and draw them with these tokens. Licence: `THIRD-PARTY-NOTICES.md`. Designs: Doop page "v4 · Ovio components".

| Ovio | Ours | Where in Rust | Kept from Ovio |
|---|---|---|---|
| Dial Menu | **Quick menu.** A half-dial opens from the docked orb; the orb grows into the 80 px hub and names the aimed item. | `crates/orb/src/dial.rs` | hub 44, icons 74, ring 102, band 110/132/154, child step 32°, long press 450 ms, drag 12 px. Aim by direction, so a flick picks. Arrows go round, Enter picks or opens a branch, Esc steps back. Edge-clamping. |
| Physical Knob | Focus length (5 to 90 min, 5-min detents), drink goal, adzan volume | `crates/panels` knob widget (Slint) | 270° sweep (−135 to 135), resistance past the end stops, capped flick inertia (1600°/s), wheel and arrow keys, `role=slider` values. Each detent plays the `press` click. |
| Rolling Number | Focus timer, drink amount, prayer countdown | Slint digit-column component | critically damped spring: roll 0.9 s (ω 7.5), quick 0.3 s (ω 22) for per-second values. The width snaps so neighbouring digits never slide. Static under reduced motion. |
| Contribution Graph | History panel: drink (goal met) and focus (time) only. **Never prayer** (see "Prayer is not a task") | grid and year maths in `crates/core` | one cell per day, column-by-column entrance on `ripple`, today outlined. Five true-neutral levels per theme (`#EEEEEE` to `#171717` white, `#242424` to `#FAFAFA` dark; see `docs/design/history.md`). |
| Liquid Tabs | Tabs in History and Tasks | Slint | an indicator that slides between tabs on `settle` |
| Toast | Already the pill and notice stack | | Errors stay 8 s (normal notices 6 s). Pressing a notice's action also dismisses it. |

Not taken from Ovio:
- Craft's tape, tilts, handwriting and orange accent.
- Retro's and Toy's looks.
- Its `:focus-visible { outline: none }`. We always show a 2 px focus ring (anti-slop R-32).

## Thinking orbs and liquid (v5)
From Libraries.dev (https://github.com/Jakubantalik/Libraries.dev, MIT): **Thinking Orbs** and **Liquid Gooey**. Designs are on the Doop page "v5 · thinking orbs + liquid". Licence: `THIRD-PARTY-NOTICES.md`.

> **Retired from the orb in v6.2.** The user rejected Thinking Orbs on the mercury drop. Tasks on the orb are thin progress rings (see "Orb body"). The notes below are kept for history only.

**Thinking orbs = task states (v5, retired).** A dotted 3D orb, drawn only while a real background task runs:

| State | Task |
|---|---|
| working (orbits) | a download |
| searching (scan meridian) | a disk scan |
| listening (wave) | recording a meeting |
| composing (sash) | writing meeting notes |
| connecting (constellation) | syncing a space |
| breathing (ring) | finding your location |

- **Not on the orb (v6.2):** Thinking Orbs are never drawn on the mercury drop (the user's rule). Earlier, the 20 px preset replaced the plain core inside the 48 px orb (a pill uses a progress ring in its icon chip instead, audit finding 9). The 64 px preset is for panel loading states. The two presets are tuned separately; one is not a scaled copy of the other.
- **Ink:**
  - Neutral ink (v6): light `#18181B` ramped toward white, dark `#FAFAFA` ramped toward black, using the library's own tint rule.
  - Ink only. The liquid-metal prayer moment still overrides the orb.
- **Rules:**
  - Only while the task runs, then the `done` tone, so there is never an idle loop.
  - One orb at a time (the newest task); the menu lists all of them.
  - Reduced motion: the static frame at t = 0.6 s plus the task name.
  - Paused while the orb is hidden.
- **Rust:**
  - Port the engine (pure maths, about 1,150 lines) to `crates/orb/src/thinking/`.
  - Verify every dot against `spec/orbs-golden.json` (tolerance 0.0001; 72 cases: 9 states × 2 sizes × 4 times).
  - Draw with Direct2D circle fills, z-sorted, with no blur.

**Liquid = how the pill leaves the orb.**
- **Orb to pill:** replaces `grow`. The pill buds out of the orb like a droplet and pinches off: bulge, neck, bridge, separated. It uses Gooey's *bouncy* spring (stiffness 320, damping 17, mass 1), about 380 ms.
- **Back into the orb:** dismiss is the reverse, on *snappy* (480 / 34).
- **Notice stack:** spreading on hover splits the pills off the front pill on *smooth* (190 / 26).
- **Where not:** pill to panel keeps the plain surface morph (`grow`). Liquid on big shapes reads as wobble.
- **Only the silhouette is liquid.** One shape layer carries the merge and a single shared shadow. Text, icons and buttons are a separate crisp layer that never blurs (Gooey's key idea).
- **Native implementation:**
  - A smooth union (signed-distance smooth-min) of the orb circle and the pill rounded rectangles, computed in one small HLSL pixel shader on a DirectComposition surface.
  - The same distance field draws the shadow.
  - The fallback is the Direct2D chain Gaussian blur σ 6 → colour matrix (alpha × 18 − 7) → shadow, which is the same recipe as the mockups.
- **Gap:** shapes closer than about 12 px bridge, so the settled orb-to-pill gap is **12 px** (was 10).
- **Limits:** bounce 0.5 or less. Reduced motion means no liquid, just a 150 ms crossfade.

## Surface depth and liquid metal (v6)
Doop pages: "v5.1 · surface depth + liquid gold" (the first pass, warm and gold) and "v6 · neutral + liquid metal" (**current**). This replaces the v3 micro-shadow and the core glow.

**Surface depth** (the Libraries.dev site's material recipe, which v6 uses at its original neutral values). Depth comes from hairlines on the edge, not from bigger shadows:
- a 1 px ring
- a darker 1 px **bottom lip**
- a light 1 px **top highlight**
- a tiny contact shadow

It is close to how Windows 11's own Fluent controls build their edges.

| Token | Light | Dark |
|---|---|---|
| Token (v6) | White | Dark |
|---|---|---|
| `--ctl` (buttons, chips, card) | `0 1px 3px rgba(0,0,0,.04), inset 0 0 0 1px rgba(0,0,0,.06), inset 0 -1px 0 rgba(0,0,0,.10), inset 0 0 0 1px rgba(196,196,196,.1)` | `0 1px 1px rgba(0,0,0,.24), inset 0 0 0 1px rgba(255,255,255,.05), inset 0 1px 0 rgba(255,255,255,.07)` |
| `--ctl-pri` (primary: `#18181B` / `#FAFAFA`) | `0 1px 3px rgba(0,0,0,.18), inset 0 0 0 1px rgba(0,0,0,.4), inset 0 1px 0 rgba(255,255,255,.12)` | `0 1px 2px rgba(0,0,0,.4), inset 0 -1px 0 rgba(0,0,0,.18), inset 0 1px 0 #fff` |
| `--press` (pressed, plus scale .98) | `inset 0 0 0 1px rgba(0,0,0,.08), inset 0 1px 2px rgba(0,0,0,.12)` | `inset 0 0 0 1px rgba(255,255,255,.05), inset 0 1px 2px rgba(0,0,0,.5)` |
| `--float` (orb, pill, panel) | `0 4px 42px rgba(0,0,0,.06), 0 2px 6px rgba(0,0,0,.05), 0 0 0 1px rgba(0,0,0,.06)` | `0 4px 42px rgba(0,0,0,.45), 0 2px 6px rgba(0,0,0,.35), 0 0 0 1px rgba(255,255,255,.07), inset 0 1px 0 rgba(255,255,255,.06)` |

- **Press:** pressing drops the lip and highlight (the surface sinks) and scales to .98.
- **Cost:** all of this is static. It is drawn once into the surface and redrawn only on a state change, so it costs nothing per frame.

**Liquid metal (chrome, untinted; no gold)** is the only metal in the app, and the only attention effect:
- **When:** the orb's **rim** becomes flowing metal at the 0 s *Memasuki waktu* prayer notice only. The core stays plain: a metal core at this size reads as a blob, and the reference keeps the centre plain too.
- **The real shader, not a gradient.** v6's first mockups faked chrome with a CSS conic gradient, and it looked cheap: hard black and white stripes on a flat band. Mockups and the app now use Paper Shaders' `liquidMetal` with metal-fx's silver preset:
  - `fit` contain, `repetition` 1.5, `softness` .05, `shiftRed`/`shiftBlue` .3 (the fine spectral glints), `distortion` .1, `contour` .4, `angle` 90
  - tint `#ffffff40` on white, `#ffffff66` on dark, with a transparent backdrop
  - a 1 px white highlight along the ring's top inside edge (metal-fx `RIM_DEFAULTS`: offset 1, blur .5, alpha .9)
- **Ring thickness:** **about 2.7 px at 1×** on the 48 px orb, the same proportion as the reference's roughly 2 px on a 32 px button. A thick band reads as plastic.
- **Neighbour reflection:** the edge of the pill next to the orb picks up a soft silver sheen (about 26 px, around 55% at the edge) while the metal plays. That is metal-fx's "reflections on neighbours".
- **White theme note:** silver on white has less contrast than on dark. If it reads too quiet in the build, go to 3 px or a slightly darker tint on white only.
- **Sequence:** it flows in from the top (0.4 s), the reflections make one slow turn, then it thins and fades (about 0.8 s). Total about 5 s, then the plain orb returns. It never loops.
- **Implementation:** a small liquid-metal pixel shader on a ring about 50 px wide, after Paper Shaders' `liquidMetal` (Apache-2.0; check its NOTICE before porting). It runs only for those 5 s.
- **Reduced motion:** a still chrome rim for 5 s.
- **Quiet:** in full screen and Focus Assist the notice queues and no metal plays.
- **Not adopted from that library:** border beam (an endless travelling glow), always-on metal rings, image FX, voice glow.

## Sound
Interface sounds are synthesized, not samples, after the Shujaat palette: soft mallet tones (a sine plus a quiet partial three times up, 6 ms attack, exponential decay) and small struck clicks (filtered noise with a short ring). Generated in Rust with `cpal`.

| Cue | Tone | Length | Level (relative) | When |
|---|---|---|---|---|
| `open` | 460→620 Hz, then 690→760 Hz at +70 ms | 130 ms | −50 dB | a pill appears (normal notices) |
| `done` | 620→820 Hz | 90 ms | −52 dB | Mark done, log a drink |
| `close` | 620→440 Hz | 120 ms | −53 dB | dismiss |
| `press` | click, 2.2 kHz band, 1.5 kHz ring | 14 ms | −52 dB | primary buttons |

- **No hover sounds:** the orb lives next to the cursor all day.
- **±5% random pitch** on each play, so repeats don't feel mechanical.
- **Silent** in focus, full screen, Focus Assist, and during a recorded meeting.
- **Adzan stays separate,** with its own volume and mute.
- **One switch** in settings ("Interface sounds"), on by default, remembered.

## Icons
- Fluent System Icons, 20 regular. Reason: native next to Windows 11.
- Muted by default; text colour on hover.

## Copy
- Prayer and drink notices are in Indonesian. The rest is in English until there's a language setting.
- No em dashes. No emoji in UI chrome: folder emoji are user content.

## Placeholders (R-23, R-38)
- **App name and logo:** not decided. "Clippy" belongs to Microsoft. Mockups use `[App]`.
- **Invite domain:** `[app-domain]`.
- **Sample content:** names, tasks, events and numbers in mockups are sample content, and each frame says so. Prayer times are the user's own (Yogyakarta, from their screenshot).
- **Mascot:** none. If one is wanted for empty states later, it comes from the user or an artist.
