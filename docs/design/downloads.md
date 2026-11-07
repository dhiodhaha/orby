## Downloads (v6.2)
Doop frames "v6.2 · 11 Downloads: offer, queue, settings (white)" (`z1nvo7q4jO`) and "v6.2 · 12 Downloads: on the orb, errors (dark)" (`kxwL8uu62N`), page "v6.2 · mercury + thinking orbs". Titles, sizes, speeds and links in them are sample content.

The engine is a yt-dlp sidecar with ffmpeg for merging and audio conversion (ORB_SPEC §6, build step 7). The UI never says "yt-dlp" except in the settings footer.

### Layout
- Frame 11 (white): three columns at 412, 400 and the rest, 24 px gaps. "When you copy a link" (pill stage), "Downloads panel", "Settings".
- Frame 12 (dark): the same grid. Column 1 stacks two stages, "On the orb, while saving" and "Errors on the orb". Column 2 is the panel with problems, then the empty state. Column 3 is the pill's right-click menu, the first-run state and four rule notes.
- Stages follow the Focus and Drink frames: the orb sits in a 64 px wrapper on the right, a 1 px screen edge line, captions 12/16 muted and right aligned under each pill.

### The offer pill (clipboard link)
- **Trigger:** the clipboard listener (`AddClipboardFormatListener`) sees a URL from a site the engine supports and that site is not muted.
- **Text:** "Save this video?" plus the default format in muted, for example `1080p MP4`. Music links (YouTube Music, SoundCloud) read "Save this audio?" with `M4A`.
- **Chip:** 36 px, a film icon for video, a music note for audio.
- **One action:** Save (32 px quiet button). It saves with the default format. A close icon follows.
- **Priority:** quiet. No sound, gone after 6 s, never shown while the orb is hidden or in focus or full screen (the offer is simply dropped, not queued, since a stale offer is noise).
- **Video or audio, without a second button:**
  - The pill states the format it will use, so Save is never a surprise.
  - The format menu opens with a right-click on the pill, `↓` while the pill has keyboard focus, or a long press (450 ms) on Save.
  - The menu sits above the pill, aligned to its right edge with a 76 px inset (it clears the orb). Width 232, radius 12, padding 4, rows 32 px radius 8. Rows: Video 1080p MP4 (default, active fill and a check), Video 720p MP4, Audio only M4A (key `A`), Audio only MP3. A hairline, then "Change the default in Settings" in 12/16 muted.
  - Keys on the focused pill: `Enter` saves with the default, `A` saves audio, `Esc` closes.
  - The default comes from Settings ("Save copied links as" Video or Audio only, plus the video quality and audio format). Music sites always start as audio.
- **Copied it twice:** if that link is already saved, the pill reads "Already saved" with Open instead of Save.
- **Privacy note under the stage:** "The clipboard is checked for media links from your sites. Nothing is fetched until you press Save." The listener reads the clipboard text only, and only to match a URL.

### Downloads panel
- Panel 400 wide, radius 20, padding 16. Title "Downloads" (14/18, 500) with a muted count on the right, for example "2 running · 1 waiting".
- **Paste field:** 40 px, radius 20, link icon, placeholder "Paste a video or audio link", a 28 px Paste button.
- **Sections:** "Now" (with "2 at a time" on the right), then "Done today" (with "Clear list").
- **Rows:** 48 px minimum (two lines), radius 8, padding 6 by 8. Title 13/18 500 with ellipsis, a 12/16 muted second line.
- **Status chip (28 px) on the left of each row:**
  - Reading the link: a quarter arc that travels (no size yet). Line: "youtu.be/q7Lm3kV · finding formats".
  - Running: progress ring, r 12, 1.5 px hairline track (`rgba(0,0,0,.1)` white, `rgba(255,255,255,.12)` dark), 2 px ink arc from 12 o'clock. A 12 px film or music icon in the centre. Line: "190 of 412 MB · 4.2 MB/s · 53 s left".
  - Waiting: a dotted ring (still to come, the identity motif). Line: "Waiting · 720p MP4 · starts next". The row has a 28 px format chip ("720p" with a chevron), so the format can still change before it starts.
  - Paused: the arc in `passed` grey (`#A3A3A3` white, `#6B6B6B` dark) with a pause glyph. Line: "Paused · 120 of 300 MB".
  - Done: a check in a 28 px active chip. Line: "1080p MP4 · 412 MB · 10:42".
  - Failed: an ember chip (`rgba(226,97,79,.14)` fill, ember icon). Disk full keeps its progress ring, with the arc in ember.
- **Row buttons:** 28 px round. Running: Pause, Cancel. Paused: Resume, Cancel. Waiting: format chip, Cancel. Done: Open (28 px text button) and Show in folder (folder icon). They show in full on hover and keyboard focus and stay as quiet icons otherwise. Pause keeps the part file. Cancel deletes it.
- **Footer:** hairline, folder icon, the save folder ("Downloads\[App]"), and free space on the right ("38.2 GB free on C:").
- Keyboard: visible 2 px focus ring (ink, 2 px offset), shown on Open in frame 11 and on Change folder in frame 12.

### Settings
Panel "Download settings", muted "this PC" (settings do not sync).
- Save to: the folder path with a Change button.
- Video quality: chips Best, **1080p**, 720p, 480p. "Takes the next lower one if missing".
- Audio format: chips **M4A**, MP3, Opus. "M4A keeps the quality. MP3 plays anywhere."
- Offer when I copy a link: switch, on by default.
- Save copied links as: chips **Video**, Audio only. "Music sites always start as audio".
- Offer for these sites (per-site mute): one 36 px row per site with a 24 px globe chip and a switch. A muted site shows "muted" in grey. "Muted sites still save from the panel".
- Footer: "Engine yt-dlp 2026.10.22", "updates by itself".

### On the orb
- **While saving:** the v6.2 progress ring (svg 64, r 30, 1.5 px track, 2 px arc). It shows the whole queue by size, not one file. Hover shows a pill with no action: "Downloading" plus `2 of 3 · 46%`.
- **Reading a link:** the short travelling arc (a quarter of the ring), because there is no size yet. Reduced motion holds it still.
- **Done:** the ring closes, one ripple, the done tone. Pill "Saved" plus the title in muted (truncated), action Open, close. Normal priority. `F` or the right-click menu shows it in the folder.
- **Several done:** same-source notices merge: "3 downloads done" plus `1.2 GB`, action Show (opens the panel).
- **Focus or full screen:** done pills wait. The ink-white badge counts them.
- **Right-click menu on any download pill:** Show in folder (`F`), Copy link, Open downloads, a hairline, "Don't offer for YouTube" (the per-site mute for the current site), "Don't offer on copy". Muting confirms with a quiet pill that carries Undo.

### Errors and states
Errors use the ember badge on the orb and an ember icon chip in the pill. They stay 8 s. Each one names the cause and the next step.

| Case | Pill | Action | Panel row |
|---|---|---|---|
| Private video | "Can't save" + `video is private` | none (nothing to retry) | "Private. Only the owner can share it." Remove |
| Removed video | "Can't save" + `video was removed` | none | "Removed by the uploader" Remove |
| No space left | "Disk full" + `D:` | Change folder | ring held in ember, "Disk full on D: · 250 of 412 MB", Change folder. Paused, not lost: it goes on in the new folder. |
| Link unreadable | "Couldn't read this link" | Retry | Retry. The engine has already updated itself once before this shows. |
| Offline | "Offline" + `2 downloads paused` | none | a banner "You're offline", rows "Waiting for internet · 34%". Not an error: no ember, the orb ring holds still in grey, and downloads go on by themselves. |
| Unsupported site | never a pill | | only in the paste field: ember outline and "This site isn't supported. Try a link from YouTube, Instagram, TikTok or X." |

- Unsupported sites can only happen in the paste field, because the clipboard listener only reacts to links the engine knows.
- **Empty:** a 36 px download chip, "No downloads yet", "Copy a video link anywhere and the orb offers to save it. Or paste one here.", a primary "Paste link" button and "Ctrl V".
- **First run (loading):** "Getting downloads ready", "once only", a progress ring with "Fetching yt-dlp and ffmpeg", "14 of 38 MB · your link waits, then starts".

### Rules
- One action per pill. The format choice lives in the pill's menu, the queue row's format chip and Settings, never as a second button.
- The ring is progress (hairline track). Dotted means waiting. Grey means paused. Ember means it needs you.
- Offline is not an error. Never ember for it.
- The offer and offline pills make no sound. Done plays the done tone. Errors never play an alarm.
- Nothing is fetched until the user presses Save or pastes a link.
- Site names appear as plain text. No site logos.
