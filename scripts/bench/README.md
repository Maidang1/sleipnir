# Sleipnir 性能基准

怎么跑吞吐 / 回填内存 / 输入延迟 / 启动时间。实测数字见 [`results.md`](results.md)。

## 0. 生成语料

```bash
scripts/bench/gen-corpus.sh
# 产物：scripts/bench/corpus/bench-ascii.txt（~150 MiB）、bench-unicode.txt（~16 MiB）、
#       bench-agent.txt（~30 MiB，重绘密集）
# 用 BENCH_DIR=/path 覆盖输出目录；用 ASCII_LINES/UNICODE_LINES/AGENT_CYCLES 覆盖规模
```

三份语料测的**不是**同一件事：

| 语料 | 形态 | 压的是 |
|---|---|---|
| `bench-ascii` | 同一行纯 ASCII 重复 147 万次 | 解析吞吐上限（对标 Mitchell / kitty 的口径） |
| `bench-unicode` | CJK / ZWJ / emoji | UTF-8 解码 + 宽字符 |
| `bench-agent` | `\r` + `ESC[K` + 光标上移原地重绘 + SGR 频繁切换 + 偶发 4–16 KiB 超长行 | **主用户场景**：终端里跑 coding agent。字节少但重绘多，瓶颈在 damage/repaint，不在解析 |

> ⚠️ `bench-agent.txt` 只还原 agent 输出的**内容形态**。它还原不了**写入形态**（几十到几百字节、间隔几毫秒的小块 flush），因为 `cat` 恒定按 64 KiB 读。写入形态用 [`agent-stream.sh`](agent-stream.sh)（见 §5）。
> 语料生成用固定 `srand`，同一份语料在任何机器上字节一致（`shasum` 可核）。

## 1. 吞吐（必须人在 GUI 前，逐个终端手测）

**测法**：在 Sleipnir 里开一个干净 tab，确认 `max_scroll_history_lines` 足够大（或临时改大，见 §3），然后：

```bash
time cat /path/to/scripts/bench/corpus/bench-ascii.txt
time cat /path/to/scripts/bench/corpus/bench-unicode.txt
```

> ⚠️ **绝不能加 `> /dev/null`**：那会把输出导去空设备，终端根本收不到数据，测到的只是「读文件」地板（~几十 ms），不是终端吞吐。B1 就是要让 cat 的输出**真正灌进终端**，由终端把 PTY 排空。

再在对照终端（Terminal.app / iTerm2 / kitty / Ghostty 任选）跑同样的命令。把三个数字（real/user/sys）记进 [`results.md`](results.md)。

> ⚠️ 口径：这是「把整份数据吃进仿真/滚动回填」的端到端耗时，**不含渲染**（对应 Mitchell 的原话「No renderers involved」）。结果会被滚动回填保留策略污染——若终端提前丢行，会更快「跑完」。所以务必记录当时 `max_scroll_history_lines` 的取值。

## 2. 滚动回填内存（人在 GUI 前）

1. 冷启动 Sleipnir，记下空闲 RSS（Activity Monitor 或 `ps -o rss= -p $(pgrep -x sleipnir)`）。
2. 跑一次 `time cat .../bench-ascii.txt`（灌满回填）。
3. 记下灌满后的 RSS。
4. 改 `max_scroll_history_lines` 为 10000 / 50000 / 无限 各测一次，填 B4 表。

## 3. 输入延迟（需要额外工具/硬件）

- **软件（typometer，需摄像头）**：<https://github.com/pavelfatin/typometer>
- **硬件（Teensy + 光敏电阻，最准）**：<https://thume.ca/2020/05/20/making-a-latency-tester/>

对标竞品：kitty ~36 ms、alacritty ~50 ms、iTerm2 ~50–53 ms（Hume 2020 硬件测量）。

## 4. 启动时间（半自动）

> GUI 应用不会自己退出，`hyperfine` 直接跑会挂住。用下面两种之一。

**A. 秒表（最直观，测「窗口出现」）**：`pkill -x sleipnir 2>/dev/null; open -a Sleipnir` → 掐表到窗口出现。

**B. 进程级计时（spawn → 进程存活，不含窗口绘制）**：

```bash
pkill -x sleipnir 2>/dev/null; sleep 1
perl -MTime::HiRes=time -e '
  my $s = time;
  system("open", "-a", "Sleipnir");
  while (system("pgrep -qx sleipnir") != 0) { select undef, undef, undef, 0.05 }
  printf "process-alive: %.2fs\n", time - $s'
```

> 注意：公开资料里几乎没有启动时间的一手 benchmark，这里测出来本身就是填补空白的数字。

## 5. Agent 场景（B5，主用户场景，必测）

这是唯一直接对标 Sleipnir 定位（**给在终端里跑 agent 的人用**）的一组指标。

### 5a 重绘密集吞吐（可自动）

```bash
# 在 Sleipnir 窗口里（不要加 > /dev/null）
time cat scripts/bench/corpus/bench-agent.txt
# 纯解析对照（无头）
cargo run --release -p parse_bench -- scripts/bench/corpus/bench-agent.txt
```

口径：这份语料的 **MB/s 会显著低于 ASCII**，这是正常的——每字节携带的仿真工作量（清行、光标寻址、SGR 状态切换）高得多。**不要拿它和 ASCII 的 MB/s 比大小**，它只和自己的历史值比（防回归）。

### 5b 写入形态（在 Sleipnir 里跑）

```bash
scripts/bench/agent-stream.sh 30 4      # 30 秒，每 4ms 一次小块 flush
# 可调：CHUNK_MIN / CHUNK_MAX（默认 32 / 256 字节）
```

4ms 是故意选的：PTY 事件循环的合并窗口就是 4ms（`crates/terminal/src/terminal.rs`，首个事件立即处理、其后 4ms 批量 flush），所以「每 4ms 一次写」是最坏的现实情况——每个批次只有一次写，合并拿不到任何好处。

### 5c 后台 pane 不得拖累窗口（回归检查）

 agent 常常在一个 tab 里跑很久，人在另一个 tab 打字。off-screen 的 pane **绝不允许**驱动窗口重绘。

```bash
scripts/bench/measure-cpu.sh 10           # ① idle 基线
# Sleipnir tab A：scripts/bench/agent-stream.sh 120
# 切到 tab B，让流在后台继续
scripts/bench/measure-cpu.sh 10           # ② 后台代价
# 切回 tab A（流可见）
scripts/bench/measure-cpu.sh 10           # ③ 前台代价
```

**通过条件**：② 与 ① 相差在几个百分点内。若 ② 接近 ③，说明有 off-screen pane 在触发重绘 —— 检查 `AppShell::is_pane_visible`（`crates/sleipnir_ui/src/app_shell.rs`），它必须与 `render_content` 的可见性判断（活跃 tab + pane zoom）保持一致。

同样的检查也适用于 **pane zoom**：zoom 时只有一个 leaf 上屏，其余 pane 仍在排 PTY，但不得触发重绘。

---

## 6. 待核查清单（非性能，但顺手）

| 项 | 命令 | 判断标准 |
|---|---|---|
| ZWJ 宽度 | `printf '🧑‍🌾\n'` 然后框选 | 数它占 2 还是 5 个 cell；Ghostty 默认 `unicode` 是 2 |
| Quick Terminal 是否全局热键 | 切到别的 App 按 `⌘⇧N` | 能唤起 Sleipnir 下拉窗 = 全局；只有 Sleipnir 前台才响应 = 应用内 |

结果记到 [`results.md`](results.md)。
