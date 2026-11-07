## Meeting notes (v6.2)

> **Postponed (2026-11-06).** The user postponed meeting notes until they can test Groq on real speech. The design below is kept for later. If the real-speech test fails, the feature is dropped.
Doop frames "v6.2 · 15 Meeting notes: detect, record, notes (white)" (`byeHYBlvyN`) and "v6.2 · 16 Meeting notes: on the orb, summary, privacy (dark)" (`9d50xbbKS4`), page "v6.2 · mercury + thinking orbs". The meeting, its people, notes, transcript and tasks in them are sample content.

Based on the Granola research (`~/granolarea/research/01` §3, `05` §5.3 and §6) and the `minutes` architecture (local speech to text, audio not kept by default). Behaviour and numbers only, no copied code or text.

- **Layout:** both frames are 1280 wide, padding 40 48, three columns: 424 px, 392 px and `minmax(0,1fr)`, gap 24.
  - Frame 15: "When a meeting starts" (stage), "Live notes" (panel), "After Stop" (panel plus state cards).
  - Frame 16: "On the orb" (stage), "After the meeting" (summary panel), "Privacy" (panel, meeting card, delete dialog).
- **Detection:**
  - A meeting app starts a mic session (`IAudioSessionManager2`, allowlist: Zoom, Teams, Chrome, Edge, Firefox, WhatsApp, Discord, Slack). The pill appears 2 s later.
  - A calendar event in progress, or starting within 90 s (and started no more than 15 min ago), names the meeting. It replaces the app pill.
  - Cooldown 2 min per app. Right-click the pill: "Never for Zoom".
- **Live notes panel** (392 wide, radius 20, padding 16):
  - Header: meeting name 14/500. On the right: an 8 px ember dot, "Recording 13 min" (12, muted) and the 3-bar level meter.
  - Meta line, 12 muted: "Zoom · You, Dimas, Sari · started 13:31".
  - Source chips, 28 px round: "Your mic · Headset" and "Meeting audio · Zoom". Clicking one mutes it. Mute writes silence, so the timeline stays whole.
  - "Your notes": a white editor card, radius 12, with bullets. Keyboard focus is a 2 px ink outline at 2 px offset.
  - "Live transcript": a raised well, radius 12. Each line is a 52 px column (speaker in 500, time in muted) and the text. The newest line is still being heard, so it is muted with no time ("now"). Mic is "You"; meeting audio is the other people, named from the call when the app allows it.
  - Buttons: Pause (quiet) and "Stop and write notes" (primary, fills the row).
  - Fine print: "Your lines stay yours. The summary is added beside them, never over them."
- **After Stop (processing):**
  - The same panel shows three steps, each with a 24 px chip (radius 7): Transcript ready (check), Writing the summary (a 20 px ring with a 2 px arc, the task ring motif), Decisions and action items (a dotted ring, still ahead).
  - Three 8 px skeleton bars stand in for the summary.
  - "You can close this. The orb keeps a ring until it's done."
- **Summary panel** (392 wide):
  - Header: name, "Fri 6 Nov · 42 min", meta with app and people.
  - Tabs (Liquid Tabs, 28 px): Summary, Your notes, Transcript.
  - A two-line overview in body text.
  - Decisions: 32 px rows with a 5 px ink bullet. A 22 px time chip on the right opens the transcript at that moment.
  - Action items: 44 px rows, radius 8. A 20 px round pick mark (filled ink when picked, hairline ring when not), the title, a muted line with the destination list and due date, and a 20 px avatar.
    - Work items go to Work lists (private). Family items go to a Family list and can be assigned to You, Ibu, Ayah or Raka. Only members of the chosen space can be picked.
    - The focused row shows a 2 px outline and a row of 28 px person chips. The picked person is the raised chip with an ink avatar.
    - Items said by someone who is not in your spaces start unpicked, with a dashed avatar.
  - Buttons: "Add 3 to tasks" (primary, counts the picked rows) and Copy notes.
  - The tasks are made as normal tasks. A Family task reaches that person's orb as the "Assigned to you" pill (see Tasks).
- **Privacy panel:**
  - Three facts, each with a 24 px chip: "Speech to text by Groq" (only speech is sent, silence is cut, nothing is kept by us; offline or over the free cap, the audio waits on this PC and is sent when possible), "Saved here, encrypted" (notes and transcripts, not synced, never shared with Family), "Notes by Groq" (qwen3.8-27b gets the transcript text and your notes, never audio; written every 10 min while the meeting runs).
  - Switches (32×20): Keep the audio (off; on keeps Opus files 30 days), Consent reminder (on), Ask when an app takes the mic (on, with the app list).
  - "This meeting" card: size of what is stored ("Transcript 41 KB. No audio kept.") and Delete meeting. When audio was kept, a second button "Delete audio" removes only the audio.
  - Delete confirm: "Delete Sprint review?" Notes, transcript and summary leave this PC for good. Tasks you added stay. Cancel and Delete (primary ink, not ember: deleting is not an error).
- **Pills:**

  | Pill | Text | Priority | Action |
  |---|---|---|---|
  | App took the mic | mic chip, "Zoom took the mic" | urgent | Take notes, plus close (not now) |
  | Calendar start | calendar chip, "Sprint review" + muted "13:30" | urgent | Take notes, plus close |
  | Consent | message chip, "Tell them you're recording" | normal, at least 5 s | Copy line: "I'm taking notes with [App] on this call." |
  | Recording (on hover) | level chip, "Recording" + "13 min · Sprint review" | quiet | Stop |
  | Paused | pause chip, "Paused" + "13 min" | quiet | Resume |
  | No sound | flat ember line chip, "No sound" + "for 30 s" | error | Check mic |
  | Mic blocked | ember mic-off chip, "Mic blocked" + "by Windows" | error | Open settings (`ms-settings:privacy-microphone`) |
  | App let go | mic chip, "Zoom let go of the mic" | normal | Stop notes |
  | Writing notes | file chip, "Writing notes" + "Sprint review" | quiet | none |
  | Notes ready | check chip, "Notes ready" + "4 action items" | normal | Open |

  - Detection pills stay 20 s while not hovered (Granola's value; other pills use 6 s). Hover pauses the clock.
  - Detection pills show while the orb is hidden and during focus, like other urgent notices. They make no sound while a call is live.
  - Error pills stay 8 s, then wait in the badge.
- **On the orb:**
  - Recording: an ember dot, 10 px, top right of the drop, with a 2 px ring in the stage colour. No progress ring.
  - Paused: the same dot in muted grey (`#9E9E9E` dark, `#6E6E6E` white).
  - Writing notes: the task progress ring (r 30, 1.5 px track, 2 px arc).
  - Notes ready: the ring closes with one ripple and the done tone.
  - Mic blocked: the ember error badge (18 px, `#E2614F` with `#171717` text dark, `#B3402F` with `#FAFAFA` text white).
- **Level meter:** three 3 px bars, 2 px apart, centred in the chip and mirrored around the middle. The level is the louder of mic and meeting audio, sent every 100 ms and smoothed with a fast-decay peak follower. Bar heights run 3 to 14 px.
- **Stop:**
  - When the meeting app lets go of the mic and the calendar event ends within 5 min, notes stop by themselves and the Writing notes pill shows.
  - Otherwise the "Zoom let go of the mic" pill asks. With no answer, recording stops after 10 min of silence on both sources.
  - Recordings shorter than 3 min never stop by themselves.
- **States:**
  - Mic blocked by Windows privacy settings: nothing records. The pill and the badge name the setting and open it.
  - No audio detected: bars go ember after 3 s of zero level on both sources. The pill comes at 30 s. Recording keeps going.
  - Processing failed: an error card, ember icon, "Couldn't write the summary". "The AI service didn't answer in 60 s. Your notes and the full transcript are saved on this PC." Try again.
  - Empty: "No meeting notes yet", how detection works, and Start notes now.
  - Loading: the processing steps and skeleton bars above.
- **Rules:**
  - Ember is only for recording and errors: the dot, the no-sound line, the mic-blocked chip and badge, the failed card icon.
  - Interface sounds and drink reminders wait while a meeting is recorded. Prayer pills still show; the adzan is silent in meetings.
  - Meeting notes are personal. They never sync to a Family space. Only tasks you choose to add are shared.
  - Speakers come from the source (mic is You). Names from the call's window are a later feature flag.
  - UI text is English. Notes and transcripts are whatever language people speak.

## Notes pipeline, modelled on Granola (plan for when this feature is built)
Source: `~/granolarea/research/03-notes-and-ai-generation.md`. We copy behaviour and structure only. Granola's prompt text lives on its server, so it was never in the REA, and we write our own.

- **Two documents per meeting.**
  - *Your notes* is what you type during the meeting; it always stays yours.
  - *Generated notes* is a separate panel the AI writes.
  - Every AI bullet carries `origin: ai`. Once you edit it, it becomes `origin: human`, and the AI never overwrites it again (Granola's "bullet ownership").
- **Inputs gathered on the PC:**
  - the transcript with speakers (Me / Them / names from the meeting app)
  - your typed notes and their headings (these steer the sections)
  - attendees and companies from the calendar event
  - the meeting title, date and time
  - your name
  - a dictionary of names
  - the template's sections
- **Templates:**
  - A template is a title plus sections, each with a heading and an instruction (for example "Keputusan: what was decided, by whom").
  - The number of headings scales with length: 4 under 8k characters, 5 under 20k, then one more per 10k, up to 8.
  - Built-in templates: Team sync (Indonesian), Client call (English), 1:1.
  - The language follows the meeting, and the template headings are in that language.
- **Rolling generation** (our change, for Groq's 8k tokens per minute):
  - Every 10 min of transcript is condensed into section notes while the meeting runs.
  - At Stop, one merge turns them into the final notes, ready in seconds.
- **Action items are a separate step:** `{intent, detail, owner, spoken deadline, transcript time}`.
  - The deadline stays as spoken ("sebelum Rabu"), and `crates/core` resolves the date (the Groq test showed models invent dates).
  - "Add to tasks" sends them to the right list and person.
- **Meeting end:** a tiny check reads the last ~100 words for a goodbye ("oke makasih ya", "thanks everyone"), together with the meeting app releasing the mic, then shows "Meeting ended · notes ready".
- **View source:** each generated line keeps its transcript timestamp. Clicking it opens the transcript at that moment (by time, not inline citation marks).
- **Rewrite:** "Make it shorter", "In English" or your own instruction re-runs the merge with the current notes as input. Your own (human) bullets are kept.
- **Later:**
  - chat over one meeting or across meetings ("apa yang Dimas janjikan minggu ini?")
  - reusable prompts like Granola's recipes
  - "what did I just miss" during the meeting, which is cheap because of rolling generation
- **Models:** Groq only, no local models. Speech uses `whisper-large-v3-turbo`; notes, action items and the end check use `qwen/qwen3.8-27b`.
- **Still gated** on the user's real-speech test (see the note at the top).
