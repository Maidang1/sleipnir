# Sleipnir 性能基准

怎么跑吞吐 / 回填内存 / 输入延迟 / 启动时间。实测数字见 [`results.md`](results.md)。

## 0. 生成语料

```bash
scripts/bench/gen-corpus.sh
# 产物：scripts/bench/corpus/bench-ascii.txt（~150 MiB）、bench-unicode.txt（~16 MiB）
# 用 BENCH_DIR=/path 覆盖输出目录；用 ASCII_LINES/UNICODE_LINES 覆盖行数
```

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

## 5. 待核查清单（非性能，但顺手）

| 项 | 命令 | 判断标准 |
|---|---|---|
| ZWJ 宽度 | `printf '🧑‍🌾\n'` 然后框选 | 数它占 2 还是 5 个 cell；Ghostty 默认 `unicode` 是 2 |
| Quick Terminal 是否全局热键 | 切到别的 App 按 `⌘⇧N` | 能唤起 Sleipnir 下拉窗 = 全局；只有 Sleipnir 前台才响应 = 应用内 |

结果记到 [`results.md`](results.md)。
