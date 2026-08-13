# Sleipnir 竞品缺口调研：用户体验与性能

| Field | Value |
|-------|-------|
| **Date** | 2026-08-13 |
| **Status** | Research complete（调研 + 交叉验证）；实现未开始 |
| **Scope** | Sleipnir（本仓库，macOS + Windows，GPUI GPU 渲染）vs Ghostty / kitty / WezTerm / Alacritty / iTerm2 / Warp / Windows Terminal（辅：Zed 内置终端，因 Sleipnir 复用其 GPUI + alacritty_terminal 栈） |
| **Focus** | **用户体验**与**性能**两个维度（不同于 `competitive-research-features.md` 的功能矩阵；本报告只讲缺口与差距） |
| **Method** | 三条并行调研线（性能 / UX / 产品边界）+ 本仓库源码核对；外部结论仅采一手来源（官方文档、changelog、维护者声明、可复现 benchmark），二手来源只做线索 |

---

## 执行摘要

Sleipnir 的**架构底子很强**：GPUI（Metal/D3D）GPU 渲染 + `alacritty_terminal` 仿真后端，与 Zed 同源。在「渲染平滑、滚动回填重负载下不卡顿」这类体验上，它天然站在第一梯队；智能粘贴（图→路径 / Finder→quoted / 强制纯文本）和 divider 拖动时的**实时 PTY reflow** 是它相对多数竞品的**差异化优势**。

但在用户明确关心的两个维度上，存在三类缺口：

1. **性能：最大缺口是「没有数据」。** 没有任何一手来源能查到 Sleipnir 的延迟/吞吐/内存数字（性能调研线明确标注「Sleipnir could not be verified from any primary source」）。这既是证据缺口，也是产品缺口——一个主打「fast」的终端，在 2026 年没有可引用 benchmark，就无法在 Ghostty（575 ms cat 150 MB）、kitty（官方延迟声明）面前建立信任。其次是**滚动回填内存策略落后**：Sleipnir 是「固定 10,000 行」纯内存，而 Ghostty 已做到「字节预算 + 自动压缩（物理内存省 70–90%）」。

2. **用户体验：搜索、tab 拖拽、链接预览、shell 集成深度是四块明确差距。** 搜索只有字面匹配（源码 `regex_escape_literal`），无正则/大小写，也无「scrollback 导出到文件/编辑器」；tab 无法拖拽重排或拖出新窗口；链接/路径只有下划线、无 hover 预览 tooltip；shell 集成停在「detect + 跳 prompt + 完成通知」，缺自动注入、新建 tab/split 继承 cwd、点击移动光标等语义层。

3. **产品边界：「不内置 AI」依然成立，「不做图形协议」最脆弱。** 2026 年 AI 已由 agent CLI（Claude Code / Codex CLI / Gemini CLI）跑进终端，而非终端内置聊天；「无 AI」站得住（Ghostty 零 AI 宣称每日数百万人/机器）。但 kitty graphics 已成图像类工具的事实标准，Ghostty/WezTerm/Windows Terminal 在 2024–2025 都落地了图形路径，Sleipnir 完全没有——这是需要单独重议的边界，而非「明确不做」能一笔带过。

**结论一句话：** 先补「所有主流终端都有、Sleipnir 没有」的搜索与 tab 拖拽（低成本高感知），再补 shell 集成深度与滚动回填内存策略（中等成本高价值），最后**给性能一个可引用的基准数字**——这是把「我们很快」变成「我们有数据」的关键一步。

---

## 1. 现状基线（Sleipnir，源码核实）

> 所有「现状」均以本仓库当前源码为准，附文件与行号；旧调研 `competitive-research-features.md`（写于 M10）里标「❌」的 M11–M15 项现已全部落地，本报告以 README Roadmap 的「M0–M15 ✅」为准。

**已具备（README / CHANGELOG）：** GPU 渲染 · tabs/splits/PaneTree · 多窗口 · session 结构恢复 · 命令面板 · scrollback 查找 · 连字 · 智能粘贴（图→路径 / Finder→quoted / 强制纯文本）· 字体热缩放 · pane zoom · 失焦变淡 · broadcast · OSC 133 跳 prompt · 完成通知 · Quick Terminal / Quick Select · 自动更新 · 关闭确认 · 路径点击打开。

**关键配置默认值**（`crates/sleipnir_settings/src/sleipnir_settings.rs:206-241`）：

| 项 | 默认 | 行号 |
|---|---|---|
| `font_size` | 14 px | `:211` |
| `max_scroll_history_lines` | `10_000`（纯内存，行数预算） | `:225` |
| `scroll_multiplier` | 3.0 | `:226` |
| `minimum_contrast` | 45.0 | `:228` |
| `bell` | `off` | `:231` |
| `copy_on_select` | `false` | `:222` |
| `confirm_close` | `dirty` | `:236` |
| `background_opacity` | 1.0（不透明） | `:238` |
| `notify_on_command_finish_secs` | 5 | `:239` |

**已核实的行为细节（用于下文缺口判断）：**

- 搜索是**字面匹配**：`app_shell.rs:1386` 对查询串调用 `regex_escape_literal(&query)`，把底层 alacritty 的 regex 能力（`RegexSearch`）刻意转义成字面——即无正则、无大小写/整词选项。
- 字体缩放是**窗口级** override（`app_shell.rs:271` `font_size_override`），应用到该窗口所有 pane（`set_font_size_override`），非 per-pane。
- session 恢复**仅结构级**：`session.rs:1-4` 明确「Only structure is restored — not scrollback, running processes, or window geometry」。
- 新建 tab / split **不继承当前 pane 的 cwd**：`add_tab`（`app_shell.rs:803`）与 `split_active`（`:982`）都走 `spawn_term_view(window, cx)`（无 cwd 参数，回落 home）；只有 session 恢复走 `spawn_term_view_with_cwd(resolve_cwd(...))`。
- `path:line:col` 解析**已实现**：`sleipnir_ui.rs:833` `parse_path_line_col`，含 `src/main.rs:10:2`、`file:///...:3:4`、`C:\foo\bar.rs:10:2` 等测试（`:1181-1235`）。
- OSC 8 超链接**已支持**（继承 alacritty_terminal：`terminal/src/alacritty/hyperlinks.rs:222` `try_osc8_url_to_path`）。
- 无图形协议：全 `crates/` 无 `sixel` / kitty graphics / inline image 任何实现（grep 零命中）。
- 网格宽度模型继承 alacritty_terminal（`terminal/src/alacritty.rs:503` 仅有 `is_wide_char_spacer`，即 wcwidth 语义），**非** Ghostty 的 Unicode grapheme-width。

---

## 2. 性能对比

### 2.1 架构差异（先讲清楚，所有性能判断都建立在此之上）

Sleipnir 不是「Alacritty 套个壳」，也不是「Ghostty 的 Metal 渲染器」。它的两层是：

- **仿真层** = `alacritty_terminal`（与 Zed 同一个 fork pin）：负责 PTY 字节 → 网格（cell）的解析、回填、超链接/搜索/OSC 处理。→ **仿真正确性 ≈ Alacritty 级**。
- **渲染层** = GPUI 文本批（Zed 的 UI 框架）：负责网格 → 屏幕 glyph。→ **渲染管线 ≈ Zed 级**（Zed 有 [120fps blog](https://zed.dev/blog/120fps)，GPUI 走 Metal 直接合成）。

因此：Sleipnir 的**吞吐上限**既不是 Alacritty 专用 GPU 渲染器、也不是 Ghostty 的手写 Metal 渲染器，而是 GPUI 文本批 + `alacritty_terminal` 的组合——这正是 Zed 内置终端同款路径。这是「有潜力快」的依据，也是「没测过就别说快」的理由。

### 2.2 输入延迟

- **kitty（唯一有官方延迟声明的终端）**：默认 `input_delay` = **3 ms**（[conf docs](https://sw.kovidgoyal.net/kitty/conf/)）；维护者 Kovid Goyal 在 [issue #2701 (2020-05-31)](https://github.com/kovidgoyal/kitty/issues/2701#issuecomment-636497270) 给过硬件测量「kitty 35 ms vs alacritty 50 ms」和 typometer「7 ms vs 30 ms」。
- **独立硬件测量（Tristan Hume，[2020-05-20](https://thume.ca/2020/05/20/making-a-latency-tester/)，Teensy+光敏电阻）**：kitty **36.1 ms**（中位 35），Apple Terminal 35.8 ms，alacritty **50.4 ms**，iTerm2 无 GPU 50.5 / GPU 53.1 ms。作者结论：kitty/Terminal.app「近似最优」，iTerm2 与 Alacritty 更差；且 iTerm2 的 GPU 渲染器**没有**降低该测试里的延迟。
- **Sleipnir：无任何数字**（性能线明确「No official latency numbers」覆盖 Ghostty/Warp/WT/Zed/WezTerm/Alacritty，Sleipnir 更无从谈起）。间接依据仅 GPUI 的 Metal 直合成管线设计上利于低延迟。

### 2.3 渲染吞吐与背压

- **Ghostty（维护者实测，[Mitchell 2026-07-06](https://hachyderm.io/@mitchellh/116873952162192565)）**：`time cat 150MB_ascii.txt` → Ghostty nightly **575 ms**、Ghostty 1.3.2 1.5 s、Alacritty **1.2 s**、kitty **1.7 s**、Warp 3.8 s、iTerm2/Terminal >60 s（中止）；Unicode 文件 536 ms vs Alacritty 1.05 s / kitty 1.35 s。作者注明「这只是 IO 吞吐测试，不涉及渲染器」。
- **kitty（官方，2023，解析器 only、**渲染被抑制**）**：`kitten __benchmark__` 平均 MB/s：kitty **134.55**、gnome-terminal 61.83、alacritty 54.05、**wezterm 48.5**、xterm 30.72（[performance docs](https://sw.kovidgoyal.net/kitty/performance.html)）。
- **Alacritty（官方，2017–18）**：macOS 滚动「比第二名快 9 倍以上」、Linux「约快 2.5%」（[scrollback 博客](https://jwilm.io/blog/alacritty-lands-scrollback/)，vtebench）。
- **Sleipnir：无数字。** 这是本报告最想强调的一点。

### 2.4 内存与滚动回填

| 终端 | 滚动回填策略 | 来源 |
|---|---|---|
| **Ghostty** | RAM-only，**字节预算**（默认 10 MB→已提至 50 MB），**自动压缩省物理内存 70–90%**，`madvise` 页管理，磁盘分页在 roadmap | [config](https://ghostty.org/docs/config/reference)、[Mitchell 2026-07-09](https://hachyderm.io/@mitchellh/116891440148870404) |
| kitty | 行数，默认 **2000**（负数=无限，纯内存） | [conf](https://sw.kovidgoyal.net/kitty/conf/) |
| WezTerm | 行数，默认 **3500** | [docs](https://wezterm.org/config/lua/config/scrollback_lines.html) |
| Windows Terminal | `historySize` 默认 **9001**，最大 32767 | [docs](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-advanced) |
| **Sleipnir** | 行数，默认 **10,000**，纯内存，无压缩/无字节预算 | `sleipnir_settings.rs:225` |

**判断：** Sleipnir 的 10,000 行上限在行数语义里不算低，但「固定行数 + 纯内存 + 无压缩」已经落后于 Ghostty 的「字节预算 + 自动压缩」路线。长输出场景（编译日志、大文件 cat）下，Sleipnir 要么内存膨胀、要么提前截断。**这是最值得抄的一个性能点子。**

### 2.5 启动时间

- **全行业无一手基准**（性能线明确「No startup benchmarks at all, any terminal」）。Sleipnir 同样无数据。标为证据缺口，不臆造。

### 2.6 缩放 / reflow

- 与常见直觉相反，多数终端**都**做 reflow：Ghostty（[issue #5718](https://github.com/ghostty-org/ghostty/issues/5718)）、kitty（Kovid 在 [issue #8325](https://github.com/kovidgoyal/kitty/issues/8325) 称 reflow 行为「完全未定义」）、Alacritty（[#591](https://github.com/alacritty/alacritty/issues/591)/[#3584](https://github.com/alacritty/alacritty/issues/3584)）、WezTerm（[#14](https://github.com/wezterm/wezterm/issues/14)）、Windows Terminal（[#4741](https://github.com/microsoft/terminal/pull/4741)）。
- **Sleipnir 的差异点是「divider 拖动时实时 PTY reflow」**（`docs/adr/0003-live-pty-reflow-on-drag.md`：拖动分栏实时发 SIGWINCH，而非 preview-line 后松手再 resize）。这是刻意为之的「native 感」取舍，且比 iTerm2/Ghostty 用户反馈的「从超长滚动回填从头 reflow、要等一会儿」体验更激进。**这是优势，应守住。**
- 窗口 resize / 字号热变的**量化成本**：全行业无测量（性能线标注）。

### 2.7 网格渲染正确性（emoji / CJK / 宽字符 / 连字）

- Ghostty 是唯一把 grapheme 宽度讲透的：`grapheme-width-method` 默认 `unicode`（自 1.2.0），🧑🌾 按终端不同渲染成 2/4/5/6 个 cell（[Grapheme Clusters 博客](https://mitchellh.com/writing/grapheme-clusters-in-terminals)）。
- Alacritty **刻意不做连字**（性能优先，[issue #50](https://github.com/alacritty/alacritty/issues/50)、[#5245](https://github.com/alacritty/alacritty/issues/5245)「most likely will never be」）。iTerm2 的连字「much slower… disables the GPU renderer」（[docs](https://iterm2.com/documentation-preferences-profiles-text.html)）。
- **Sleipnir**：连字是**可选项**（`font_ligatures`，默认 false，`sleipnir_settings.rs:234`）——比 Alacritty 灵活；CJK/emoji 回退在 0.1.7 修好（CHANGELOG）。但宽度模型继承 `alacritty_terminal`（wcwidth 语义，`alacritty.rs:503`），**与 Ghostty 的 Unicode grapheme-width 默认不同**：🧑🌾 这类 ZWJ 序列的 cell 宽度可能不一致。**中置信度推断，待实测确认**（见 §6 证据缺口）。

### 2.8 图形协议（也见 §4 产品边界）

- Sixel：Windows Terminal 1.22（2024-08）、WezTerm 支持；**Alacritty 无**（CHANGELOG 到 0.17.0 零条目，强验证负例）、kitty 无（RFC [#2511](https://github.com/kovidgoyal/kitty/issues/2511) 自 2020 未实现）、Ghostty 无（官方文档只列 kitty graphics）。
- kitty graphics 协议已成**事实标准**（[spec](https://sw.kovidgoyal.net/kitty/graphics-protocol/)）：Ghostty、WezTerm、kitty 支持，客户端生态包括 fzf/mpv/nvim/ranger/timg/chafa/broot。
- iTerm2 走自有 `OSC 1337;File=` 协议（WezTerm 也兼容它）。
- **Sleipnir：无任何图形协议。** Alacritty（仍无图、仍广泛使用）证明「缺图可活」，但围绕 kitty graphics 的工具生态（yazi 文件管理器、nvim 图片预览、演示器）会与 Sleipnir 不兼容。

---

## 3. 用户体验对比

> 每维度给「Sleipnir 现状 → 最佳实践 → 差距」。竞品事实与来源详见 UX 调研全文（`docs/research/terminal_ux_report.md`），此处只保留结论与一手来源链接。

### D1 搜索 — **明确差距**
- Sleipnir：字面匹配、无正则/大小写/整词，无 scrollback 导出（`app_shell.rs:1386`）。
- 最佳：iTerm2 最全（正则 ICU、智能大小写默认、live 高亮、跨 tab 全局搜索、Filter 隐藏非匹配行，3.5.0 起）；Ghostty 的 `write_scrollback_file`/`write_screen_file`/`write_selection_file` 可复制/粘贴/在编辑器打开（1.3.0）。
- **差距：正则/大小写选项 + scrollback→文件/编辑器导出。** kitty 拒绝 live 搜索（性能哲学，[issue #893](https://github.com/kovidgoyal/kitty/issues/893)）说明「live」不是必须，但「正则」与「导出」是普遍能力。

### D2 超链接 / 路径 — 基本达标，缺预览
- Sleipnir：URL 打开 + `path_links` + `path:line:col` 解析（`sleipnir_ui.rs:833`）+ OSC 8 + hover 下划线（M11）。
- 最佳：iTerm2 Smart Selection（可编辑正则+Precision）+ Semantic History（`\1` 文件名 `\2` 行号，3.4 起 `file:line:col`）；kitty hints kitten `path:line` 精确跳行；Ghostty 的 `link-previews`（1.2.0）。
- **差距：hover 预览 tooltip（iTerm2/kitty/Ghostty 有，Sleipnir 只有下划线）；`path:line:col` 用系统 `open` 打开、未必跳转到编辑器指定行列（iTerm2 Semantic History 用 `\1/\2` 精确到行）。**

### D3 Shell 集成 — **深度差距**
- Sleipnir：OSC 133 detect + 跳上/下 prompt（`⌘⇧↑/↓`）+ 完成通知（`notify_on_command_finish_secs=5`），`prompt_markers` 上限 500（`terminal.rs:1994-1996`）。
- 最佳：Ghostty（自动注入 bash/elvish/fish/nushell/zsh + `jump_to_prompt` + ctrl/cmd 三击选命令输出 + opt/alt 点击移动光标 + 1.3.0 更完整的 OSC 133）；iTerm2（marks + 命令时长/退出码 + 「下一条 mark 提醒」）。
- **差距：Sleipnir 是「detect-first」——不自动注入 shell 集成脚本；新建 tab/split 不继承 cwd（§1 已证，回落 home）；无「点击移动光标 / 三击选命令输出」。**

### D4 Session 恢复 — 结构级达标，深度可后置
- Sleipnir：结构级（布局+cwd），不恢复 scrollback/进程/窗口几何（`session.rs:1-4`）。
- 最佳：iTerm2 是七者中**唯一**能 crash/升级后 reattach 到运行进程的（long-lived servers）；WezTerm 靠 mux reattach；WT 1.21 起恢复屏显内容。
- **差距：scrollback 恢复（WT/iTerm2 有，中等成本）；进程 reattach（仅 iTerm2/WezTerm，工程量大，建议 P2）。**

### D5 主题 / 字体 — 主题库小，min-contrast 已达标
- Sleipnir：约 10 套主题（Catppuccin + Tokyo Night/Nord/Gruvbox/Solarized/GitHub）；**已有 `minimum_contrast=45.0`**（`sleipnir_settings.rs:228`）；`auto` 跟随系统外观。
- 最佳：WezTerm 700+/1001 套主题 + 自动热重载；iTerm2 min-contrast 稳定滑块；Ghostty `minimum-contrast`（WCAG 2.0，1.3.0）。
- **差距：主题库数量与导入（P2）。** 注意：调研确认「**没有任何终端支持真正的 per-pane 独立字体**」（kitty 明确同窗口同字号）——所以 Sleipnir 的 per-pane 字体 defer 是**行业一致、非缺口**，不必再当差距追。

### D6 复制 / 粘贴 — 智能粘贴是领先项
- Sleipnir：智能粘贴（图→路径、Finder/Explorer→quoted、强制纯文本）**领先**；`copy_on_select` 默认 false。
- copy-on-select 默认值行业 3 开 / 4 关（开：iTerm2/Warp/Ghostty；关：kitty/Alacritty/WT；WezTerm 靠默认绑定等效开）。Sleipnir 归「关」阵营，符合 kitty/Alacritty 惯例，**非缺口**。矩形/块选择：Sleipnir 未明确，**轻微缺口**（iTerm2/Warp/WezTerm/Alacritty 均有）。

### D7 Tab / 分屏操作 — **tab 拖拽是明确差距**
- Sleipnir：tabs/splits/PaneTree、divider 拖动（实时 reflow）、pane zoom、失焦变淡、broadcast；**无 tab 拖拽重排 / 拖到新窗口 / pane 跨 tab 拖动**。
- 最佳：kitty（0.46.0 拖 tab、0.47.0 拖 pane，均可跨 OS 窗口）；Warp（拖 tab 出窗口 + pane 跨 tab 拖 + 布局持久化）；iTerm2/WT 亦支持。WezTerm 是反面（拖拽请求 [#549](https://github.com/wezterm/wezterm/issues/549) 自 2021 挂起）。
- **差距：tab 拖拽重排 + 拖到新窗口。** 这是「每日高频、感知强」的交互。

### D8 Quick Terminal — 已具备，核查是否全局热键
- Sleipnir：Quick Terminal（`⌘⇧N`，M15）。
- 最佳：Ghostty `toggle_quick_terminal` 用 `global:` 前缀的**全局 OS 热键**（macOS 需 accessibility 权限）；iTerm2 Dedicated Hotkey Window 最成熟；kitty 0.42+ quake kitten；WT Quake Mode；Warp Global Hotkey。
- **待核查：Sleipnir 的 `⌘⇧N` 是应用内快捷键还是全局热键。** 若是前者，则缺「失焦时全局唤起」这一 Quick Terminal 的核心价值（P2）。

### D9 无障碍 — 行业普遍弱，min-contrast 已达标
- Sleipnir：`minimum_contrast=45.0` 已有；无屏幕阅读器。
- 最佳：Windows Terminal 是七者中唯一有文档化 UIA 屏幕阅读器（2019→2022 演进）；Warp macOS VoiceOver WIP（自述导航受限）；Ghostty macOS 只读 AX（1.2.0，opt-in）。
- **差距：屏幕阅读器（P2）。** 全行业除 WT 外普遍缺失，Sleipnir 现状不落后于多数竞品。

### D10 通知 — 基础达标，矩阵可深化
- Sleipnir：完成通知（macOS 通知，Windows 仅日志）+ bell（system/visual）。
- 最佳：kitty 最深（`notify_on_cmd_finish` 阈值+动作矩阵+自定义命令 + 最全 bell + OSC 99）；Ghostty `notify-on-command-finish`（never/unfocused/always + 阈值 + 可组合 bell/notify，1.3.0）；iTerm2 Notification Center + OSC 1337 富通知。
- **差距：通知阈值/动作矩阵、OSC 9/777 桌面通知（P1/P2）。**

---

## 4. 产品边界复核（「明确不追」是否仍成立）

| 边界 | 2026 事实 | 复核结论 |
|---|---|---|
| **不内置 AI** | Warp 全押 AI/agents（Windows 2025-02、2.0 ADE 2025-06、3.2B 行编辑、2026-04 开源）；iTerm2 AI 仅 opt-in（3.5.0 需自带 key，3.5.1 移入企业可屏蔽的独立插件）；kitty/WezTerm/Ghostty **无 AI**；真正的趋势是 agent CLI（Claude Code 2025-02 / Gemini CLI / Codex CLI）跑进终端 | **仍成立。** 但注意：趋势要求的是「**对 agent 友好**」——shell 集成、滚动回填语义、重输出下稳定——而非内置聊天。这正好指向 §3 的 shell 集成深度与 §2 的吞吐/背压 |
| **不做图形协议** | kitty graphics 成事实标准，Ghostty/WezTerm/WT 2024–2025 都落地图形路径；Alacritty 0.17.0 仍无图（证明可活，但生态脱节） | **最脆弱。** 建议从「明确不做」改为「远期/P2 重议」，至少跟踪 kitty graphics |
| **无脚本平台** | kitty kittens + `@` 远端控制；WezTerm 全 Lua；iTerm2 Python API；Ghostty 仅 macOS AppleScript（1.3.0）；Alacritty 无 | **可守。** 若要低成本对齐，加一个只读+quit 的最小 AppleScript 字典即可（Ghostty 同款） |

---

## 5. 缺口优先级矩阵（决策表）

| 优先级 | 缺口 | 证据强度 | 工程影响 | 建议动作 |
|---|---|---|---|---|
| **P0** | **性能基准缺失** | 高（一手负例） | 高——「fast」无据可依 | 用 vtebench + 延迟测试器 + `cat` 大文件实测，写进 README/docs |
| **P0** | 搜索无正则/大小写、无 scrollback 导出 | 高（源码+竞品） | 中 | 打开底层 regex（去掉 `regex_escape_literal` 或加开关）+ `write_scrollback_file` 式导出 |
| **P0** | tab 拖拽重排 / 拖到新窗口 | 高（kitty 0.46/0.47 等一手 changelog） | 中 | PaneTree 已有树结构，加 drag 重排与 detach |
| **P0** | 链接/路径 hover 预览 tooltip | 高 | 低 | M11 已有 hover 下划线，加 tooltip |
| **P1** | 滚动回填内存策略（字节预算+压缩） | 高（Ghostty 一手数据） | 中 | `max_scroll_history_lines` 之外加字节预算/压缩 |
| **P1** | shell 集成深度（自动注入、新 tab/split 继承 cwd、点击移光标、选命令输出） | 高 | 中 | 在 OSC 133 detect 之上补注入脚本 + cwd 继承 |
| **P1** | 网格宽度正确性核查（🧑🌾 等 ZWJ） | 中（推断） | 中 | 实测 vs Ghostty `unicode` 方法，决定是否接 grapheme-width |
| **P1** | 通知阈值/动作矩阵 + OSC 9/777 | 高 | 低 | 扩展现有 `notify_on_command_finish_secs` |
| **P2** | session 恢复加 scrollback | 中 | 中 | 结构恢复之上存屏显内容 |
| **P2** | 进程 reattach | 高（仅 iTerm2/WezTerm） | 高 | 大工程，明确延期 |
| **P2** | Quick Terminal 全局热键核查 | 中（待核查） | 低 | 若现为应用内快捷键，补 `global:` 全局绑定 |
| **P2** | 屏幕阅读器 | 中 | 高 | 行业普遍弱，可延后 |
| **P2** | 主题库扩充 + 导入 | 高 | 低 | 接 iterm2-color-schemes |
| **P2（重议）** | kitty graphics 协议 | 高 | 高 | 从「不追」改为跟踪，评估与 GPUI 文本批的集成成本 |
| **P2（可选）** | 最小 AppleScript 字典 | 高 | 低 | Ghostty 1.3.0 同款，只读+quit |

**应守住的领先项：** 智能粘贴（图→路径，唯一）· divider 实时 PTY reflow · GPU 渲染 + alacritty 仿真正确性 · 多窗口 + session 结构恢复 · OSC 133 跳 prompt + 完成通知（已具备）。

---

## 6. 风险与证据缺口

1. **Sleipnir 无一手性能数据**：所有性能判断均为「架构间接 + 竞品对标」，非实测。**这是首要缺口**，也是 P0 建议的由来。
2. **「最快」声明互相冲突**：Alacritty（vtebench，2017–18）vs kitty（解析器-only、渲染抑制，2023：kitty 134.55 vs alacritty 54.05 MB/s）vs Ghostty（端到端 `cat`，2026：575 ms vs kitty 1.7 s）——**工作负载不同、渲染开/关不同、年份/硬件不同，且无一个基准同时含三者**。不能直接比大小，只能分别引用。
3. **延迟数字口径差 ~5×**：kitty「7 ms vs 30 ms」（typometer）vs Hume 硬件「kitty 36 vs alacritty 50 ms」——两者都指向 kitty 更快，但量级不可混用。iTerm2 官方「Metal 改善延迟」未被 Hume 2020 复现（且是 3.4 之前的构建）。
4. **WezTerm 快慢定性矛盾**：Mitchell「WezTerm 比 Ghostty 慢得多」（[2026-07-06](https://hachyderm.io/@mitchellh/116874257465889808)，定性）vs kitty 2023 表（wezterm 48.5 ≈ alacritty 54.05）——无共同基准含 Ghostty 与 WezTerm，数字不可调和。
5. **Sleipnir 的 ZWJ/grapheme 宽度**：中置信度推断（继承 alacritty_terminal），未实测。
6. **Quick Terminal 全局热键**：待核查（§3 D8）。
7. **竞品数字多为一手但常无日期**：文档页普遍不带日期，changelog/release 日期为准；Ghostty 1.3.0 = 2026-03-09、1.3.1 = 2026-03-13。
8. **Warp 采用数字为厂商自报**（3.2B 行、96% 接受率），首页计数器为 JS 占位，不可独立验证。

---

## 7. 来源（按角色去重）

**官方文档 / changelog（一手）**
- Ghostty: [Config Reference](https://ghostty.org/docs/config/reference) · [1.1.0](https://ghostty.org/docs/install/release-notes/1-1-0) · [1.2.0](https://ghostty.org/docs/install/release-notes/1-2-0) · [1.3.0](https://ghostty.org/docs/install/release-notes/1-3-0) · [Shell Integration](https://ghostty.org/docs/features/shell-integration)
- kitty: [Configuration](https://sw.kovidgoyal.net/kitty/conf/) · [Performance](https://sw.kovidgoyal.net/kitty/performance.html) · [Changelog](https://sw.kovidgoyal.net/kitty/changelog/) · [Shell integration](https://sw.kovidgoyal.net/kitty/shell-integration/) · [Graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
- WezTerm: [Scrollback](https://wezterm.org/scrollback.html) · [Hyperlinks](https://wezterm.org/hyperlinks.html) · [Multiplexing](https://wezterm.org/multiplexing.html) · [Changelog](https://wezterm.org/changelog.html)
- Alacritty: [Configuration](https://alacritty.org/config-alacritty.html) · [CHANGELOG](https://raw.githubusercontent.com/alacritty/alacritty/master/CHANGELOG.md)
- iTerm2: [One-page docs](https://iterm2.com/documentation-one-page.html) · [3.5.0 changelog](https://iterm2.com/downloads/stable/iTerm2-3_5_0.changelog) · [Downloads](https://iterm2.com/downloads.html)
- Warp: [Blocks](https://docs.warp.dev/terminal/blocks/) · [Session restoration](https://docs.warp.dev/terminal/sessions/session-restoration/) · [2025 in Review](https://www.warp.dev/blog/2025-in-review) · [launching Warp on Windows](https://www.warp.dev/blog/launching-warp-on-windows)
- Windows Terminal: [Search](https://learn.microsoft.com/en-us/windows/terminal/search) · [Shell integration](https://learn.microsoft.com/en-us/windows/terminal/tips-and-tricks) · [Releases](https://github.com/microsoft/terminal/releases)

**维护者声明 / benchmark（一手，带日期）**
- Mitchell Hashimoto（Ghostty）吞吐：[2026-07-06](https://hachyderm.io/@mitchellh/116873952162192565) · 内存压缩：[2026-07-09](https://hachyderm.io/@mitchellh/116891440148870404) · 内存对比：[2026-07-27](https://hachyderm.io/@mitchellh/116993012549857824)
- Kovid Goyal（kitty）延迟：[issue #2701 comment](https://github.com/kovidgoyal/kitty/issues/2701#issuecomment-636497270)（2020-05-31）· reflow：[issue #8325](https://github.com/kovidgoyal/kitty/issues/8325)
- Tristan Hume 硬件延迟：[Making a Latency Tester](https://thume.ca/2020/05/20/making-a-latency-tester/)（2020-05-20）
- Joe Wilm（Alacritty）：[Announcing](https://jwilm.io/blog/announcing-alacritty/)（2017-01-06）· [Scrollback benchmarks](https://jwilm.io/blog/alacritty-lands-scrollback/)（2018-09-17）
- Zed：[/120fps blog](https://zed.dev/blog/120fps)（2024-02-07）

**本仓库（一手，本地）**
- `README.md` · `CHANGELOG.md` · `docs/settings.example.json`
- `crates/sleipnir_settings/src/sleipnir_settings.rs`（默认值 `:206-241`）
- `crates/sleipnir_ui/src/app_shell.rs`（搜索字面转义 `:1386`、字体窗口级 `:271`、new tab `:803` / split `:982`）
- `crates/sleipnir_ui/src/sleipnir_ui.rs`（`parse_path_line_col` `:833`）
- `crates/sleipnir_ui/src/session.rs`（仅结构恢复 `:1-4`）
- `crates/terminal/src/alacritty/hyperlinks.rs`（OSC 8 `:222`）
- `docs/adr/0003-live-pty-reflow-on-drag.md`

**调研线产物（本仓库外部工作区，供审计）**
- `docs/research/terminal_ux_report.md`（500 行，10 维度逐终端一手证据）
- `docs/research/terminal-emulator-performance-research.md`（性能 6 维度一手来源 + 完整书目）
- `docs/research/verification_notes.md`（UX 调研的独立 curl 验证笔记）
