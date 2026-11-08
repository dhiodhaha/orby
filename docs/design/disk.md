## Disk (v6.2)
Doop frames "v6.2 · 13 Disk: low space, scan, results (white)" and "v6.2 · 14 Disk: big folders, clean up safely (dark)". Drive sizes, folder and file names, dates and counts in them are sample content.

The scanner is Arbor (`~/treesize-rea`, code name only). [App] shows its results in the Disk window. It never cleans anything by itself.

- **Window (v6.3):** Disk is its own window, like Settings, not a panel beside the orb, so choosing what to move to the Recycle Bin has room and the keyboard.
  - An ordinary window with a taskbar button and Alt+Tab. It is not topmost, and a click elsewhere leaves it open. It takes the keyboard when it opens.
  - 756 px wide and 704 px tall, less on a short screen (the work area minus 32 px, at least 480 px). It is not resizable. It opens centred on the orb's screen and does not remember where it was, the same as Settings.
  - A 44 px title band: "Disk" (13/18, 500) on the left and a 28 px round close button on the right, with a hairline under them. Dragging the band's empty part moves the window. Radius 16, hairline edge.
  - Every screen spans the full width. The lists stretch to the window's height: about 9 folder rows plus the tail row beside a taller treemap, 10 file rows, and 12 Review rows, where the panel showed 6, 7 and 9. State cards sit under the list and take their room from it, never below 4 rows. The selection bars sit at the bottom edge.
  - The drive list's heading is "Pick a drive". Safe to clean's back button moves to the left (an arrow, like Review), so the band's close button is the only cross.
  - It closes with Esc (after the confirm sheet, which Esc cancels first), the close button, Alt+F4 or the taskbar. Every way in opens the window: dial More → Disk, tray 40, Find big folders, Open on Scan done, Retry. The pills that say "only when the window is closed" mean exactly that.
  - The confirm sheet is 420 px wide, centred under the band, with a scrim over the whole window.

- **Disk v2 (v6.3, built):** Doop page "v6.3 · Foundation + Disk v2", frames 02 Results (white), 03 Scanning live, 04 Safe to clean, 05 Confirm, 06 Results (dark), on the shared tokens of frame 01 Foundation. Where v2 and the v6.2 sections below disagree, v2 wins. Drawing is `disk_panel.rs`, state and motion `disk.rs`, the frame timer `disk_window.rs`.
  - **Header row** (y 16, 32 px): a drive chip, a 32 px pill in `active` with a hairline, holding a 24 px round icon chip on the panel colour, "(C:) Windows" (13/500) and "· scanned 09:12" muted (an older scan says the date). Clicking the chip goes back to Pick a drive. On the right, quiet 28 px buttons: Safe to clean and Scan again (Stop while scanning).
  - **Headline:** "438 GB used" (26/30, 500, −0.02em), then "of 476 GB" (13, muted) on the same baseline, and "38 GB free" (13, muted) right aligned. A folder scan reads "1.5 GB in clippy" with its file count on the right.
  - **Capacity bar:** 12 px, fully round at the ends, segmented by the drive's top-level items with 2 px gaps and a 4 px minimum: the four biggest in the first four greys, then System files (in use but not counted: restore points, metadata; shown only above 0.5% of the drive), Other, and Free. Free is a light wash with a hairline (`rgba(0,0,0,.05)` / `rgba(255,255,255,.05)`), never `#D4D4D4`. A legend sits under it (8 px swatch radius 2, 12 px muted, 14 px apart; what does not fit is left out).
  - **Folder greys,** biggest first: white `#171717 #525252 #8A8A8A #B5B5B5 #D4D4D4 #E5E5E5`; dark `#FAFAFA #C2C2C2 #8F8F8F #666666 #4A4A4A #303030` (the lightest is the biggest). The palest takes a hairline. The Types bar uses the same six.
  - **Linked hover:** pointing at a bar segment, a tile or a list row lights that one; every other segment and tile fades to 30% over 150 ms, and the matching row takes the hover fill. A segment with no row (System files, Other, Free) only lights itself.
  - **Toolbar** (y 158): the breadcrumb shows the folder being viewed ("C:" alone at the top level, "C: › Users" inside), earlier levels muted and clickable; the Folders / Files / Types control on the right (12/500, 28 px items in a 32 px track).
  - **Treemap** 300 px wide, as tall as the list (384 px in the full window). Squarified, 2 px gaps, radius 6, neutral alpha fills (×1.4 in dark). A row gets its own tile only if the tile is at least 80 × 36; the rest roll into "N smaller". Labels: name and size at 60 × 32 and up, the name alone at 48 × 18 and up, nothing smaller. Hovering a tile shows a label card (radius 8, hairline, raised fill, 12 px right of the pointer, kept inside the map). The hovered and the selected tile take a 2 px ink inset ring.
  - **List** beside the map: a column header ("Name", "%", "Size", 12 muted), then 36 px rows: a 16 px checkbox, the 24 px chip, the name (500) with "hidden" or "system" muted, a 56 × 4 px bar against the biggest sibling, the percent column 44 px (12 muted, right) and the size column 64 px (13/500, right). Hover is half of active (`rgba(0,0,0,.025)`), selected and ticked rows take `active`. Sizes and percents have one decimal with ".0" dropped ("182 GB", "9.4 GB", "41%", "30.6%"), base 1024 like Explorer, tabular figures. The tail row "6 more items" is muted with no checkbox.
  - **Detail strip** (44 px, hairline above) under the map: the hovered or selected item's name (13/500) and path (12 muted, cut in the middle) on the left; "30.6% · 18 GB" and Open in Explorer on the right. With nothing in focus it describes the folder on show. When rows are ticked, the selection bar takes its place.
  - **Scanning, live inside Results:** the scan opens Results at once. The chip says "· scanning", the headline "251 GB counted" with "of 438 GB used", and the right side "4 of 9 folders" (a top-level folder counts as done once its bytes have held for 1.5 s and the scan is no longer inside it). The bar shows the counted folders ("Users, so far" for the one being counted), a dotted segment for used space not counted yet, then free. The map and list fill in as the scan runs; a dotted tile stands for the uncounted space and skeleton rows (8 px bones) hold the empty places, with a shimmer only while scanning. A veil (the window colour at 60%) covers map and list, with a centred 40 px pill: a 28 px chip with the progress ring (1.5 px track, 2 px ink arc) and a search glyph, "Scanning Users" (13/500) and the current path, cut in the middle. The strip reads "512,340 files so far · 0:31" and "Results fill in as each folder finishes". Only Stop answers under the veil.
  - **When the scan ends:** tiles reflow to the final layout on cubic-bezier(.7,0,.3,1) over 700 ms and the numbers roll on; once the totals have held for 1.5 s the veil fades over 400 ms. Rows entering (a new level, tab, list or filter) ripple in: y 6 → 0 and opacity 0 → 1, 200 ms each, 30 ms apart.
  - **Safe to clean** (frame 04): back arrow, "Safe to clean" (14/500) with "C:, from the 09:12 scan" muted, and the total "43.5 GB" (20/500) "to review" on the right. Filter chips with counts: All, Safe, Check first (28 px; the chosen one is ink with panel text). A scrolling list of groups: "Suggested" (the three suggestions above), then the cleaning rules' groups (Developer caches, App caches, Browser caches, AI models, Crash reports), each heading with "count · size" on the right. Rows are 42 px: checkbox, 32 px round chip, name (500) over its one plain line (12 muted), the tag right aligned in a 112 px column, then the size (13/500). Clicking a row's body opens a suggestion's Review list, or a rule's folder in Explorer. "Not counted above" (Recycle Bin, System files) ends the list.
  - **Tags:** "Check first" is a 20 px pill with a 1 px border in ink at 40%, ink text (11/500) and a 12 px eye glyph; "Safe to remove" is the `active` fill with muted text. Same rule in dark (border `#FAFAFA` 40%, fill `rgba(255,255,255,.08)`, text `#9E9E9E`).
  - **Selection bar** (56 px, hairline above, used by Safe to clean, Review, Files and ticked folders): "3 selected" (13/500) "· 11.3 GB" muted on the left; Clear and the primary "Move to Recycle Bin (11.3 GB)" (32 px, ink) on the right, with Open in Explorer where it applies. The primary's width springs (k 700, c 53) as the size suffix appears, and the suffix fades in over 150 ms.
  - **Confirm sheet** (frame 05): 420 px, radius 20, padding 20, over a black scrim (32% white, 50% dark). Title "Move 5 items to the Recycle Bin?" with a 32 px trash chip. The "Check first" group comes first: its heading with "1 · 6.1 GB", a warning line in ink with a 16 px outline "!" ("If you're not sure about one, cancel and untick it."), and a sunken well whose rows show the name and its path. Then "Safe to remove" with its well. Up to five lines per group, then "and N more". A muted note ("You can put them back from the Recycle Bin until it's emptied. [App] never empties it for you."), then a footer under a hairline: "Moving 11.3 GB to the Recycle Bin" muted on the left, Cancel (focused, 2 px ring) and "Move 5 to Recycle Bin" on the right. Your own files from Results and Files always sit under Check first. Cleaning-rule folders move only through `recycle_rule_items`, which checks every path again.
  - **Dark** (frame 06): the same layout on the dark tokens; folder greys flip so the biggest is the lightest; selected tab `rgba(255,255,255,.1)`.
  - **Motion rules:** every motion ends at a fixed time and the window's frame timer stops when nothing moves, so an idle window costs nothing. Windows "Animation effects" off snaps all of it: no reflow, ripple, shimmer, roll or fade.
  - **Keyboard:** arrows move in the list (Left and Right switch tabs in Results and filters in Safe to clean), Space ticks, Enter opens, Backspace goes up, Delete opens the confirm for what is ticked (or the row under the keyboard), Esc cancels the confirm sheet first and then closes the window.

- **Layout:**
  - Frame 13 has two columns: on the left, the orb stage (pills) above the Scanning panel (400 px wide). On the right, the Results panel (756 px) with two state cards under it.
  - Frame 14 has three columns: the drill-down panel and orb stage (404 px), the confirm dialog and three state cards (340 px), and the Safe to clean panel (384 px) with its empty state.
- **Scanning panel:**
  - Header: "Scanning C:" (14/18, 500) with a small Stop button (28 px) on the right. Stop keeps what was counted.
  - Big number "41%" (26/30, 500, -0.02em) with "of the 438 GB in use on C:" (12/16, muted). The percentage is bytes counted over the volume's used bytes, so it is honest without knowing the file count in advance.
  - A 4 px progress bar: track `#D4D4D4` / `#3A3A3A`, fill ink, radius 2.
  - Three stat cells in `raised`, radius 12: files, folders, time so far. Numbers are 500, labels muted.
  - The current path in 12/16 muted with a folder icon, cut with an ellipsis at the end.
  - "Biggest so far": three 36 px rows (folder chip, name, a 64 × 4 px size bar, size). It fills in while the scan runs.
- **Results panel:**
  - Header: a 32 px drive chip, "C: Windows" (14/18, 500), "Scanned today at 09:12 · 1,204,318 files in 0:52" (12/16 muted), and Scan again (28 px, refresh icon).
  - Capacity bar, 8 px, radius 4: used in ink, free in `track`. Under it: "**438 GB** used, 92%" and "**38 GB** free of 476 GB".
  - Toolbar: a 28 px back button, the breadcrumb (previous levels muted with 12 px chevrons in `passed`, the current level in 500 ink, then its size muted), and a segmented control: Folders, Files, Types.
  - Treemap, 336 × 252, squarified, 2 px gaps, tile radius 6. Area is size. Tiles are neutral grey only (`rgba(0,0,0,.10)` down to `.035` on white; white alpha on dark), darker for bigger. No hue, no cushion shading. Labels are name (500) over size (400), both in text colour, because muted grey fails AA on the darker tiles. Nested levels show as inner tiles in `rgba(255,255,255,.45)` with a hairline. The selected tile has a 2 px ink inset ring.
  - Folder list beside it: 36 px rows, radius 8. A 24 px folder chip (radius 7), name in 500 (with "hidden" muted when the folder is hidden), a 64 × 4 px bar relative to the biggest sibling, size (right aligned, `tnum`) and percent of the parent (muted). The tail row is "6 more items".
  - The treemap and list share one selection. Hover or keyboard focus on a row swaps the bar for two 28 px icon buttons: Open in Explorer and Move to Recycle Bin.
  - File types: a 10 px stacked bar of six greys (`#171717`, `#525252`, `#8A8A8A`, `#B5B5B5`, `#D4D4D4`, and `#E5E5E5` with a hairline for Other). A six-column legend with an 8 px swatch, name in 500 and size muted. Grey order goes from biggest to smallest, so the legend never relies on telling hues apart.
- **Drill-down (Files tab):**
  - Breadcrumb "C: › Users › You › Downloads", with Open in Explorer as a 28 px icon button at the right end.
  - "214 files · 33.5 GB" next to the tabs.
  - Rows are 36 px with a 4 px gap: a 16 px checkbox (radius 5), a type chip (archive, video or file), the name (500, ellipsis at the end), the modified month (muted, 58 px) and the size (56 px, right aligned). Selected rows have the `active` fill and a filled checkbox.
  - The tail row is "208 more files", muted, with no checkbox.
  - The selection bar under a hairline: "**2 selected** 11.3 GB", a keyboard hint, then two equal 28 px buttons: Open in Explorer and Move to Recycle Bin.
  - Keyboard: Space selects, Enter opens a folder, Backspace goes up, Delete opens the confirm. Every row and tile takes a visible 2 px focus ring (ink on white, `#FAFAFA` on dark, offset 2 px).
- **Confirm dialog:**
  - Title "Move 2 items to the Recycle Bin?" with a 32 px trash chip.
  - A sunken well (`raised`, radius 12) lists each item with its size. More than 5 items collapse to "and 12 more".
  - Body: "You can put them back from the Recycle Bin until it is emptied."
  - Honest muted line: "The 11.3 GB comes back only when the Recycle Bin is emptied. [App] never empties it for you, and has no permanent delete."
  - Buttons: Cancel (quiet) and Move to Recycle Bin (primary). Focus starts on Cancel, so a stray Enter does nothing harmful.
  - The move uses the shell (`IFileOperation` with `FOFX_RECYCLEONDELETE`). Shift+Delete opens the same confirm and still recycles.
- **Safe to clean panel:**
  - Header "Safe to clean" and "C:, from the 09:12 scan". The big number "23.0 GB" with "to review, nothing goes until you confirm".
  - Each suggestion is a row with a 32 px round chip, a title (500), one or two lines of plain truth (12/16 muted), an honesty tag (20 px pill, 11 px 500), and on the right the size (500) over a Review button (28 px).
  - Tags: "Check first" is an outline pill and is used for the user's own files. "Safe to remove" is a filled `active` pill and is used for things Windows or apps rebuild. Tags are words, never colours.
  - Suggestions:
    - **Downloads older than 90 days.** "38 files downloaded before 8 Aug. They are yours, so look first." Check first. Age uses the file's created or modified date, not last access, because Windows does not always update last access.
    - **Windows temp files** (`%TEMP%` and `C:\Windows\Temp`). "Windows makes new ones when it needs them. Files in use are skipped." Safe to remove.
    - **Browser caches** (Edge and Chrome cache folders only, never cookies, history or passwords). "Sites load a bit slower for a day. You stay signed in." Safe to remove. If the browser is open, Review says to close it first.
  - "Not counted above" (a muted label under a hairline):
    - **Recycle Bin**, 1.6 GB. "Already deleted. Emptying it is permanent, so that stays your call." The button is Open, which opens the Recycle Bin in Explorer.
    - **System files.** "Update leftovers and restore points. Windows manages these itself." The button is Settings (`ms-settings:storagesense`). No size is shown, because [App] does not measure them.
  - Footer with a shield icon: "Everything goes to the Recycle Bin. [App] never cleans by itself."
  - Review opens the drill-down list filtered to that suggestion, with nothing pre-selected, and then the same confirm.
- **Cleaning rules:** caches that a tool or app rebuilds by itself, beyond the three suggestions above. The rules are one static table in `crates/disk/src/rules.rs` (`RULES`), data rather than scattered code. Rule structure and safety lessons adapted from Purge (MIT, see `THIRD-PARTY-NOTICES.md`); the Windows rules are our own.
  - Each rule has an id, a group, a display name, one or more paths starting with `%LOCALAPPDATA%`, `%APPDATA%` or `%USERPROFILE%` (a `*` part matches one real folder, never a junction), a tier (Safe to remove or Check first) and one plain line on what removing it costs and how it comes back.
  - Groups, in this order: Developer caches, App caches, Browser caches, AI models, Crash reports.
  - | Rule | Group | Tier | Paths | Line |
    |---|---|---|---|---|
    | npm cache | Developer | Safe | `%LOCALAPPDATA%\npm-cache\_cacache`, `\_npx` | Packages npm already downloaded. Re-downloaded on your next install. |
    | Yarn cache | Developer | Safe | `%LOCALAPPDATA%\Yarn\Cache`, `\Yarn\Berry\cache` | Packages Yarn already downloaded. Re-downloaded on your next install. |
    | pip cache | Developer | Safe | `%LOCALAPPDATA%\pip\Cache` | Re-downloaded the next time you install them. |
    | uv cache | Developer | Safe | `%LOCALAPPDATA%\uv\cache` | Re-downloaded on your next uv sync. |
    | Cargo registry | Developer | Safe | `%USERPROFILE%\.cargo\registry\cache`, `\registry\src` | Re-downloaded on your next build. Installed tools stay. |
    | Cargo git checkouts | Developer | Safe | `%USERPROFILE%\.cargo\git` | Fetched again on your next build. |
    | Gradle wrapper downloads | Developer | Safe | `%USERPROFILE%\.gradle\wrapper\dists` | Re-downloaded on your next build. |
    | Gradle caches | Developer | Check first | `%USERPROFILE%\.gradle\caches` | Offline builds need it. The next online build downloads every dependency again. |
    | NuGet download cache | Developer | Safe | `%LOCALAPPDATA%\NuGet\v3-cache` | Re-downloaded on your next restore. |
    | Go build cache | Developer | Safe | `%LOCALAPPDATA%\go-build` | Rebuilt on your next build, which is slower once. |
    | NuGet packages | Developer | Check first | `%USERPROFILE%\.nuget\packages` | Restored on the next build, which needs the internet and takes a while. |
    | Maven repository | Developer | Check first | `%USERPROFILE%\.m2\repository` | Downloaded ones come back; ones you built with mvn install do not. |
    | pnpm store | Developer | Check first | `%LOCALAPPDATA%\pnpm\store`, `%USERPROFILE%\.pnpm-store` | Shared through hard links, so it may free less than shown. |
    | VS Code caches | App | Safe | `%APPDATA%\Code\Cache`, `\CachedData`, `\CachedExtensionVSIXs` | VS Code rebuilds them; its next start is a little slower. |
    | Discord, Slack, Teams cache | App | Safe | `Cache` and `Code Cache` under each app (new Teams: `EBWebView\*\Cache`) | Loaded again as you use it. You stay signed in. |
    | Firefox cache | Browser | Safe | `%LOCALAPPDATA%\Mozilla\Firefox\Profiles\*\cache2` | Sites load a bit slower for a day. You stay signed in. |
    | Hugging Face downloads | AI models | Check first | `%USERPROFILE%\.cache\huggingface\hub` | Can mean many gigabytes to get again. Your sign-in token stays. |
    | Ollama models | AI models | Check first | `%USERPROFILE%\.ollama\models` | Pull them again to use them. |
    | App crash dumps | Crash reports | Safe | `%LOCALAPPDATA%\CrashDumps` | Only useful for debugging those crashes. Nothing comes back. |
    | Windows error reports | Crash reports | Safe | `%LOCALAPPDATA%\Microsoft\Windows\WER\ReportArchive` | Reports Windows already sent. Nothing comes back, and nothing needs to. |
  - **Never touched** (`NEVER_TOUCH`, `NEVER_REMOVE_ROOTS`, `NEVER_NAMES`): the profile, AppData and Local AppData folders themselves; any browser or app profile root (`User Data`, `Default`, `Profile N`, a Firefox profile); each tool's home (`.gradle`, `.cache`, `.cache\huggingface`, `.cargo`, `.nuget`, `.m2`, `.ollama`, and the app folders the caches sit in); `.cargo\bin`; anything named Service Worker, IndexedDB, Local Storage or Session Storage; and credentials (`.ssh`, `.gnupg`, `.aws`, `.azure`, `.kube`, `.docker`, `.git-credentials`, `.npmrc`, Cargo credentials, `gradle.properties`, Maven settings, the Hugging Face token, the Ollama key, `%APPDATA%\NuGet`, Windows Credentials, Protect and Vault). A path inside a never-touch entry, or one that holds it, is refused too.
  - A rule shows only when one of its folders exists, passes the checks below and holds more than 0 bytes. Sizes come from the same scanner as Results (logical bytes), measured on a background thread that a cancel stops (`RuleMeasure`). A folder that turns out to hold app state (a `NEVER_NAMES` folder inside) is not offered.
  - **Checks**, when measuring and again just before the move (`check_removal`): the path is absolute and plain (no `.`, `..`, wildcards or `\\?\`); it is not on the never-touch list; it is the rule's own folder or inside it; and neither it nor any folder between Local AppData, AppData or the profile and it is a junction or other reparse point. A refused path is reported as `Refused` and never reaches the shell. A wrong rule can only narrow what is offered, never widen it.
  - Removal is the Recycle Bin only (`SHFileOperationW` with `FOF_ALLOWUNDO`), like everything else in Disk. `move_to_recycle_bin` itself also refuses the profile, AppData, Windows and Program Files folders (and anything holding them) for every caller.
  - **Screen:** filter chips "All 12", "Safe to remove 8", "Check first 4"; group headings; rows with the name, the plain line, the honesty tag, the size and a checkbox. Nothing is pre-selected, and "select all" takes Safe to remove rows only: Check first rows are ticked one by one. The selection bar reads "3 selected · 11.3 GB". The confirm sheet lists Check first rows first, then Safe to remove, each block with its total, then the usual honest line with the whole total.
  - **Unfinished downloads:** in the Files list, a file ending in `.crdownload`, `.part` or `.tmp` shows "Unfinished download" in its muted line.
- **On the orb (pills):**
  - **Low space:** a drive chip, "**C:** has 38 GB left", one action, Find big folders (which starts the scan and opens the Disk window on Scanning). Normal priority, the open tone, gone after 6 s. It fires when C: free space drops below 10%, checked every 10 min. It shows once, then only again after free space drops by another 5 points. It never shows while the orb is hidden or in full screen.
  - **Scanning:** the thin progress ring around the drop (svg 64, r 30, 1.5 px hairline track, 2 px ink arc from 12 o'clock) shows bytes counted over used bytes. Hovering shows "**Scanning C:** 41% · 512,340 files" with no action. No sound.
  - **Scan done:** "**Scan done** C: 438 GB used" with Open. It shows only if the Disk window is closed. The ring closes with one ripple and the done tone plays.
  - **Moved:** "**In Recycle Bin** 2 items" with Undo, which restores the items to where they were. Normal, no sound.
  - **Error:** an ember chip icon, "**D:** is not ready" with Retry. The orb gets the ember badge, and the pill stays 8 s. It shows only for a scan the user started.
  - Pill text stays short so every pill fits within 300 px next to the orb.
- **States:**
  - Loading: the Scanning panel itself. Results fill in live; nothing waits for the end.
  - Scan stopped: "Scan stopped at 41%. Showing the 182 GB counted so far, so some folders look smaller than they are." with Scan again.
  - Access denied: "3 folders were skipped. Windows keeps them private, so about 0.9 GB is not counted." with Restart as admin. The scan always continues past them.
  - Drive not ready: "Plug the drive back in, or unlock it in Explorer, then scan again." with Try again.
  - No Recycle Bin (USB and network drives): "Files on this USB drive would be gone for good, so [App] only opens them in Explorer." Move to Recycle Bin is not offered on these drives.
  - File in use: "Win11_24H2_x64.iso is open in another app. The other file was moved." with Try again. A partial move reports what did move.
  - Nothing to clean: "Temp files and caches are already small. Big folders are still in Results."
- **Rules:**
  - Never delete automatically, never on a schedule, never permanently. The only remove action is Move to Recycle Bin, and it always confirms.
  - [App] never empties the Recycle Bin.
  - Labels say what removing something really costs (a slower first page load, or space that comes back only after emptying the bin). No "junk" and no scare numbers.
  - Cloud placeholder files (OneDrive) count at their logical size but are never opened, so a scan never downloads them. Reparse points are not followed.
  - No hue anywhere in the feature. Ember is only for errors. Sizes use `tnum`.
  - The disk log stays on this PC. Nothing about files is synced or shared with Family.
