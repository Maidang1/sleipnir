# Post-Competitive-Research Roadmap

| Field | Value |
|-------|-------|
| **Date** | 2026-08-14 |
| **Status** | **Docs only** — roadmap 定稿；分步实现需用户逐条点名（如「实现第 1 步的搜索」） |
| **Input** | [`docs/competitive-research-ux-performance.md`](../../competitive-research-ux-performance.md)（UX + 性能缺口调研，2026-08-13） |
| **Companion** | [`docs/perf-baseline.md`](../../perf-baseline.md)（第 0 步方法论）· [`docs/perf-baseline-results.md`](../../perf-baseline-results.md)（第 0 步实测结果） |

> 这份路线图把调研报告的执行摘要与缺口优先级矩阵（报告 §5）落成四步执行计划：先补数据（第 0 步），再补高频低成本 UX（第 1 步），再投深度能力（第 2 步），最后重议边界与锦上添花（第 3 步）。每步有明确时间盒、对标依据与验收出口。

---

## 总览

| 步 | 主题 | 时间盒 | 性质 |
|----|------|--------|------|
| **第 0 步** | 建立性能基线 | 1–2 天 | 先做，最优先 |
| **第 1 步** | 补高频、低成本的 UX 缺口 | 1–2 周 | 高感知、小改动 |
| **第 2 步** | 深度能力 | 2–4 周 | 价值最高、更重 |
| **第 3 步** | 边界重议与锦上添花 | 选择性 | 按需做 |

---

## 第 0 步：先建立性能基线（1–2 天，最该先做）

**做什么**：跑一组可复现的基准，把数字写进 `docs/`：

- **吞吐**：`time cat 150MB`（对标 Ghostty 575ms / kitty 1.7s / Alacritty 1.2s 那套测法）。
- **输入延迟**：typometer 或 Hume 的 Teensy 硬件法（对标 kitty 36ms / alacritty 50ms）。
- **内存**：滚动回填灌满 1 万行后的 RSS。
- **顺便验证两个「待核查」项**：
  1. 🧑‍🌾 这类 ZWJ 序列的 cell 宽度（vs Ghostty `unicode`）；
  2. Quick Terminal 的 `⌘⇧N` 是不是全局热键。

**为什么先做**：报告里最大的缺口是「没有数据」——你没法对「我们快」建立信任，也没法判断下面哪些性能优化是必要的。先量，再决定投不投。

> 说明：方法论与可自动化的数字已落盘（[`perf-baseline.md`](../../perf-baseline.md) + [`perf-baseline-results.md`](../../perf-baseline-results.md)）；剩余待办集中在**必须人在 GUI 前跑**的三项（渲染吞吐 / 输入延迟 / 回填内存增长）与两个待核查项（见 `perf-baseline.md` §4）。

---

## 第 1 步：补高频、低成本的 UX 缺口（1–2 周）

这些是「所有主流终端都有、Sleipnir 没有」、感知强、改动小：

| 做什么 | 依据 |
|--------|------|
| 搜索加正则/大小写开关 + scrollback 导出到文件/编辑器 | 现在只有字面匹配（`app_shell.rs:1386` 强制转义）；iTerm2/Ghostty/WezTerm 都有。**进度：正则/大小写开关 ✅ + scrollback 导出 ✅（Shell/File → Export Scrollback…，写临时文件并用默认编辑器打开）** |
| tab 拖拽重排 + 拖到新窗口 | kitty 0.46/0.47、Warp、WT、iTerm2 都已支持。**进度：✅ 已完成（拖到另一 tab 重排；拖到终端区 detach 到新窗口，跨窗口 re-wire 观察者）** |
| 链接/路径 hover 预览 tooltip | M11 已有下划线，只差 tooltip；iTerm2/kitty/Ghostty 有。**进度：✅ 已落地（hover 显示 URL/path 文本 tooltip）** |

---

## 第 2 步：深度能力（2–4 周，价值最高但更重）

| 做什么 | 依据 |
|--------|------|
| **shell 集成深化**：自动注入脚本 + 新建 tab/split 继承 cwd + 点击移动光标 + 三击选命令输出 | 现在 detect-first、新 tab/split 回 home（源码确认）；Ghostty/iTerm2 的语义层是它们「爽」的根因，也是「对 agent 友好」的关键。**进度：新建 tab/split 继承 cwd ✅；OSC 133 真实 PTY 已接线；`inject_osc133` 自动注入 zsh/bash/fish ✅（默认关）；Option/Alt-click 移光标 ✅；Cmd/Ctrl-三击选输出 ✅** |
| **滚动回填内存策略**：从「固定 1 万行」升级到字节预算 + 压缩 | Ghostty 已做到省 70–90% 物理内存。**进度：⏳ 待做；压缩需改 alacritty 回填存储（上游 fork 级改动）** |
| **通知矩阵**：阈值 + 动作组合 + OSC 9/777 | kitty/Ghostty 已有，Sleipnir 只有单一完成通知。**进度：阈值+动作矩阵 ✅（`notify_on_command_finish_mode`）；OSC 9/777 display-only + 真实 PTY ✅（vendor `osc_custom` → `Event::Notify`）** |

---

## 第 3 步：边界重议与锦上添花（选择性做）

| 做什么 | 依据 |
|--------|------|
| **kitty graphics**：从「明确不做」改为「跟踪」，先评估它在 GPUI 文本批里的集成成本，再决定要不要上 | 这是报告里最脆弱的那条边界。**进度：评估 ✅（[`ADR-0004`](../../adr/0004-kitty-graphics-track-not-implement.md)）→ 结论「跟踪、暂不上」** |
| **主题库扩充 + 导入** | 对齐 WezTerm 700+。**进度：✅ 全链路完成（13 套内置 + 601 套 iterm2-color-schemes 打包目录 `resources/themes.json`（MIT，脚本转换）+ `theme` 按名选择 + `themes.json` 用户覆盖 + 选择器搜索「type to filter」）** |
| **最小 AppleScript 字典**（只读 + quit） | Ghostty 1.3.0 同款。**进度：✅ 已完成（`resources/Sleipnir.sdef`：只读 name/version/frontmost + quit，`NSAppleScriptEnabled` + `OSAScriptingDefinition` 接入，`make-app.sh` 打包）** |
| **屏幕阅读器** | 行业除 WT 外都弱，可最后。**进度：只读 AX 基础 ✅（`Terminal::visible_screen_text` 暴露当前屏幕为 `MultilineTextInput` 的 value，VoiceOver 可朗读；Ghostty 1.2.0 同款只读 AX）；完整 AX 树 / 交互 ⏳ 待做** |

---

## 守住的领先项（不做减法）

智能粘贴（图→路径，唯一）· divider 实时 PTY reflow · GPU 渲染 + alacritty 仿真正确性 · 多窗口 + session 结构恢复 · OSC 133 跳 prompt + 完成通知。

## 关键依赖与阻塞（2026-08-14 排查）

第 2 步 OSC 公共根因 **已修**（ADR-0005）：`vendor/alacritty_terminal` + `vendor/vte` 给 `Handler` 加 `osc_custom`，真实 PTY 的 `EventLoop::parser.advance` 会 emit `Osc133` / `DesktopNotification`，经 `ZedListener` 进入 `record_osc133_marker` / `Event::Notify`。

仍待做的第 2 步项：
- 回填内存字节预算 + 压缩（另一处上游级改动）

> 滚动回填压缩（字节预算/压缩）是另一处上游级改动（alacritty 回填是行数语义，压缩需改其 grid 存储），与 OSC 无关。

## 落地约定

- **先量再投**：第 0 步的数字是第 2 步性能项（回填内存、通知矩阵）是否必要的判据；第 0 步不完成，不轻易投第 2 步的性能优化。
- **分步实现**：每一步是独立可交付的最小单元；实现某一项时，从调研报告对应小节（D1/D2/D3/D7/D10、§2/§4）取一手来源与验收口径。
- **实现驱动**：如需任务级拆解，参照 [`2026-08-12-post-m10-feature-roadmap.md`](2026-08-12-post-m10-feature-roadmap.md) 的 checkbox 风格，为被点名的项展开成专属 plan 后再编码。
