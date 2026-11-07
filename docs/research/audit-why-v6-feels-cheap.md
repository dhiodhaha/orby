# Audit: why v6 feels cheaper than the references

Date: 2026-11-06.

**Method:**
- I rendered libraries.dev/metal and libraries.dev/gooey at 1440 px in Chromium and dumped the computed typography, control sizes, padding, radii, fills and colours (`render-*/desktop.json`).
- I compared them with the CSS of our v6 frames.
- I cropped Gooey's "Gooey Project 45 files" pill (the same component as our notification pill) next to our download pill, both at 2×.
- I checked shujaat.info and orenji.studio dumps as secondary references.

**Libraries.dev is the closest reference** to the look you want (dark, neutral, metal), so it is the main yardstick.

## Findings, ranked by impact

| # | Area | Ours (v6) | Reference (measured) | Why it reads cheap | Fix |
|---|---|---|---|---|---|
| 1 | **Typeface** | Atkinson Hyperlegible Next, plus Atkinson Mono for every time and number | **Inter** everywhere in the UI. Display text uses Saans 36/500. Shujaat uses Suisse 14/450. | Atkinson is drawn for low vision: wide, quirky shapes and slashed zeros. Its mono makes "11:26" look like a terminal. Every reference uses a quiet grotesk. | **Inter** (optical sizes), with `font-feature-settings: "tnum"` for numbers. No monospace anywhere. |
| 2 | **Weight** | Titles 600 to 700, pill titles 600 | **500 at most** for UI (Libraries 500, Shujaat 450 body). 600 only for 28 px or larger. | Heavy weights at small sizes look loud. Premium UIs build hierarchy with colour and opacity, not bold. | Titles and labels 500, body 400, big numbers 500. |
| 3 | **Text size vs container** | 14.5 px title in a 44 px pill; 13.5 px body; 15 px panel title | **13 px** for almost everything (body, buttons, nav), 12 px secondary, 11 px tiny, inside 32/36/40/48 px controls | Bigger text in a smaller box looks crowded. The reference pill puts 13 px text in a 48 px pill with lots of air. | One 13 px base, 12 px secondary, 14 px panel titles. Pill 48 px tall. |
| 4 | **Too many things in a pill** | icon + title + value + a bordered button + close | icon chip + title + value. Nothing else. | Every extra bordered control adds edges, and the pill turns into a toolbar. | At most **one** action, as a quiet text button (fill `rgba(255,255,255,.06)`). Close is a small icon at the end. The icon sits in a 36 px round chip. |
| 5 | **Grey hue** | Zinc (`#18181B`, `#27272A`, `#A1A1AA`), which has a blue-violet cast | **True neutral** greys (`#171717`, `#202020`) and **white at low alpha** for surfaces (`.04 / .06 / .07 / .08`) and text (`rgba(202,202,202,.7)`) | Blue-tinted grey next to the silver metal reads cool and cheap, like a Tailwind template. Alpha layers keep every level in one family. | Neutral palette. Dark surfaces are white alpha over `#171717`. |
| 6 | **Radius language** | 10 px buttons, 8 px chips, 20 px panels | Inline controls **fully round** (50 px). Menu rows 8, cards 12, panels and containers 16 / 20 / 36 | A 10 px button is neither soft nor sharp. The reference uses a clear pill for every tappable control and keeps rectangles for content. | Buttons and chips: pill. Rows: 8. Cards: 12. Panels: 20. |
| 7 | **Shadows in dark** | `--float`: 42 px plus 6 px black shadows on every floating surface | Dark surfaces are almost flat: a hairline plus a lighter fill. Big shadows only on things that really float (menus). | Big black shadows on near-black just look muddy. | Dark: hairline plus `inset 0 1px 0 rgba(255,255,255,.05)`. Keep the soft 42 px shadow for the light theme and for the orb/pill over other apps. |
| 8 | **Icons** | 16 px line icons floating directly in rows | Icons sit in **small chips** (24 px rounded square in the sidebar, 36 px circle in the pill) with their own fill and hairline | A chip gives each icon a "seat". Loose icons look unfinished. | Pill icons in a 36 px circle chip. Menu and list icons in a 24 px chip, radius 7. |
| 9 | **The dotted orb inside the pill** | A 20 px thinking orb as the pill icon | The reference shows a calm glyph in a chip | At 20 px the orbiting dots look like noise next to text. | Pill: a progress ring in the chip for downloads. Keep the thinking orb for the collapsed orb, where it is the main subject. |
| 10 | **Line height** | 15 to 18 px on 12 to 15 px text, plus extra paddings | Controls use line-height equal to the font size (13/13, 13/14, 13/16); body 13/18.2 | Loose line heights inside controls push them taller and less precise. | Control text 13/16, body 13/18. |
| 11 | **Letter spacing** | −0.02em on hero numbers only | −0.15 px at 15 px, −0.18 px at 36 px (Libraries); −0.56 px at 28 px and −1.92 px at 48 px (Shujaat) | Big numbers without tight tracking look loose. | −0.01em from 15 px, −0.02em from 24 px. |

## What is already fine
- Surface depth idea, the hairline rings and lip (refined by finding 7).
- The metal, now that it uses the real shader.
- The panel layout and information.
- Contrast. It stays checked: muted `#9E9E9E` on `#171717` is 6.7:1, and `#6E6E6E` on `#FCFCFC` is 5.0:1 (4.7:1 on `#F5F5F5`).

## v6.1 tokens, derived from the above
- **Font:** Inter 13/16 controls, 13/18 body, 12/16 secondary, 14/18 panel titles at 500, 24 to 26 numbers at 500 with −0.02em and `tnum`.
- **Dark:**
  - desk `#0F0F0F`, panel `#171717`
  - raised `rgba(255,255,255,.06)`, active `rgba(255,255,255,.08)`
  - hairline `rgba(255,255,255,.06)`
  - text `#FAFAFA`, muted `#9E9E9E`
- **White:**
  - desk `#F5F5F5`, panel `#FCFCFC` (sidebar `#F9F9F9` from your screenshot)
  - raised `rgba(0,0,0,.04)`, hairline `rgba(0,0,0,.07)`
  - text `#171717`, muted `#6E6E6E`
- **Controls:** pill buttons 32 px, padding 0 12, 13/500. Chips 28 px. Rows 36 px, radius 8. The pill is 48 px with a 36 px icon chip.
