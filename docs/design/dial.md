## Quick menu dial (v6.2)
Doop frames "v6.2 · 17 Quick menu dial (white)" (id CCI-LhVxV3) and "v6.2 · 18 Quick menu dial: branches, keyboard (dark)" (id 88aAvOibL1), page "v6.2 · mercury + thinking orbs". Values, task names and download numbers in them are sample content. Prayer times are the user's own (Yogyakarta, Kemenag). It replaces the warm v4 frame "v4 · 01 Dial menu from the orb".

### What it is
- The quick menu is a half dial that fans out of the mercury drop. It is the "Menu" state in ORB_SPEC §2.
- **Opens with** a click on the orb, a press and drag from the orb, or `Ctrl+Alt+Space` (which shows the orb first if it was hidden). Pressing the shortcut again while the dial is open hides the orb.
- **Faces the centre of the screen.** Docked right, it opens to the left. Docked left, it mirrors and opens to the right. The order stays top to bottom in both.
- **Six items, top to bottom:** Tasks, Today, Prayer, Focus, Drink, More. Keys 1 to 6 follow the same order.
- The drop stays the hub. It does not grow and nothing is drawn on it. Its rings (focus, task progress) stay visible while the dial is open.

### Geometry (1×, from the hub centre)
- **Angles** are measured from 12 o'clock toward the screen centre. Items sit at 15°, 45°, 75°, 105°, 135° and 165°. Each item owns a 30° sector.
- **Main band:** an arc at radius 104, 56 px wide (inner 76, outer 132), from 10° to 170°, with round ends. It is the dial's only surface.
  - White: `#FCFCFC`, a 1 px `rgba(0,0,0,.08)` edge, shadow `0 4px 14px rgba(0,0,0,.07), 0 1px 2px rgba(0,0,0,.06)`.
  - Dark: `#1F1F1F` (the pill fill), a 1 px `rgba(255,255,255,.08)` edge, shadow `0 10px 22px rgba(0,0,0,.55)`.
  - In the app the band uses the same Acrylic tint as the orb and pill.
- **Item chips:** 44 px circles at radius 104. Fill `active` (`rgba(0,0,0,.05)` / `rgba(255,255,255,.07)`), icon 18 px, stroke 1.6, text colour.
- **Branch band:** an arc at radius 166, 52 px wide (140 to 192), spanning only its children, round ends. Same surface as the main band, with an 8 px gap to it.
- **Child chips:** 40 px circles at radius 166, 32° apart (28° for More's four). Numbers are 13/16, 500 ("15", "250"); the rest are 16 px icons.
- **Edge clamp for children:** the fan is centred on its parent, then shifted so no child sits past 172° or before 8°. More's fan ends at 172° and runs upward.
- **Hub:** the 48 px drop, its centre 36 px from the screen edge (it slides out 12 px when the dial opens).
- **Footprint:** about 330 × 330 px for the ring, about 400 px tall with a branch open. The window for it is the pill and panel window (ORB_SPEC §1), not the orb window.

### Aiming
- **Aim is direction, not distance.** The pointer's angle from the hub picks the sector, however far out it is.
  - Closer than 30 px to the hub: nothing is aimed. A click there closes the dial.
  - From 30 px to 140 px: the main item in that sector.
  - Past 140 px, with a branch open: the child in that direction, inside the branch's own sectors. Outside the branch's span, the parent stays aimed.
- **Aimed item:** its sector of the band takes the `wedge` fill (`rgba(0,0,0,.055)` / `rgba(255,255,255,.06)`), masked to the band so the round ends stay round. The chip turns ink (`#171717` with `#FAFAFA` icon / `#FAFAFA` with `#171717` icon).
- **Label:** the aimed item's live value tag becomes a 28 px label pill, radius 14, 13/28, the band surface and the pill shadow. Two-tone: the noun in text colour 500, the value muted 400. For example **Today** `4 left · Standup 13:30`, **Focus** `25 min`, **+250 ml** `to 1250 / 2000`, **Disk** `C: 92% full`.
- **Placement of tags and labels:** on the item's ray, just outside the band (radius 138 for main items, 198 for children). The box sits outward from that point: its centre is offset by half its size along the ray, so a label at 90° is right-aligned to the point and one at 15° sits above it.
- **Branches open after 120 ms of steady aim,** so sweeping across the ring never flashes branches. While a branch is open the main value tags hide.

### Picking
- **Click mode** (click the orb): the dial stays open. Aim, then click anywhere in the sector. Clicking a parent opens its panel (Focus, Drink, Prayer); clicking a child does its action.
- **Mark mode** (press on the orb and drag 12 px, or hold 450 ms): release picks whatever is aimed. A quick flick works: out toward Drink, on toward 250, let go. Releasing past the dial still picks, because only direction counts.
- **Cancel:** Esc, a click outside the dial, or releasing on the hub. Nothing changes.
- **Confirmation:** a pick that changes something (log a drink, start focus, mute the adzan) closes the dial and shows a quiet pill with Undo. Example: cup chip, **+250 ml** `1250 / 2000 ml`, action Undo. It follows the pill rules: 48 px, one action, gone after 6 s, done tone for a logged drink.
- Picks that open a panel morph the orb into that panel instead (surface morph, `grow`).

### Items, live values and branches
| # | Item | Value tag (live) | Click on the item | Branch |
|---|---|---|---|---|
| 1 | Tasks | "3 due" (due today, not done) | Tasks panel | none |
| 2 | Today | "4 left" | Today panel | none |
| 3 | Prayer | next prayer and time, "Jumat 11:26" or "Ashar 14:41" | Prayer panel | Mute next adzan, Qibla, Reminders |
| 4 | Focus | minutes left while a session runs, "18 min" | Focus panel | 15, 25, 50 min |
| 5 | Drink | "750 / 2000" | Drink panel | 150, 250, 500 ml |
| 6 | More | a running download, "64%" | nothing (branch only) | Downloads, Disk, Meeting notes, Settings |

- Value tags are 22 px pills, radius 11, 12/22 muted, on the band surface with the pill shadow. They are surfaces, never bare text, because the desktop behind the dial can be anything.
- Values update live while the dial is open. A value uses `tnum`, so it never jiggles.
- On Friday, Prayer shows "Jumat" for Dzuhur when "Show as Jumat" is on.
- **Focus chip while running:** the focus ring around the chip (radius 25.5, dotted `0 5` track, 2 px solid arc for time spent), the same motif as the orb's focus ring.
- **Downloads chip while running:** the task progress ring (radius 23.5, 1.5 px hairline track, 2 px arc). The drop carries the same ring.
- **Branch details:**
  - Focus 15 / 25 / 50: 25 is the default child. While a session runs, the Focus branch becomes Pause, Skip to break, Stop.
  - Drink 150 / 250 / 500: 250 (the default glass from Drink settings) is the default child. Picking logs at once, no panel.
  - Prayer: Mute next adzan (label "Mute next adzan" `Ashar 14:41`), Qibla (`295° WNW`, opens the Qibla view), Reminders (opens the reminders editor). Once muted, the first child reads "Unmute adzan". Muting only silences the sound; the pill still shows.
  - More: Downloads, Disk (`C: 92% full`), Meeting notes, Settings. Each opens its panel.

### Prayer rule
- The Prayer item and its branch only inform. There is never a check, a done or missed state, a count or a "did you pray?". The value is the next time only.

### Keyboard
- `Ctrl+Alt+Space` opens the dial at the orb in keyboard mode, with Tasks aimed. Every main item then shows a tag with its number keycap and name ("1 Tasks"). The aimed item shows its label with the keycap and live value ("3 Prayer Ashar 14:41").
- `↑` `↓` go round the ring. `Tab` and `Shift+Tab` do the same. `Home` and `End` jump to Tasks and More.
- `Enter` picks the aimed item, or opens its branch. A branch opens on its default child (Focus 25, Drink 250, Prayer and More on the first child).
- `←` goes into the branch (toward the screen centre). Docked left, this is `→`.
- `Esc` steps back: branch to ring, ring to closed. The arrow away from the screen centre also steps back.
- `1` to `6` jump to an item (and open its branch). In a branch, `1` to `4` pick a child.
- **Focus ring:** always visible on the aimed chip: `0 0 0 2px` band colour, then `0 0 0 4px` text colour. Never hidden (Ovio hides it; we don't).
- Keycaps: 16 px tall, radius 5, `raised` fill, a 1 px edge and a 1 px lip, 11/16 500.
- **Accessibility:** the dial is a `menu` with `menuitem`s; branch parents have `aria-haspopup` and `aria-expanded`. The live value is part of each item's name ("Drink, 750 of 2000 ml").

### Motion
- **Open:** the band unfolds from the drop on `grow` (360 ms). Chips ripple in from the middle out (`ripple`, 30 ms apart). The `open` tone plays.
- **Branch:** after 120 ms of aim, the branch band and children fan out from the parent on `settle` (250 ms).
- **Close:** folds back into the drop on `shrink` (200 ms). A pick plays `press`; Esc or a click outside plays `close`.
- **Reduced motion:** every step is a 150 ms fade. Nothing moves.

### States
- **Loading:** a value tag holds a 30 × 6 `track` bar for up to 2 s, then the value fades in.
- **No location:** Prayer's tag reads "Set place". Picking Prayer opens Place.
- **Sync error:** an ember badge (16 px, `#B3402F` / `#E2614F`, white "!", 2 px band ring) on the Tasks chip and an ember "Can't sync" tag. Tasks still opens from this PC's copy and retries in 30 s.
- **Nothing to say:** no tag at all (More with no download, Focus when idle). Never a placeholder.
- **Orb hidden:** the shortcut brings the orb out with the dial already open. When the dial closes, the orb hides again.

### Rules
- Only the real sprite is metal. The band, chips and tags are neutral; no hue except ember for errors.
- Weight 500 at most. Inter with `cv11` and `tnum`.
- At most one branch open at a time. Children never show a second level.
- The dial never steals focus from the app behind it in click mode. In keyboard mode it takes focus until it closes, then hands it back.
- Sample content in mockups is labelled as such.
