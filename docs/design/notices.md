## Notices (v6.2)
Doop frames "v6.2 · 19 Notice stack and queue (white)" (`Y_alKzOsBY`) and "v6.2 · 20 Quiet rules: full screen, focus, mute (dark)" (`IcwMYj4XTv`), page "v6.2 · mercury + thinking orbs". Notice texts, names, times and counts in them are sample content. The sample "now" is 14:20 on Jumat, 6 Nov 2026, in Yogyakarta.

This section is meant to replace "Notice stack (v3.1)" in DESIGN.md. The v3.1 numbers stay. Everything else below is new or changed.

### Layout
- **Frame 19 (white):** three columns of 400 px, `minmax(0,1fr)` and 300 px, with 28 px gaps.
  - Column 1: the stack on the orb (collapsed, expanded, and tucked with the badge), then a Pacing card.
  - Column 2: priorities, with one pill from each feature, then the empty state.
  - Column 3: the Notices panel ("Earlier today"), the row menu, and the "Downloads muted" confirmation.
- **Frame 20 (dark):** three columns of `minmax(0,1fr)`, `minmax(0,1fr)` and 380 px, with 28 px gaps.
  - Column 1: full screen or presenting, then "Counts as full screen" and "In a meeting".
  - Column 2: focus session, then the attention shake and Pause notices.
  - Column 3: the "What still breaks through" matrix, then the Notice sources settings.
- **Stages:** `#EFEFEF` white / `#141414` dark, radius 20, with a hairline. The orb sits on the right edge, next to a 1 px screen edge line (`rgba(0,0,0,.16)` / `rgba(255,255,255,.14)`). Captions under the pills are 12/16 muted and right aligned.

### The notice
- `{id, source, priority: urgent | normal | quiet, title, value?, action?, ttl}`.
- **Pill:**
  - 48 px tall, radius 24, with a 36 px icon chip.
  - A two-tone label: the noun in text colour, the value in muted.
  - At most one 32 px quiet action, and an optional 28 px close.
  - Background `#FCFCFC` white / `#1F1F1F` dark.
- **Chips by source:**
  - prayer at the time: a 10 px filled ink dot
  - prayer offsets: a bell
  - tasks: a rounded checkbox
  - drink: a cup
  - calendar: a video camera
  - downloads: an arrow pointing down
  - disk: a drive
  - family activity: a check
  - summaries: a bell
- **Time on screen:** 6 s of visible time without hover, and 8 s for errors. Hovering pauses the clock. Pressing the action also dismisses the pill.

### Priorities
| Priority | Sound | Shake | Queue | While hidden | Examples |
|---|---|---|---|---|---|
| urgent | yes | once (see below) | jumps to the front, never merges | shows the orb and pill, then hides again | "Memasuki waktu Ashar" (Mute adzan); "Standup starts now" (Join) |
| normal | yes | never | waits its turn: priority first, then age | waits and adds to the badge | "Kirim invoice due 14:00" (Done); "Minum dulu 750 / 2000 ml" (+250 ml); "10 menit lagi Ashar 14:41" (no action); errors |
| quiet | no | never | goes straight to the list if anything else is showing | list only | "Ayah finished Gula 1 kg · Belanja" (no action) |

- Prayer reminders at −30 and −10 min are normal. By default only two things are urgent: prayer at the time, and a meeting starting.
- Drink is always normal, and it never shows while the orb is hidden.

### The stack
- **Collapsed:** the newest pill is in front. Up to two older ones peek out **below** it:
  - offset 6 px and 12 px
  - scale .96 and .92, from the bottom centre
  - opacity .7 and .45

  The peeking pills take the front pill's width and show no text, so the edge reads as one clean layer. There is no rotation: the stack is a column, never a fan.
- **Badge:** it counts the notices waiting behind the front pill (4 in the sample).
  - White theme: ink `#171717` with `#FAFAFA` text. Dark theme: `#FAFAFA` with `#171717` text.
  - 18 px tall, radius 9, 11/18 at 500, with a 2 px ring in the stage colour.
- **Tucked, or brought back from hidden:** the badge sits on the visible half of the drop. It stays at full strength while the drop is at 40%.
- **Hover the stack:** it spreads into a column with 8 px gaps (`settle`, 400 ms).
  - At most 3 pills show.
  - The rest sit behind a 28 px "2 more · open the list" button, which opens the Notices panel.
- **Pacing:**
  - A new pill waits at least 1.5 s after the last one appeared.
  - Notices of the same kind from the same source merge: "3 downloads done · Videos", with Show.
  - Urgent jumps to the front and never merges.

### Notices panel (Earlier today)
- **Opening:** from the menu or from "2 more". The panel is radius 20 with 12 px padding.
- **Header:** "Notices" (14/18, 500), a muted "Today", and a 28 px "Clear all" button.
- **Groups:** "Waiting · 2" and "Earlier", each with a 12/16 muted label.
- **Rows:** 40 px, radius 8, left to right:
  - a 6 px ink dot on unseen notices
  - a 24 px icon chip (radius 7)
  - the two-tone label on one line, with an ellipsis if it runs long
  - the time (12/16 muted) on the right
- **Hover and keyboard focus:** a raised fill and a 2 px ink focus ring. A 28 px round "···" button takes the place of the time.
- **Row menu** (open it with ···, a right-click or Shift+F10):
  - Mark as seen.
  - A muted heading naming the source ("Downloads"), then Mute for 1 hour, Mute until tomorrow, and Don't show downloads again.
  - Items are 32 px with radius 8. The menu is radius 12.
- **After a mute:** a small confirmation, "Downloads muted · for good", with Undo.
- **Clear all** empties the list and the badge. Undo stays for 6 s.
- The list clears at midnight.
- **Prayer notices never stay in this list.** They inform at the time and then leave, so the list can never read as a prayer log.
- **Empty state:** an inbox chip, "Nothing today", and "Notices you missed or dismissed show here until midnight."
- **No loading or error state:** the list is local. If the notice store fails to open, the panel shows the empty state and the error goes to the log.

### Quiet rules
- **Prayer reminders (2026-11-07, the user's rule).** "30 menit lagi" and "10 menit lagi" pass every quiet time except presenting: focus, full screen (silently, the orb shows for the pill and hides again), a pause, and the orb hidden for a full-screen app. Presenting (Windows presentation mode) holds them; one that cannot show within 2 min of its moment is dropped, and a newer one for the same prayer replaces it, so a reminder is never shown late. "10 menit lagi" stays until you close it: covered by a panel or the dial it comes back, and a new normal notice takes the front for its turn and then gives it back. Hidden by you, prayer reminders are skipped. Screen sharing is not detected yet (see followups).
- **Full screen or presenting.** This applies when `SHQueryUserNotificationState` reports busy, presentation mode or Do not disturb, or when the foreground window is full screen. It is checked on every foreground change. Only a full-screen window on the orb's own monitor counts.
  - The orb hides itself. Normal and quiet notices queue without a sound.
  - Urgent still shows, with no sound and no shake. Adzan stays silent. The orb hides again when the pill closes.
  - After you leave full screen, one summary pill appears ("3 notices · waited", with Show), along with the badge. Show opens the list. Waiting notices never come out as a run of pills.
- **Focus session:**
  - Normal notices are held until the break.
  - There is no badge while focus runs. Hovering the orb shows "Focus · 18 min left · 2 held".
  - A meeting starting comes through, with sound and one shake.
  - Prayer at the time pauses focus (see "Focus (v6.2)").
  - At the break, one pill: "Break 5 min · 2 waited", with Show.
- **In a meeting** (detected or recorded):
  - Sounds are off.
  - Normal notices wait until the meeting ends. Drink waits too.
  - Urgent still shows, silently.
- **Pause notices** (from the menu or the tray): 30 min, 1 hour, until tomorrow, or off.
  - It works like focus: prayer time and a meeting starting still show.
  - Everything else waits and arrives as one summary.
- **Attention shake:**
  - Urgent notices only. ±3 px.
  - It shakes once on arrival, and again only while still unread. Shakes are at least 5.5 s apart, at most three in all (0 s, 5.5 s, 11 s). Then the pill stays still.
  - Never while full screen.
  - Under reduced motion, the chip fades once instead.

### What still breaks through
| | Focus | Full screen | Hidden |
|---|---|---|---|
| Prayer at the time (urgent) | Shows | Silent | Shows |
| Meeting starting (urgent) | Shows | Silent | Shows |
| Prayer −30 / −10 min (normal) | Shows | Silent | Skipped if you hid the orb |
| Task due, drink (normal) | Waits | Waits | Badge |
| Family activity (quiet) | List | List | List |
| A muted source | Off | Off | Off |

- **Silent:** the pill shows with no sound and no shake.
- **Waits:** it queues until the break or the end.
- **Badge:** it adds to the orb's count.
- **List:** it appears only in Earlier today.

### Notice sources (Settings)
Each row is 40 px, with a 24 px chip, a two-tone label, and either a 32 × 20 switch or a 28 px button.

| Source | Label value | Control |
|---|---|---|
| Prayer | set in Prayer | Reminders |
| Calendar | start is urgent | switch |
| Tasks | overdue joins the badge | switch |
| Drink | never while hidden | switch |
| Family activity | muted per space | Spaces |
| Meeting detected | 2 min cooldown | switch |
| Disk space | once a day | switch |
| Downloads | muted today | Unmute |

- Prayer at the time can't be muted here. Only its adzan sound can.
- **Cooldown:** after a pill from a source, the next pill from that source waits until the cooldown ends, and merges with others where it can. Meeting is 2 min and disk is once a day. The other sources have none.

### Rules
- **Prayer is not a task.** No prayer notice is ever checkable, counted, or kept in a history list. Reminders never ask whether you prayed.
- One pill at a time in front.
- After any quiet time, waiting notices come back as one summary, never a run of pills.
- Only urgent notices shake or show while the orb is hidden.
- **No colour for priority.** Urgent is shown by the ink tag, the filled dot chip and the shake. Ember red stays for errors and recording.
- **Keyboard:**
  - Tab reaches the stack, and the arrow keys move between pills.
  - Enter runs the action, and Esc dismisses.
  - S snoozes where it applies.
  - A visible 2 px focus ring everywhere.
