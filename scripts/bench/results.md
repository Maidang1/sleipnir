# Sleipnir 性能基准：实测结果

| Field | Value |
|-------|-------|
| **Date** | 2026-08-13 |
| **机器** | Apple M3 Pro（arm64，6P+6E，12 核），36 GB RAM，macOS 26.6 |
| **被测二进制** | `target/release/sleipnir`（profile：`codegen-units=1` + thin LTO）与 `target/debug/sleipnir` |
| **仿真后端** | `alacritty_terminal` v0.26.1-dev（zed fork `4c129667ce`）—— Sleipnir 的解析/仿真核心 |
| **方法 / runbook** | [`README.md`](README.md) |

> **口径说明**：「解析吞吐」是无头、渲染抑制的仿真解析速度（对应 kitty `kitten __benchmark__`）；「端到端 `cat` 吞吐」是在 Sleipnir 窗口里让输出真正灌进 PTY 的耗时。两者不是一回事。输入延迟 / 回填内存增长仍需人在 GUI 前跑（延迟另需硬件），如实标注「未测」。

---

## 1. 已实测

### 1.1 端到端 `cat` 吞吐（B1，Sleipnir 窗口内实测）

命令（在 Sleipnir 窗口里，**无 `> /dev/null`**）：`time cat ...`；另用 `scripts/bench/bench-in-shell.sh` 自动测 Unicode + ASCII。

| 语料 | 字节 | real | **吞吐** |
|---|---|---|---|
| ASCII 150 MiB（手动） | 157,290,000 | 1.091 s | 144 MB/s |
| ASCII 150 MiB（脚本） | 157,290,000 | 1.130 s | 139 MB/s |
| Unicode 18.5 MiB（脚本） | 19,440,000 | 0.179 s | **109 MB/s** |

**解读：**
- ASCII 两次 1.091 / 1.130 s（≈1.11 s），**可复现**；Unicode 0.179 s ≈ 109 MB/s，比 ASCII 慢 ~22%，是 UTF-8 解码 + 宽字符/ZWJ 的开销。
- cat 自身 0.00s user / 0.47s system、43% CPU → 其余 ~57% 时间在**阻塞等终端排空 PTY**，正是端到端吞吐的特征。
- 对比 §1.2 纯解析 0.92s，端到端多出 ~0.17s，即 PTY 排水 + 系统调用的开销，内部自洽。
- ~~竞品参照（⚠️ Mitchell 在他机器上测，**非同机**，只看量级）：Ghostty nightly 575 ms < **Sleipnir ≈1.11 s** < Alacritty 1.2 s < kitty 1.7 s。~~
  → **这条跳机器对比已被 §1.6 的同机实测推翻：同一台 M3 Pro 上 Sleipnir 比 Ghostty 快 ~20%，不是慢一倍。**
  拿别人机器的数字当对标结论，结论可以正好反了——这就是同机对比必须做的原因。

### 1.2b 解析吞吐重测（n=3，2026-08-15）

| 语料 | 3 次实测 | 与 §1.2 的单次值对比 |
|---|---|---|
| ASCII | 198.3 / 204.0 / 203.3 MB/s | 170.95 → **+19%** |
| Unicode | 184.3 / 183.9 / 184.4 MB/s | 117.72 → **+57%** |
| Agent | 174.0 / 161.2 / 163.8 MB/s | 新增 |

同一台机器、同一份语料（字节数完全一致）、同一个 pin。**§1.2 的单次数字今天复现不出来**，
差距大到（Unicode +57%）不可能是真实优化。结论：**单次量测不可信，任何数字至少 n=3 并记
区间**；历史单次值只能当量级参考，不能当回归基线。

### 1.2 解析吞吐（parser-only，渲染抑制，无头）

命令：`cargo run --release -p parse_bench -- <corpus>`（120 列 × 40 行，scrollback=10000，64 KiB 分块）

| 语料 | 字节 | 耗时 | **吞吐** |
|---|---|---|---|
| ASCII（150 MiB） | 157,290,000 | 0.9201 s | **170.95 MB/s（163 MiB/s）** |
| Unicode/CJK/emoji（18.5 MiB） | 19,440,000 | 0.1651 s | **117.72 MB/s（112 MiB/s）** |

**解读：** 这是 Sleipnir 仿真核心的上限；Unicode 更慢（UTF-8 解码 + 宽字符/ZWJ）。**不可**直接与 kitty 官方 2023 表（kitty 134.55 / alacritty 54.05 MB/s）比大小——不同机器/版本/语料。

### 1.3 空闲内存（RSS）

启动后 3s 采样；两者均**恢复了 3 个 tab（3 个子 shell）**。

| 二进制 | RSS | VSZ | %MEM |
|---|---|---|---|
| release | **~98 MB**（100,336 KB） | ~435 MB | 0.3% |
| debug | ~108 MB（110,688 KB） | ~435 MB | 0.3% |

### 1.4 系统 IO 地板（对照，非 Sleipnir）

| 命令 | real |
|---|---|
| `cat 150MiB > /dev/null` | **0.02–0.08 s**（warm cache） |

任何终端的吞吐都不可能低于此地板（~7 GB/s）。你之前跑出的 0.041s 就是这条地板。

### 1.6 同机竞品对比（B1/B5a，**2026-08-15 实测**，全自动）

工具：[`drain-compare.sh`](drain-compare.sh)。方法：启动终端 → 找到它 spawn 的 shell 的 pty →
把语料**直接写进该 pty 的 slave**（`cat corpus > /dev/ttysNNN`）。对终端而言这与窗口里的
程序输出完全等价，写入会阻塞到仿真器排空 master，所以 wall time 就是它的 ingest 耗时。

为什么不用手敲 `time cat`：敲键盘需要 Accessibility 授权（自动化拿不到），而且 shell 自己的
prompt/OSC 开销会混进去。写 pty 对每个被测终端完全一致，这才是公平的前提。

| 终端 | ASCII 150 MiB | Unicode 18.5 MiB | **Agent 29 MiB** |
|---|---|---|---|
| **Sleipnir** (v2) | **1.18 s / 134 MB/s** | 0.19 s / 102 MB/s | **0.31 s / 99 MB/s** |
| Alacritty 0.16 | 1.15 s / 137 MB/s | 0.19 s / 100 MB/s | 0.28 s / 110 MB/s |
| Ghostty 1.2 | 1.68–1.75 s / 90–94 MB/s | 0.22–0.23 s / 83–87 MB/s | 0.39–0.42 s / 73–80 MB/s |
| kitty 0.44 | 1.69–1.74 s / 90–93 MB/s | 0.20–0.21 s / 92–97 MB/s | 0.39–0.41 s / 76–78 MB/s |

n=2–3 per cell（Sleipnir ASCII 首次 1.42 s 后稳定在 1.18 s，取中位数）。滚动回填：kitty/Alacritty
强制 10000 行，Sleipnir 用默认 `max_scroll_history_lines`=10000；**Ghostty 按字节而不是行做预算，
无法精确对齐**，已如实标注而非隐匿。

**结论：**
1. **“比 Ghostty 慢一倍”是错的。** 同机上 Sleipnir 在三份语料上均比 Ghostty **快 ~20–30%**。
   旧结论来自跟別人机器的数字相减（§1.1）。
2. **Alacritty 仍领先一个身位**（agent 语料 110 vs 99 MB/s，快 ~11%）。它没有 tab/split/chrome，
   这是一个诚实的参照上限，不是缺陷。
3. **优势在 agent 语料上最大**（vs Ghostty/kitty 快 ~27%）——正好是主用户场景。

### 1.5 启动时间（部分）

- 进程 spawn → 存活：瞬时（<50 ms）。
- 「restored session: 3 tab(s)」日志与启动同一秒出现（session 恢复 <1s）。
- **窗口出现时间未测得**：裸二进制未被 LaunchServices 注册，`System Events` 查不到（osascript -1728）。需秒表，见 runbook §4。

---

## 2. 未测（必须人在 GUI 前 / 需硬件）

| 项 | 为何不能自动测 | 方法 |
|---|---|---|
| **B3 输入延迟** | 需 typometer（摄像头）或 Teensy 硬件 | runbook §3；对标 kitty 36ms / alacritty 50ms |
| **B4 回填内存增长** | 要冷启动 GUI + 灌满回填 + 看 RSS 增量 | runbook §2 |
| **窗口出现时间** | 需眼睛/秒表 | runbook §4 |
| **B5a 重绘密集吞吐** | 端到端那半要人在 GUI 前（`parse_bench` 那半可自动） | runbook §5a，语料 `bench-agent.txt` |
| **B5b agent 写入形态** | 要在 Sleipnir 的 pane 里跑 | runbook §5b，`agent-stream.sh` |
| **B5c 后台 pane 重绘代价** | 要人切 tab | runbook §5c，`measure-cpu.sh` |

> **B5 是主用户场景（在终端里跑 coding agent），优先级高于 B1 的 ASCII 吞吐。**
> 已有的 B1/B2 语料（同一行纯 ASCII 重复 147 万次）测的是解析上限，**完全测不到** agent 的
> 真实负载：小块高频写入 + `\r`/`ESC[K` 原地重绘 + 光标寻址 + SGR churn。这三项测出来之前，
> 「agent 跑起来很流畅」是断言，不是事实。

---

## 3. 下一步（按优先级）

1. **B5c 后台 pane 重绘**（无需硬件，10 分钟）：`is_pane_visible` 的修复已落地（见 §5），**需要实测确认**：idle / 后台流 / 前台流 三段 CPU，按 runbook §5c 记进本文件。
2. **B5a + B5b agent 语料与写入形态**（无需硬件）：这是唯一对标定位的指标组。
3. **同机公平对比**：在**同一台机器**上装 Ghostty/kitty/Alacritty，用同一份语料跑 `time cat`（无重定向），才能消除「Mitchell 的机器」这个变量。**在这张表出来之前，README 里的性能主张（"fast"）没有同机证据支撑。**
4. **输入延迟**：上 typometer 或硬件，这是「手感快」的关键指标，目前仍是空白。可先加软件段打点（keydown → 帧提交）当回归闸门，但必须标注它不含 compositor + 显示器的 1–2 帧。
5. **回填内存增长**：把 `max_scroll_history_lines` 调到 1 万/5 万各测一次 RSS 增量，对比 Ghostty 的字节预算+压缩。

---

## 4. 已修复的性能缺陷（需回归实测）

### 4.1 后台 tab / 被 zoom 遮住的 pane 会驱动整窗重绘

**症状**：`AppShell::wire_term_view` 的 `cx.observe` 只判断 pane「属于本窗口」，不判断它是否**上屏**。
于是后台 tab 里跑 agent 时，每个 4 ms 的 PTY 批次都让整个窗口 `notify()` 一次
（≈250 次/秒），与用户正在打字的 pane 抢重绘。pane zoom 时被遮住的 pane 同理。

**修复**：新增 `AppShell::is_pane_visible`，只有「在活跃 tab 内」且「（若有 zoom）就是被 zoom 的那个」
的 pane 才能触发 `cx.notify()`。可见性判据抽成纯函数 `pane_is_on_screen`，有单测覆盖。
背景 pane 需要展示的信息（tab 标题、visual bell）走的是低频 `TermViewEvent`，不受影响；
切 tab / 切 zoom 本身都会 `cx.notify()`，所以重新上屏时立刻按当前 grid 全量重绘，不会丢内容。

**待补实测**：runbook §5c 的三段 CPU 数字（本文件 §2 已列为 B5c）。

---

## 5. 复现命令

```bash
# 生成语料
scripts/bench/gen-corpus.sh

# 端到端吞吐（在 Sleipnir 窗口里，不要加 > /dev/null）
time cat scripts/bench/corpus/bench-ascii.txt

# 解析吞吐（无头，可自动）
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-ascii.txt
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-unicode.txt

# agent 场景（B5，主用户场景）
time cat scripts/bench/corpus/bench-agent.txt            # 在 Sleipnir 窗口里
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-agent.txt
scripts/bench/agent-stream.sh 30 4                       # 在 Sleipnir pane 里，小块写入
scripts/bench/measure-cpu.sh 10                          # 外部 shell，采 CPU/RSS

# IO 地板（对照）
time cat scripts/bench/corpus/bench-ascii.txt > /dev/null
```
