# Third-party notices

Orby is licensed under Apache-2.0 (see `LICENSE`). It uses or learns from the work below.
Where a licence requires its text to travel with the code, it is reproduced here.

## Paper Shaders (`liquidMetal`)
Source: `@paper-design/shaders` 0.0.80 (https://github.com/paper-design/shaders), Apache License 2.0, copyright 2026 Paper ("Powered by Paper Shaders: https://shaders.paper.design").
- **Used for:** the liquid-metal drop. The fragment shader is ported to HLSL in `apps/orb-proto/src/metal.hlsl`, keeping only the circle shape path and adding a rim ring pass.
- **Parameters:** follow the silver preset of metal-fx (MIT, Jakub Antalik; see the MIT text below).
- **Changes:** ported from GLSL to HLSL, reduced to the circle shape, premultiplied-alpha output, and a brightness control.
- The full Apache-2.0 text is in `LICENSE`, and the attribution is in `NOTICE`.

## Libraries.dev
Source: https://github.com/Jakubantalik/Libraries.dev, at commit `9a42e88847dab0ac145733f15cf90535e9cb59ab` (2026-11-05), MIT, copyright (c) 2026 Jakub Antalik.
- **Used for:** the spring presets (`liquid-gooey`: bouncy 320/17, smooth 190/26, snappy 480/34) and the idea of a pill that buds out of the drop. Both are reimplemented natively in Rust.
- **Not used:** the Thinking Orbs engine was studied and then deliberately left out of the app.

## Ovio
Source: https://github.com/JanaSundar/ovio, at commit `81a1bcb6737356038163728b3b2a657c9dcffa32` (2026-10-24), MIT, copyright (c) 2026 Jana Sundar.
- **Used for (behaviour and numbers, reimplemented natively):** the dial-menu geometry and aiming, the knob's detents and end-stop resistance, the rolling-number spring timing, and the contribution-graph grid and year layout.
- Visual styling is Orby's own (see `DESIGN.md`).

MIT licence text for Libraries.dev and Ovio (the copyright lines are as above):

```
MIT License

Copyright (c) 2026 Jakub Antalik / Jana Sundar (see the sections above)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Inspiration only (no code, prompts or assets copied)
- **Granola**: the floating "nub" orb, docking behaviour, and how meeting notes are structured. Studied from the outside.
- **Poni**: a native overlay with prayer times and a downloader, used as a memory and CPU reference.
- **TreeSize**: the idea behind the disk scanner.
- **Kemenag RI / Bimas Islam**: the published prayer schedule, used only to check the calculation.

## Used at run time, not bundled
- **yt-dlp** (Unlicense, https://github.com/yt-dlp/yt-dlp): downloaded by the user's own click, then driven as a separate program.

## Rust and npm dependencies
Cargo and npm dependencies keep their own licences (mainly MIT or Apache-2.0): see `Cargo.lock` and `apps/server/package-lock.json`. Notable ones: `windows` (Microsoft), `windows-numerics`, `serde`, `serde_json`, `rusqlite` with bundled SQLite (public domain), and for the server `wrangler`, `typescript` and `vitest`.

## Build process
Built with Claude Code (Anthropic) and the pstack agent skills (https://github.com/backnotprop/pstack, MIT).
