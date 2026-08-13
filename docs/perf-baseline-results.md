# Sleipnir 性能基准：实测结果

| Field | Value |
|-------|-------|
| **Date** | 2026-08-13 |
| **机器** | Apple M3 Pro（arm64，6P+6E，12 核），36 GB RAM，macOS 26.6 |
| **被测二进制** | `target/release/sleipnir`（profile：`codegen-units=1` + thin LTO）与 `target/debug/sleipnir` |
| **仿真后端** | `alacritty_terminal` v0.26.1-dev（zed fork `4c129667ce`）—— Sleipnir 的解析/仿真核心 |
| **方法文档** | `docs/perf-baseline.md` · runbook：`scripts/bench/README.md` |

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
- 竞品参照（⚠️ Mitchell 在他机器上测，**非同机**，只看量级）：Ghostty nightly 575 ms < **Sleipnir ≈1.11 s** < Alacritty 1.2 s < kitty 1.7 s。同机公平对比需在本机跑 Ghostty/kitty/Alacritty 再测一次。

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

---

## 3. 下一步

- **同机公平对比**：在**同一台机器**上装 Ghostty/kitty/Alacritty，用同一份语料跑 `time cat`（无重定向），才能消除「Mitchell 的机器」这个变量，得到可直接对比的表格。
- **输入延迟**：上 typometer 或硬件，这是「手感快」的关键指标，目前仍是空白。
- **回填内存增长**：把 `max_scroll_history_lines` 调到 1 万/5 万各测一次 RSS 增量，对比 Ghostty 的字节预算+压缩。

---

## 4. 复现命令

```bash
# 生成语料
scripts/bench/gen-corpus.sh

# 端到端吞吐（在 Sleipnir 窗口里，不要加 > /dev/null）
time cat scripts/bench/corpus/bench-ascii.txt

# 解析吞吐（无头，可自动）
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-ascii.txt
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-unicode.txt

# IO 地板（对照）
time cat scripts/bench/corpus/bench-ascii.txt > /dev/null
```
