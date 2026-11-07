## History (v6.2)
Doop frames "v6.2 · 23 History: drink and focus (white)" (id `Bm7GaZ8U3r`) and "v6.2 · 24 History: year view, empty (dark)" (id `s1JLm1o0lZ`), page "v6.2 · mercury + thinking orbs". It replaces the look of "v4 · 03 History graph + tabs" (old tokens, and it still had a Prayer tab).

History covers **drink and focus only**. Prayer is never in History: no tab, no hidden setting, no opt-in, no export. See "Prayer is not a task".

### Where it opens
- From the Focus panel "This week" strip: opens on the Focus tab.
- From the Drink panel "Last 7 days" bars: opens on the Drink tab.
- From the quick menu, More, History: opens on the last tab used.
- It opens in Month view. A user with under 30 days of data always lands in Month view.

### Panel
- Solid panel, radius 20, padding 16. Width 360 in Month view. Year view grows the same panel to 700 wide with the `grow` morph (360 ms); going back uses `shrink`.
- Header: "History" (14/18, 500) on the left, Liquid Tabs on the right.
- **Liquid Tabs (Drink, Focus):** a 32 px track, radius 16, `raised` fill with a hairline. Two 64 px tabs, 13/16 500; the inactive one is muted. The indicator is a 28 px pill inset 2 px: white theme `#FCFCFC` with `0 1px 2px rgba(0,0,0,.08)` plus hairline; dark `#2A2A2A` with an inset hairline and `inset 0 1px 0 rgba(255,255,255,.06)`. It slides on `settle` (250 ms). `role=tablist`; Left and Right move between tabs.
- **Range row:** a 28 px round previous button (`active` fill), the range in 13/16 500 ("5 Oct to 6 Nov", "Nov 2025 to Nov 2026"), and a next button that goes track-coloured with no fill when you are already at today. On the right, Month and Year as 28 px chips (12/28 500); the chosen one has the `active` fill.
- **Summary:** three equal columns between two hairlines, 12 px padding, hairline dividers. Number 26/30 500 at -0.02em with the unit at 13 px muted ("1840 ml", "6 h 40 min"). Below it the name (12/16, text colour) and the span (12/16, muted).
  - Drink: Average a day, Goal met (as a count of days), Best week (litres, "from 26 Oct").
  - Focus: This week (Mon to today), Average a day, Best week ("from 19 Oct").
  - Spans: Month view says "last 5 weeks". Year view says "since 5 Jan" (the install date) if that is later than a year ago.
- **Legend row:** muted 12/16. A plain reading on the left ("Darker is closer to your goal", "Darker is more focus time", "Outlined days are before you started"), and Less, five 10 px swatches (radius 3), then "Goal met" or "2 h or more" on the right.

### Contribution graph
One cell per day. Five neutral levels per theme, the same greys for both tabs:

| Level | Drink (share of goal) | Focus (time) | White | Dark |
|---|---|---|---|---|
| 0 | Nothing logged | Nothing logged | `#EEEEEE` | `#242424` |
| 1 | Under half | Under 30 min | `#D4D4D4` | `#3A3A3A` |
| 2 | Half or more | 30 min to 1 h | `#A3A3A3` | `#6B6B6B` |
| 3 | Three quarters or more | 1 h to 2 h | `#6E6E6E` | `#9E9E9E` |
| 4 | Goal met | 2 h or more | `#171717` | `#FAFAFA` |

- Date numbers on cells use text colour on levels 0 to 2 and the inverse on 3 and 4 (white theme `#FAFAFA` on 3 and 4; dark `#171717` on 3 and 4). All pairs pass AA.
- **Before you started:** no fill, a 1 px inset hairline, muted number. Never shown as level 0.
- **Still ahead (rest of this week):** no fill, the dotted time track (1.5 px dots, 6 px pitch, `passed` colour), muted number. Year view does not draw future days.
- **Today:** a 1.5 px ring in text colour, 2 px offset (1 px in Year view), number in 500. Today counts as it fills; it is never shown as short.
- **Keyboard focus:** a 2 px ring in text colour, 2 px offset (1.5 px in Year view), and the day detail opens. It is never hidden.
- Entrance: cells appear column by column on `ripple` when the tab opens. Static under reduced motion.

**Month view (360 panel):** the last 5 weeks, Monday first, ending this week. Weekday letters 12/16 muted. 7 columns of `minmax(0,1fr)` with 6 px gaps, square cells (about 42 px), radius 8, date number 12/16 centred.

**Year view (700 panel):** 53 week columns by 7 days. Cells 10 px, gap 2 px, radius 3, as a CSS grid with `grid-auto-flow: column`. Month labels (12/16 muted) sit above the column where each month starts. Day labels Mon, Wed, Fri on the left in a 28 px gutter.

**Focus by task (Focus tab, Month view):** a label row "By task, last 5 weeks" with the total on the right, then 36 px rows: a 24 px task chip (radius 7), the task name (truncates with an ellipsis), a 64 x 4 px bar (`active` track, ink fill, scaled to the largest task), and the time right-aligned in 76 px. Untagged time is the last row, "No task", with a timer chip and a muted name. Sessions come from Focus "Focus on a task".

### Day detail (hover or arrow keys)
- A popover: radius 12, padding 10 12, a 10 px caret toward the cell. White `#FCFCFC` with `0 8px 28px rgba(0,0,0,.10), 0 2px 6px rgba(0,0,0,.06)` plus hairline. Dark `#1F1F1F` with a hairline, `inset 0 1px 0 rgba(255,255,255,.05)` and `0 16px 40px rgba(0,0,0,.35)`.
- It sits above the cell; at the right edge and in Year view it sits to the left so it never covers the month labels or the legend.
- Lines: the date (13/16 500), the value with the rest muted, then one muted 12/16 line.
  - Drink: "Wed 4 Nov", "2150 ml · goal met", "8 drinks, 3 from your phone". A short day reads "1800 ml / 2000 ml". No "short by".
  - Today: "Today, so far", "750 ml / 2000 ml", "Ahead of pace for 11:00".
  - Focus, Month view: "Tue 3 Nov", "1 h 42 min · 4 sessions", then up to 3 task lines under a hairline.
  - Focus, Year view: one line only, "Most on Kirim invoice, 1 h 20 min", to keep the popover short.
  - Nothing logged: "Sun 18 Oct", "Nothing logged" in muted.
  - Before you started: "Before you started".
- Keys: Left and Right a day, Up and Down a week, Page Up and Page Down a month, Home goes to today, Esc closes the detail. The graph is `role=grid`; each cell's label reads the same text as the popover.

### States
- **No data yet:** the header and tabs, then "Nothing logged yet" (13/18 500) and "Log a drink and today fills in. A darker square is a day closer to your goal." A primary "+250 ml" button (the default glass). Focus says "Finish a focus session and today fills in." with "Start 25 min". The grid shows past days as outlines, today ringed at level 0, and the rest of the week dotted. No summary row.
- **First week (under 7 days):** one number only, the total since the first day ("3 h 46 min", "Since Wed 4 Nov"). Then "Averages and best week show once you have 7 days, so three days don't paint a false picture." Days before the first day are outlines.
- **Sync still loading:** this PC's days show at once. Days still coming from the phone are blank `raised` or `active` squares with no number (a number means real data). A sync icon with "Syncing with your phone", "This PC's log is shown now. Days from your phone fill in as they arrive.", a 4 px determinate bar and "18 of 35 days". The summary is two blank 12 px bars until it is done. No shimmer and no endless loop.
- **Sync failed:** a strip under the graph: a 32 px chip with the alert icon in ember (`#B3402F` / `#E2614F`), "Couldn't sync with your phone", "Showing this PC's log. Last synced today at 08:12. Nothing is lost; it retries every 15 min.", and a quiet "Retry" button. Ember is only on the icon.
- **A gap in the data** (holiday, mudik, away from the PC) shows as level 0, with no note or warning.

### Pill (optional, off by default)
- "Your week · 6 h 40 min focus" with a grid icon chip, **Open** (one 32 px quiet action) and close.
- Only if turned on in settings. Sunday 20:00, normal priority, held during focus, at most once a week, never while the orb is hidden.
- It never names prayer and never compares weeks ("less than last week" is banned).

### Rules
- No prayer data in History, the pill, sync payloads or exports. Not even as an off-by-default option.
- No guilt language: no "missed", "failed", "broke", "streak", "behind", red, or warning icons for a short day. A short day just has a lighter square.
- No streak counters. "Best week" is the only superlative, and it never says "beat it".
- Drink and focus history sync to your own devices only and are never shared with Family.
- Numbers are tabular (`tnum`). Weight 500 at most. Neutral greys only; ember is for sync errors.
