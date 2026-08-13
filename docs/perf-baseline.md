# Sleipnir 性能基线

| Field | Value |
|-------|-------|
| **Date** | 2026-08-13（方法论定稿） |
| **Status** | 工具链 + 方法就绪；可自动化的数字已实测 → 见 [`perf-baseline-results.md`](perf-baseline-results.md)；渲染吞吐/延迟/回填内存增长仍需人在 GUI 前跑（延迟另需硬件） |
| **Companion** | `scripts/bench/README.md`（runbook）· `scripts/bench/gen-corpus.sh`（语料生成）· `docs/competitive-research-ux-performance.md`（缺口调研） |
| **Goal** | 给「我们快」一个可引用的数字；对标 Ghostty/kitty/Alacritty 的一手数据，并明确哪些是实测、哪些是证据缺口 |

---

## 0. 为什么先做这个

调研的**首要缺口**是：没有任何一手来源能查到 Sleipnir 的延迟/吞吐/内存数字。一个主打「fast」的终端没有可引用 benchmark，就无法在 Ghostty（`cat` 150 MB ≈ 575 ms）、kitty（官方 3 ms `input_delay`）面前建立信任。本文件定义「怎么测、跟谁比、数字填哪」，并诚实标注：**渲染吞吐 / 输入延迟 / 滚动回填内存这三项必须人在 GUI 前测，输入延迟还需硬件；我无法代替你测，也不伪造数字。**

---

## 1. 方法总则（口径）

1. **一次只变一个变量**：同一份语料、同一台机器、同一 `max_scroll_history_lines` 取值，跨终端对比才有意义。
2. **「cat 150 MB」是端到端吞吐、不含渲染**：Mitchell 原话「This is just an IO throughput test. No renderers involved.」——它测的是「把数据吃进仿真/滚动回填」的速度，不是帧率。
3. **滚动回填保留策略会污染吞吐**：默认回填越小的终端越可能「提前丢行 → 更快跑完」。所以每个吞吐数据都必须注明当时的 scrollback 上限。
4. **延迟分软件/硬件两种口径，量级不可混用**：kitty 官方「7 ms vs 30 ms」（typometer）与 Hume 硬件「kitty 36 / alacritty 50 ms」相差 ~5×，两者都指向 kitty 更快，但不能直接比大小。
5. **竞品参照一律采一手、带日期**，且只作「标杆」不作「结论」——各家的「最快」声明工作负载不同（vtebench / 解析器-only / 端到端 cat），无共同基准。

---

## 2. 测试项与结果表

### B1 吞吐（端到端 `cat`）— **待测，人在 GUI 前**

命令（详见 runbook §1）：

```bash
time cat scripts/bench/corpus/bench-ascii.txt
time cat scripts/bench/corpus/bench-unicode.txt
```

> ⚠️ **不能加 `> /dev/null`**：加了就把输出导去空设备、终端收不到，测的是文件读取而非终端吞吐。必须让输出真正灌进终端。

**竞品参照（150 MB ASCII，`cat`，来源 [Mitchell 2026-07-06](https://hachyderm.io/@mitchellh/116873952162192565)）：**

| 终端 | ASCII | Unicode | 备注 |
|---|---|---|---|
| Ghostty nightly | **575 ms** | 536 ms | 字节预算 50 MB + 压缩 |
| Ghostty 1.3.2 | 1.5 s | — | |
| Alacritty | 1.2 s | 1.05 s | |
| kitty | 1.7 s | 1.35 s | |
| Warp | 3.8 s | — | |
| iTerm2 / Terminal.app | >60 s（中止） | — | |

**Sleipnir（已测 2026-08-13，本机 M3 Pro，默认 scrollback=10000）：**

| 语料 | real | user | sys | 备注 |
|---|---|---|---|---|
| ASCII 150 MB | **1.091 / 1.130 s** | 0.00 s | 0.47 s | ≈144/139 MB/s，两次可复现 |
| Unicode 18.5 MB | **0.179 s** | | | ≈109 MB/s；详见 `perf-baseline-results.md` §1.1 |

**系统地板（本机实测，非 Sleipnir 数字，用于对照）：**

| 命令 | real |
|---|---|
| `cat bench-ascii.txt > /dev/null` | **0.02–0.08 s**（warm cache，见 §5） |

### B2 解析吞吐（可选，对应 kitty parser-only）

kitty 官方 `kitten __benchmark__`（**渲染被抑制**，2023）：kitty 134.55 / gnome-terminal 61.83 / alacritty 54.05 / wezterm 48.5 / xterm 30.72 MB/s（[来源](https://sw.kovidgoyal.net/kitty/performance.html)）。

> Sleipnir 复用 `alacritty_terminal` 仿真层，解析吞吐与 Alacritty 同源。已封装成无头 bench：`cargo run --release -p parse_bench -- <file>`（`crates/parse_bench/`，直接驱动 `alacritty_terminal`，不依赖 GPUI）。本机实测 ASCII **170.95 MB/s**、Unicode **117.72 MB/s**，详见 `perf-baseline-results.md` §1.2。

### B3 输入延迟 — **待测，需硬件/摄像头**

竞品参照（Hume 硬件，[2020-05-20](https://thume.ca/2020/05/20/making-a-latency-tester/)）：kitty **36.1 ms**、Apple Terminal 35.8、alacritty **50.4**、iTerm2 无 GPU 50.5 / GPU 53.1。

| 终端 | 延迟 | 方法 |
|---|---|---|
| Sleipnir | _____ | typometer / 硬件 |
| 对照（任选） | | 同法 |

### B4 滚动回填内存 — **待测，人在 GUI 前**

竞品默认（一手）：Ghostty 字节预算（10→50 MB）+ 自动压缩省 70–90% 物理内存；kitty 2000 行；WezTerm 3500 行；Windows Terminal `historySize` 9001；**Sleipnir 10000 行**（`crates/sleipnir_settings/src/sleipnir_settings.rs:225`，纯内存、无压缩）。

| `max_scroll_history_lines` | 空闲 RSS | 灌满 RSS | 增量 |
|---|---|---|---|
| 10000（默认） | | | |
| 50000 | | | |

### B5 启动时间 — **待测，可半自动**

全行业**无**一手启动 benchmark（性能调研明确「No startup benchmarks at all, any terminal」）。测出来即是填补空白。

| 二进制 | 冷启动 | 热启动 |
|---|---|---|
| `target/debug/sleipnir` | | |
| release `.app` | | |

### B6 缩放 / reflow

Sleipnir 的差异点是 **divider 拖动实时 PTY reflow**（`docs/adr/0003`）——这是相对 iTerm2/Ghostty「从超长回填从头 reflow」的**优势**。窗口 resize / 字号热变成本全行业无量化数据，暂不建表，仅作主观「拖 divider 是否跟手」检查项。

---

## 3. 结果汇总（发布用）

| 指标 | Sleipnir | Ghostty | kitty | Alacritty | iTerm2 |
|---|---|---|---|---|---|
| 吞吐 150 MB | **1.091 s**（本机实测） | 575 ms | 1.7 s | 1.2 s | >60 s |
| 输入延迟 | 待测 | ~? | 36 ms | 50 ms | 50–53 ms |
| 回填内存策略 | 10000 行纯内存 | 字节预算+压缩 | 2000 行 | 10000 行例 | 无限 |
| 启动 | spawn→会话恢复 <1s（窗口出现待秒表） | — | — | — | — |

> ⚠️ 竞品列（Ghostty/kitty/Alacritty/iTerm2）来自 Mitchell/维护者**各自机器**的测量，与 Sleipnir 列**非同机**，只能看量级；同机公平对比见 `perf-baseline-results.md` §3。

---

## 4. 待核查清单（非性能，但影响结论）

| 项 | 方法 | 结果 |
|---|---|---|
| 🧑‍🌾 ZWJ 序列占几个 cell（vs Ghostty `unicode`=2） | `printf '🧑‍🌾\n'` 后框选 | |
| Quick Terminal `⌘⇧N` 是否全局热键（失焦可唤起） | 切到别 App 按 `⌘⇧N` | |

---

## 5. 当前已实测的系统地板（本机）

> 以下为「文件读入 / 纯 IO」地板，**不是 Sleipnir 数字**；用于说明任何终端的吞吐都不可能低于它。

| 命令 | real | user | sys |
|---|---|---|---|
| `cat scripts/bench/corpus/bench-ascii.txt > /dev/null` | **0.02–0.08 s** | ~0.00 | 0.02–0.05 |

> 2026-08-13 本机实测（3 次：0.02 / 0.02 / 0.08 s）。**warm cache**（语料刚生成、页缓存命中），约 7 GB/s；冷启动从磁盘读会明显更慢。语料字节数：ASCII 157,290,000（≈150 MiB）、Unicode 19,440,000（≈18.5 MiB）。此值为「任何终端吞吐都不可能低于」的地板，**不是 Sleipnir 数字**。
