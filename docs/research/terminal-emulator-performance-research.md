# Terminal Emulator Performance: Primary-Source Research Report

**Scope:** Ghostty, Kitty, WezTerm, Alacritty, iTerm2, Warp, Windows Terminal, Zed (built-in terminal).
**Dimensions:** (1) input latency, (2) rendering/IO throughput + backpressure, (3) memory & scrollback, (4) startup time, (5) resize/reflow, (6) grid-rendering correctness (emoji/CJK/wide chars/ligatures/long lines).
**Method:** evidence from official docs/blogs, maintainer statements (GitHub issues/comments, Mastodon, HN), and reproducible benchmark repos. Secondary sources were used only as leads. No numbers are invented; every figure carries its source. Research performed 2026 (primary sources dated up to 2026-08).
**No cross-terminal recommendations or rankings are made** beyond what a cited source itself states.

---

## Dimension 1 — Input latency (keypress → glyph)

**Kitty — the only terminal with official latency claims.**
- [kitty performance docs](https://sw.kovidgoyal.net/kitty/performance.html) (current; benchmark table built for kitty 0.33, 2023). States kitty's goals are "user perceived latency while typing" and "smoothness"; rendering runs in a separate thread from child-program interaction; SIMD parsing. Documents deliberate latency knobs: `input_delay` **default 3 ms** (confirmed in [conf docs](https://sw.kovidgoyal.net/kitty/conf/): "Delay before input from the program running in the terminal is processed... decreasing it will increase responsiveness, but also increase CPU usage"), `repaint_delay`, `sync_to_monitor`. **Proves:** kitty deliberately introduces a default few-ms input delay to save energy — any comparison must control for this.
- [kitty issue #2701 comment by Kovid Goyal (2020-05-31)](https://github.com/kovidgoyal/kitty/issues/2701#issuecomment-636497270): cites hardware measurements "showing kitty's latency is optimal, and much better than alacritty's (35ms vs 50ms)" and typometer figures "kitty at 7ms vs 30ms for alacritty". **Proves:** maintainer's own summary of third-party measurements.
- [kitty index](https://sw.kovidgoyal.net/kitty/): "Uses threaded rendering for absolutely minimal latency" (links to issue #2701).

**Independent hardware measurement (reproducible benchmark):**
- [Tristan Hume, "Making a Latency Tester" (2020-05-20)](https://thume.ca/2020/05/20/making-a-latency-tester/): Teensy + light sensor measuring keypress→screen-change. Results: **kitty 36.1 ms** (median 35), **Apple Terminal 35.8 ms** (median 34), **alacritty 50.4 ms** (median 50), **iTerm2 no-GPU 50.5 ms / GPU renderer 53.1 ms**. Author's words: "the default Apple Terminal and kitty have similar approximately optimal latency, while iTerm2 and Alacritty have worse latency." **Proves:** on this 2020 macOS setup, kitty/Terminal.app beat alacritty and iTerm2 by ~15 ms; the iTerm2 GPU renderer of that era did NOT reduce measured latency (slightly worse).
- [Typometer](https://pavelfatin.com/typometer/) (tool) — the software-latency tool used in kitty's docs.

**Ghostty — maintainer statements, no official numbers.**
- [Mitchell Hashimoto, HN Ghostty 1.0 thread (2024-12-27)](https://news.ycombinator.com/item?id=42518612): Ghostty does vsync by default, supports variable refresh rates (DisplayLink), prefers the integrated GPU, parks non-focused rendering threads on E-cores, and "slows down rendering significantly if the window is obscured completely." **Proves:** design intent for latency/energy, not measured latency.
- [Ghostty "About" docs](https://ghostty.org/docs/about): positions Ghostty as fast/feature-rich/native but explicitly "not trying to claim that Ghostty is the best (i.e. the fastest...)".
- No official Ghostty input-latency measurement exists in primary sources. The "Ghostty is crazy slow" reports are user discussions (e.g. [ghostty discussion #5113](https://github.com/ghostty-org/ghostty/discussions/5113)), and Mitchell's response in [HN (2026-06-08)](https://news.ycombinator.com/item?id=48448945) says "Everything we've measured so far firmly places Ghostty in the 'fast' camp (with friends such as Kitty). We're sometimes faster, sometimes slower, but in any case not noticeably so."

**WezTerm / Warp / Windows Terminal / Zed — no official latency numbers.** WezTerm's only latency-adjacent primary note is the mux "Improved latency for large window sizes" changelog entry ([WezTerm changelog](https://wezterm.org/changelog.html), 2022 release, PR #1872). Windows Terminal's only official rendering-latency claims concern the renderer (see Dimension 2). iTerm2's Metal wiki claims "most users will see improved latency" ([Metal Renderer wiki](https://gitlab.com/gnachman/iterm2/-/wikis/Metal-Renderer.md)) — contradicted by Hume's 2020 measurement (see Contradictions).

---

## Dimension 2 — Rendering/IO throughput & backpressure

**Ghostty — official measured numbers (maintainer).**
- [Mitchell Hashimoto, Mastodon (2026-07-06)](https://hachyderm.io/@mitchellh/116873952162192565): "Ghostty is now indisputably the fastest terminal emulator at IO throughput, by a very large margin. On ASCII, Unicode, and CSI tests, Ghostty is more than 2x (double!) faster than any other leading 'fast' terminal." Numbers (`time cat 150MB_ascii.txt`): **Ghostty nightly 575 ms, Ghostty 1.3.2 1.5 s, Alacritty 1.2 s, Kitty 1.7 s, Warp 3.8 s, iTerm2 & Terminal.app >60 s (test aborted)**; Unicode file: 536 ms / 1.22 s / 1.05 s / 1.35 s / 3.4 s. DOOM-Fire-Zig (IO test): Ghostty nightly 842 fps vs Alacritty 593, Warp 577, kitty 485, iTerm2/Terminal 60. **Proves:** maintainer-measured end-to-end throughput, 2026 nightly libghostty builds; includes the full PTY pipeline (rendering not suppressed).
- [Mitchell, follow-up (2026-07-06)](https://hachyderm.io/@mitchellh/116873980514808799): "This is just an IO throughput test. No renderers involved, just a question of how many bytes/second you can push through the terminal emulator." **Proves:** these figures do NOT measure input latency or frame rate.
- [Mitchell, IO gather thread (2026-07-06)](https://hachyderm.io/@mitchellh/116874083461527212): documents **backpressure mechanics** — macOS/Linux PTY `read()` returns max 1024 bytes; when the kernel-side pty buffer fills, the writer (TUI program) blocks. Ghostty added a dedicated IO gather thread that spin-loops on full reads. **Proves:** concrete backpressure design; pty-buffer-full stalls are the actual throughput bottleneck.
- [Mitchell, Linux kernel regression (2025-06-29)](https://hachyderm.io/@mitchellh/114767017754076838): kernel 6.15.4's io_uring change "destroys Ghostty's performance"; workaround epoll. **Proves:** Ghostty's IO path (io_uring by default) is performance-sensitive to kernel changes.
- [Ghostty Devlog 006 (2024-02-12)](https://mitchellh.com/writing/ghostty-devlog-006): IO throughput work incl. SIMD plain-text parsing (PR #1472). **Proves:** throughput was an explicit design focus from 2024.

**Alacritty — official benchmarks (maintainer).**
- [Joe Wilm, "Announcing Alacritty" (2017-01-06)](https://jwilm.io/blog/announcing-alacritty/) — launch post with early benchmark claims.
- [Joe Wilm, "Alacritty Lands Scrollback, Publishes Benchmarks" (2018-09-17)](https://jwilm.io/blog/alacritty-lands-scrollback/): introduces **vtebench** ([repo](https://github.com/alacritty/vtebench)); on 2015 MBP/i7: Alacritty v0.2 "over 9x faster than the next terminal emulator on macOS for scrolling" (vs iTerm2/Kitty/Terminal.app); on Linux "about 2.5% faster than the next" (vs kitty/urxvt); all at "buttery smooth 60fps". **Proves:** Alacritty's own reproducible vtebench throughput numbers (2018; note era and hardware).
- [Alacritty README FAQ (current)](https://github.com/alacritty/alacritty): "Is it really the fastest terminal emulator? ... Alacritty uses vtebench to quantify terminal emulator throughput and manages to consistently score better than the competition using it." **Proves:** current official throughput claim, scoped to vtebench.

**Kitty — official benchmark (maintainer), parser-only.**
- [kitty performance docs, Benchmarks](https://sw.kovidgoyal.net/kitty/performance.html) (kitty 0.33, 2023): built-in `kitten __benchmark__`; **rendering suppressed** by default to isolate parser speed; Ryzen 7 PRO 5850U, Linux/X11. Table (ASCII/Unicode/CSI/Images/Average, MB/s): **kitty 121.8/105.0/59.8/251.6/134.55; gnome-terminal 61.83; alacritty 43.1/46.5/32.5/94.1/54.05; wezterm 16.4/26.0/11.1/140.5/48.5; xterm 30.72; konsole 27.48; alacritty+tmux 24.73**. "kitty is twice as fast as the next best." Notes that with rendering enabled the numbers are "not really useful for comparison, as it is just a game about how much input to batch before rendering the next frame" (async rendering). **Proves:** parser-throughput ranking under kitty's own test; also documents why renderer-inclusive comparisons are confounded. (WezTerm appears here — 48.5 MB/s avg — one of the few primary throughput data points for WezTerm at all.)
- [kitty FAQ: memory perception (current)](https://sw.kovidgoyal.net/kitty/faq.html) — not throughput; listed for completeness.

**Windows Terminal — official renderer/throughput statements.**
- [AtlasEngine PR #11623 by lhecker (2021-10-27, merged 2021-11-13)](https://github.com/microsoft/terminal/pull/11623): new renderer; DirectWrite/Direct2D only rasterize glyphs; D3D + HLSL shader blits; "Runs at up to native display refresh rate"; glyph atlas is grow-only with a documented 256 MB (~20k glyphs) failure point. **Proves:** architecture + throughput intent + an early memory caveat.
- [Windows Terminal Preview 1.13 release notes (2022-02-12)](https://devblogs.microsoft.com/commandline/windows-terminal-preview-1-13-release/): `experimental.useAtlasEngine` opt-in; "While the performance improvements aren't generally noticeable, they can be seen in certain edge cases, most notably when presenting text with a large number of colors. In these cases, this new renderer will draw at the display refresh rate regardless of screen resolution." **Proves:** the official, measured-sounding statement that the new renderer's gains were NOT generally perceptible at introduction.
- [Pseudoconsoles (ConPTY) docs](https://learn.microsoft.com/en-us/windows/console/pseudoconsoles): the host application "must become responsible for displaying the graphical output and collecting user input" — the architectural basis for Windows Terminal's extra relay layer (perceived overhead vs native consoles).

**iTerm2 — official claims.**
- [Metal Renderer wiki (George Nachman)](https://gitlab.com/gnachman/iterm2/-/wikis/Metal-Renderer.md): "60 frame per second rendering... Increased throughput (e.g., when you `cat` a giant file, it'll finish sooner)."
- [iTerm2 news: "iTerm2 GPU Renderer Released" (2018-08-07)](https://iterm2.com/news.html): v3.2 Metal drawing engine, "Screen updates are much faster, leaving your CPU free... Scrolling is buttery smooth!"

**Warp / Zed — no official throughput numbers.** Only Mitchell's third-party measurement (above) and vague claims (e.g., [Warp docs](https://docs.warp.dev/llms-full.txt): "a modern, high-performance terminal... Built with Rust for high performance").

---

## Dimension 3 — Memory usage & scrollback (RAM vs disk)

**Ghostty — richest primary data.**
- [Ghostty config reference, `scrollback-limit`](https://ghostty.org/docs/config/reference): "The size of the scrollback buffer **in bytes**. This also includes the active screen... **Scrollback currently exists completely in memory.** ... It is not currently possible to set an unlimited scrollback buffer. This is a future planned feature." **Proves:** Ghostty scrollback is RAM-only; byte-budgeted; disk-paged history is a roadmap item.
- [Mitchell, scrollback compression (2026-07-09)](https://hachyderm.io/@mitchellh/116891440148870404): automatic scrollback compression yields "70 to 90% less physical memory usage", applied incrementally when idle, "no measurable effect on IO throughput"; default scrollback limit raised **10 MB → 50 MB**; implementation uses `madvise` to drop resident pages of the PageList while keeping virtual address space; "Unlimited, disk-paged history is on the roadmap too." **Proves:** RAM-compressed scrollback; concrete default-change numbers.
- [Mitchell, libghostty vs alacritty_terminal memory (2026-07-27)](https://hachyderm.io/@mitchellh/116993012549857824): a single Rust binary embedding both libghostty and alacritty_terminal, testing empty/full screens and 10K-row scrollback (plain/unicode/styled/mixed); claims libghostty memory is "significantly better" even uncompressed, with compression on by default. **Proves:** the only head-to-head embedder-level memory comparison; numbers are in an attached chart, not the text.
- [Mitchell, "Finding and Fixing Ghostty's Largest Memory Leak" (2026-01-10)](https://mitchellh.com/writing/ghostty-memory-leak-fix): leak present since 1.0, triggered at scale by Claude Code; a user reported **37 GB after 10 days**; data structure is a **PageList** (doubly-linked page-aligned mmap pages with a memory pool); fix in 1.3 (March 2026). **Proves:** Ghostty's memory architecture + an actual historical leak magnitude.

**Kitty / WezTerm / Alacritty / iTerm2 / Windows Terminal — scrollback is RAM-backed, defaults differ:**
- [kitty conf docs](https://sw.kovidgoyal.net/kitty/conf/): `scrollback_lines` **default 2000**, "Number of lines of history to keep **in memory**... Negative numbers are (effectively) infinite scrollback. Note that using very large scrollback is not recommended" (followed by memory caution).
- [WezTerm scrollback docs](https://wezterm.org/config/lua/config/scrollback_lines.html): `scrollback_lines` **default 3500**; the [scrollback guide](https://wezterm.org/scrollback.html) says "The larger this value, the more memory is required to manage the tab... may put some pressure on your system depending on the amount of RAM."
- [Alacritty v0.2 scrollback post (2018-09-17)](https://jwilm.io/blog/alacritty-lands-scrollback/): history config example `history: 10000`, in-memory by construction. (Alacritty's current default config documents `history_limit`; RAM-only.)
- [iTerm2 docs, Terminal preferences](https://iterm2.com/3.3/documentation-preferences-profiles-terminal.html): "**Unlimited scrollback will allow it to grow indefinitely, possibly using all available memory.**"
- [Windows Terminal profile-advanced docs](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-advanced): `historySize` **default 9001 lines, max 32767**.
- None of kitty/WezTerm/Alacritty/iTerm2/Windows Terminal provides disk-backed scrollback in primary docs.

**Zed / Warp — no primary memory documentation.**

---

## Dimension 4 — Startup time (cold/warm)

**No official primary benchmark exists for any of the 8 terminals.** What primary material exists:
- [WezTerm changelog](https://wezterm.org/changelog.html): "Improved startup performance on X11" (Nightly section, post-2026, PRs #5923/#5802) — qualitative.
- [Ghostty discussion #5364 "Slow startup time on Linux >1s"](https://github.com/ghostty-org/ghostty/discussions/5364), [#8475 "Further ways to reduce startup time?"](https://github.com/ghostty-org/ghostty/discussions/8475), [#13004 "Slow startup - help diagnosing?"](https://github.com/ghostty-org/ghostty/discussions/13004), [#7088 "Starting lag"](https://github.com/ghostty-org/ghostty/discussions/7088) — user-reported startup data in the project's own forum (secondary-quality, official venue).
- [Ghostty issue #4632](https://github.com/ghostty-org/ghostty/issues/4632): "Ghostty should not delay startup if dbus is unreasonably slow" — startup is partially dependent on platform services (DBus).
- [Windows Terminal issue #15001 "WT is relatively slow at executing cmd scripts at Windows startup"](https://github.com/microsoft/terminal/issues/15001) — user report.
- Everything else (Alacritty, kitty, iTerm2, Warp, Zed): no primary data found. **Flagged as a data gap.**

---

## Dimension 5 — Resize / reflow behavior

Contrary to a common belief that modern terminals "don't reflow", primary sources show most covered terminals DO reflow wrapped lines on resize:

- **Ghostty** reflows on resize: [ghostty issue #5718 "Terminal resize with reflow doesn't reflow the saved cursor (ESC 7)"](https://github.com/ghostty-org/ghostty/issues/5718). Resize UI: `resize-overlay` config ([config reference](https://ghostty.org/docs/config/reference), default `after-first`).
- **Kitty** reflows wrapped lines on resize: [kitty issue #8325 (2025-02-12)](https://github.com/kovidgoyal/kitty/issues/8325) — reproduction: "Open Kitty smaller than full screen... maximize the window so the text reflows to one line". Kovid Goyal's comment: "reflow behavior for terminals on resize is *completely unspecified* and terminal applications really can't make any assumptions about it currently. Someday I may sit down and write a spec for it..." **Proves:** kitty reflows; behavior is ad-hoc, no spec.
- **Alacritty** reflows: [issue #591 "Reflow on resize"](https://github.com/alacritty/alacritty/issues/591), [issue #3584](https://github.com/alacritty/alacritty/issues/3584), [PR #7873 "Fix logic for reflowing cursor when growing columns"](https://github.com/alacritty/alacritty/pull/7873).
- **WezTerm** reflows wrapped lines (imperfectly): [changelog fix #971](https://wezterm.org/changelog.html) — "if a line of text was exactly the width of the terminal it would get marked as wrappable... causing text to reflow incorrectly on resize"; long-standing [issue #14 "Smarter handling of wrapped lines on resize"](https://github.com/wezterm/wezterm/issues/14).
- **Windows Terminal** reflows the main buffer: [PR #4741 "Add support for 'reflow'ing the Terminal buffer"](https://github.com/microsoft/terminal/pull/4741) and [PR #4354 (don't remove lines from scrollback on resize)](https://github.com/microsoft/terminal/pull/4354); [PR #12719 "Don't reflow the alt buffer on resize"](https://github.com/microsoft/terminal/pull/12719).
- **iTerm2**: reflow observed by users ([HN comment, 2024-12-27](https://news.ycombinator.com/item?id=42518612): "iTerm seems to then reflow from the beginning [of a long scrollback], which can take a while"); **no primary documentation found** — flagged.
- **Resize cost (measured)**: no primary measurements. User observations only (same HN comment: iTerm2 reflow from start of long scrollback is slow; Ghostty "a bit more sluggish in resizing on an Intel Mac").
- **Font-size change cost**: no measured primary data for any terminal. Related primary config docs only: WezTerm `adjust_window_size_when_changing_font_size` ([config docs](https://wezterm.org/config/lua/config/adjust_window_size_when_changing_font_size.html)), Ghostty `font-size` ([config reference](https://ghostty.org/docs/config/reference)). **Flagged as a data gap.**

---

## Dimension 6 — Grid rendering correctness (emoji, CJK, wide chars, ligatures, long lines)

- **Ghostty**:
  - [Mitchell Hashimoto, "Grapheme Clusters and Terminal Emulators" (2023-10-02)](https://mitchellh.com/writing/grapheme-clusters-in-terminals): pasting 🧑‍🌾 moves the cursor 2, 4, 5 or 6 cells depending on terminal; explains cell-width inconsistencies and Ghostty's approach. **Proves:** the wide-char/grapheme problem space and Ghostty's stance.
  - [Config reference, `grapheme-width-method`](https://ghostty.org/docs/config/reference) (since 1.2.0): default `unicode` (Unicode-standard width; "may result in cursor-desync issues with some programs... that use a legacy method such as wcswidth"); `legacy` alternative. **Proves:** Ghostty exposes the unicode-vs-wcwidth tradeoff as a config.
  - Ligatures: `font-feature` config; "To disable programming ligatures, use -calt" / "To generally disable most ligatures, use -calt, -liga, -dlig" ([config reference](https://ghostty.org/docs/config/reference)). **Proves:** Ghostty supports ligatures via OpenType features.
- **Kitty**: official claims of ligatures + emoji with per-glyph font substitution (`symbol_map`) and variable fonts ([kitty index](https://sw.kovidgoyal.net/kitty/)); specialized native box-drawing rendering (multiple [changelog](https://sw.kovidgoyal.net/kitty/changelog/) entries, e.g., "Speed up rendering of box drawing characters by moving the implementation to native code").
- **WezTerm**: [font_shaper docs](https://wezterm.org/config/lua/config/font_shaper.html) — "The shaper is responsible for handling **kerning, ligatures and emoji composition**. The default is **Harfbuzz**"; `custom_block_glyphs` for pixel-perfect block glyphs (incl. braille/sextants; changelog).
- **Alacritty — no ligatures, no shaping** (by design):
  - [Issue #50 "Support for ligatures" (2017-01-04, open, locked)](https://github.com/alacritty/alacritty/issues/50) — the canonical no-ligatures ticket.
  - [PR #5696 (HarfBuzz ligatures, closed unmerged, 2021-12-17)](https://github.com/alacritty/alacritty/pull/5696); [PR #2677 (2019)](https://github.com/alacritty/alacritty/pull/2677) also closed.
  - [Issue #5245 (2021-06-16)](https://github.com/alacritty/alacritty/issues/5245): "doesn't support Ligatures, and most likely will never be... as it does require some more advanced (but solvable) text shaping techniques the developers are reluctant to implement as it *will* increase performance." **Proves:** explicit performance-vs-ligatures tradeoff decision.
- **iTerm2**: [Text preferences docs](https://iterm2.com/documentation-preferences-profiles-text.html): ligatures supported but "**This makes drawing much slower for two reasons: first, it disables the GPU renderer. Second, it uses a slower API.**" **Proves:** official statement that ligatures disable GPU rendering in iTerm2 (directly couples dims 1/6).
- **Windows Terminal**: AtlasEngine supports "Emojis, including zero width joiners" and "Custom font axes and features" ([PR #11623](https://github.com/microsoft/terminal/pull/11623)); font features/axes spec ([doc/specs/#1790](https://github.com/microsoft/terminal/blob/main/doc/specs/%231790%20-%20Font%20features%20and%20axes-spec.md)).
- **Zed / Warp**: no primary correctness documentation found — flagged. (Zed's grid logic is inherited from `alacritty_terminal`, so Alacritty's parser/cell constraints apply; rendering is GPUI's.)
- **Long lines**: no primary benchmark isolates very-long-line rendering for any terminal. Ghostty's CSI-heavy test ([toot 2026-07-06](https://hachyderm.io/@mitchellh/116873952162192565)) is the closest (control-sequence-heavy IO). **Flagged.**

---

## Contradictions and measurement-method differences

1. **"Fastest" claims collide.**
   - Alacritty README: "consistently score[s] better than the competition" using vtebench (current; vtebench-era data 2017–2018).
   - kitty perf docs: "kitty is twice as fast as the next best" (parser-only, rendering suppressed, kitty 0.33 / 2023; alacritty 54 vs kitty 134 MB/s avg).
   - Ghostty (Mitchell, 2026-07): "indisputably the fastest... more than 2x faster than any other leading 'fast' terminal" (end-to-end `cat`, nightly libghostty).
   - **Why they differ:** kitty's kitten benchmark suppresses rendering to isolate parser speed; Ghostty's `time cat` includes parsing+rendering through a real PTY; vtebench drives the PTY with generated sequences (different workloads, e.g., scrolling-in-region vs cat). Different years, hardware, and terminal versions; none of the three is a head-to-head of the same workload on the same machine, and kitty's 2023 table contains no Ghostty while Ghostty's 2026 table contains no WezTerm.
2. **Mitchell's own framing shifts by scope.** (2026-07-06) "indisputably fastest at IO throughput" vs (2026-06-08, HN) "We're sometimes faster, sometimes slower [vs Kitty], but in any case not noticeably so." The toot itself scopes to IO throughput ("No renderers involved") and Mitchell pre-empts the "cat speed doesn't matter" objection by arguing IO throughput bounds TUI workloads. **Resolution:** statements are about different metrics; the IO claim is the only one with published numbers.
3. **Latency figures:** kitty docs (typometer, "7ms vs 30ms", and "best in class") vs Hume's hardware numbers (kitty 36 ms, alacritty 50 ms — i.e., kitty better, consistent with Kovid's "35 vs 50" summary, but the "7 ms" typometer figure is from a different tool/environment and is not comparable to photodiode measurements). iTerm2's wiki ("most users will see improved latency" with the Metal renderer) is not supported by Hume's 2020 measurement (GPU 53.1 vs non-GPU 50.5 ms) — though that predates iTerm2 3.4's mature Metal renderer and uses a 2020 macOS/iTerm2 build.
4. **WezTerm "much slower than Ghostty"** ([Mitchell toot 2026-07-06](https://hachyderm.io/@mitchellh/116874257465889808): "its performance is somewhere much slower than Ghostty and much faster than Iterm") is an opinion, not a measurement, and no cross-terminal dataset contains both. kitty's 2023 parser table actually shows wezterm (48.5 MB/s avg) close to alacritty (54.05) — not "much slower" than anything there. **Method note:** qualitative maintainer comparisons cannot be reconciled with numbers because no shared benchmark includes both terminals.
5. **Windows Terminal renderer messaging:** PR #11623 (2021): "Runs at up to native display refresh rate"; 1.13 release notes (2022-02): "the performance improvements aren't generally noticeable" (AtlasEngine was opt-in then; it later became the default). The official position evolved from "revolutionary" to "not generally noticeable" to default-on — cite the version you mean.

---

## Explicit data gaps (no primary source found)

| Terminal | Latency | Throughput | Memory | Startup | Resize cost | Correctness |
|---|---|---|---|---|---|---|
| Ghostty | no official numbers (design statements only) | official (2026 toots) | official (2026) | user reports only | reflow proven; cost unmeasured | official (blog/config) |
| Kitty | official claims + 3rd-party | official (parser-only) | defaults only (scrollback_lines 2000) | none | reflow proven (issue 8325); cost unmeasured | official (index/changelog) |
| WezTerm | none | kitty's table only (48.5 MB/s) | defaults only (3500) | changelog qualitative | reflow partial (changelog #971) | official (font_shaper) |
| Alacritty | 3rd-party only | official (vtebench, 2017-18) | defaults only | none | reflow proven (issues/PRs) | official (no-ligature policy) |
| iTerm2 | wiki claim only | official claims (news/wiki) | "unlimited" warning only | none | reflow user-observed only | official (ligature costs GPU) |
| Warp | none | Mitchell's numbers only | none | none | none | none |
| Windows Terminal | none | official (renderer notes) | defaults only (9001/32767) | user reports only | reflow proven (PRs) | official (PR/spec) |
| Zed | none | none | none | none | none | none (inherits alacritty_terminal) |

Other flagged gaps:
- **Sleipnir**: could not be verified from any primary source (HN, GitHub, crates.io searches, 2026). The verified GPUI + `alacritty_terminal` projects are Zed's built-in terminal ([crates/terminal/Cargo.toml](https://github.com/zed-industries/zed/blob/main/crates/terminal/Cargo.toml)) and [zortax/gpui-terminal](https://github.com/zortax/gpui-terminal) ("A terminal emulator component for GPUI. Uses alacritty_terminal for VTE parsing"). Zed terminal performance facts: none beyond architecture; rendering inherits GPUI's Metal pipeline ([Zed, "Optimizing the Metal pipeline to maintain 120 FPS in GPUI" (2024-02-07)](https://zed.dev/blog/120fps)).
- **Font-size change cost, long-line rendering, per-pane memory numbers, warm/cold startup times**: no primary data for any terminal.

---

## Bibliography (all cited URLs)

- Ghostty: https://ghostty.org/docs/about · https://ghostty.org/docs/config/reference · https://mitchellh.com/writing/ghostty-1-0-reflection (2024-12-26) · https://mitchellh.com/writing/ghostty-devlog-006 (2024-02-12) · https://mitchellh.com/writing/grapheme-clusters-in-terminals (2023-10-02) · https://mitchellh.com/writing/ghostty-memory-leak-fix (2026-01-10) · https://hachyderm.io/@mitchellh/116873952162192565 (2026-07-06) · https://hachyderm.io/@mitchellh/116873980514808799 (2026-07-06) · https://hachyderm.io/@mitchellh/116874083461527212 (2026-07-06) · https://hachyderm.io/@mitchellh/116874257465889808 (2026-07-06) · https://hachyderm.io/@mitchellh/116891440148870404 (2026-07-09) · https://hachyderm.io/@mitchellh/116993012549857824 (2026-07-27) · https://hachyderm.io/@mitchellh/114767017754076838 (2025-06-29) · https://news.ycombinator.com/item?id=42518612 · https://news.ycombinator.com/item?id=48448945 · https://github.com/ghostty-org/ghostty/issues/5718 · https://github.com/ghostty-org/ghostty/discussions/5364 · https://github.com/ghostty-org/ghostty/discussions/5113 · https://github.com/ghostty-org/ghostty/issues/4632
- Kitty: https://sw.kovidgoyal.net/kitty/performance.html · https://sw.kovidgoyal.net/kitty/conf/ · https://sw.kovidgoyal.net/kitty/faq.html · https://sw.kovidgoyal.net/kitty/changelog/ · https://github.com/kovidgoyal/kitty/issues/2701 (comments 2020-05-31 and typometer) · https://github.com/kovidgoyal/kitty/issues/8325 (2025-02-12)
- Alacritty: https://github.com/alacritty/alacritty (README FAQ) · https://jwilm.io/blog/announcing-alacritty/ (2017-01-06) · https://jwilm.io/blog/alacritty-lands-scrollback/ (2018-09-17) · https://github.com/alacritty/vtebench · https://github.com/alacritty/alacritty/issues/50 · https://github.com/alacritty/alacritty/pull/5696 · https://github.com/alacritty/alacritty/issues/5245 · https://github.com/alacritty/alacritty/issues/591 · https://github.com/alacritty/alacritty/pull/7873
- WezTerm: https://wezterm.org/config/lua/config/front_end.html · https://wezterm.org/config/lua/config/scrollback_lines.html · https://wezterm.org/scrollback.html · https://wezterm.org/config/lua/config/font_shaper.html · https://wezterm.org/changelog.html · https://github.com/wezterm/wezterm/issues/14
- iTerm2: https://iterm2.com/news.html (2018-08-07) · https://gitlab.com/gnachman/iterm2/-/wikis/Metal-Renderer.md · https://iterm2.com/documentation-preferences-profiles-text.html · https://iterm2.com/3.3/documentation-preferences-profiles-terminal.html
- Windows Terminal: https://github.com/microsoft/terminal/pull/11623 · https://devblogs.microsoft.com/commandline/windows-terminal-preview-1-13-release/ (2022-02-12) · https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-advanced · https://learn.microsoft.com/en-us/windows/console/pseudoconsoles · https://github.com/microsoft/terminal/pull/4741 · https://github.com/microsoft/terminal/pull/4354 · https://github.com/microsoft/terminal/pull/12719 · https://github.com/microsoft/terminal/blob/main/doc/specs/%231790%20-%20Font%20features%20and%20axes-spec.md
- Latency benchmarks: https://thume.ca/2020/05/20/making-a-latency-tester/ (2020-05-20) · https://pavelfatin.com/typometer/
- Zed: https://github.com/zed-industries/zed/blob/main/crates/terminal/Cargo.toml · https://zed.dev/docs/terminal · https://zed.dev/blog/120fps (2024-02-07) · https://github.com/zortax/gpui-terminal
- Warp: https://www.warp.dev/blog/how-warp-works · https://docs.warp.dev/llms-full.txt
