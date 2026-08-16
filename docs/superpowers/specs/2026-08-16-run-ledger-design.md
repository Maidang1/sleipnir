# Run Ledger: 从「更快的终端」转向「你可以离开的终端」

**Status:** approved (brainstorm 产出，待实现计划)
**Date:** 2026-08-16

## 1. 命题与边界

### 命题

Sleipnir 是**你可以离开的终端**。人跑 coding agent 和长任务，Sleipnir 负责在你不在的时候
记住发生了什么，并在你回来时如实交还给你。

对外叙事的第一句话从 "A fast, native terminal emulator — GPU-rendered" 改为体验向表述。
性能不再作为卖点，但仍是**底线要求**：GPUI 带来的流畅是入场券，不是差异点。

### 用户

- **主用户**（沿用 [ADR-0008](../../adr/0008-no-builtin-ai.md)）：在终端里跑 coding agent 的人 ——
  人是用户，agent 是负载。本设计把这条定义从「性能取向的注脚」升级为整个体验主线的出发点。
- **次要用户**：跑长构建 / 测试 / 部署的人。旅程形状相同：启动 → 走开 → 回来。

### 设计原则

后续所有细节争议都用这四条裁决：

1. **事实先于装饰** — 只呈现系统已知的事实（命令行、耗时、退出码、pane 归属）。不推断超出
   数据的结论，不生成，不总结。
2. **内容区不可侵犯** — 终端网格保持 VT 纯净。新增视觉元素只活在 chrome、gutter overlay
   和面板里。
3. **默认克制，按需展开** — 干活时不占一寸空间；回来时一键拿到全部。
4. **记住元数据，不记住输出** — 绝不把 scrollback 写盘（[ADR-0006](../../adr/0006-tombstone-session-restore.md) 的红线）。

### 审计依据

本设计来自对一条真实旅程（装 → 首次开 → 起活 → 离开 → 回来 → 找结果 → 重启）的审计。
6 个断点中最疼的三个是「回来」「找结果」「重启」，且它们有共同根因：

> 系统内部已经知道的事实，没有出现在界面上。

证据：

- `crates/terminal/src/terminal.rs` 已有 `looks_busy()`、`busy_since`、
  `foreground_process_command_name()`、`exit_status` / `task_summary()`；
  `crates/sleipnir_ui/src/app_shell.rs` 的 tab chip 只用它渲染标题和铃声闪，**没有状态指示**。
- OSC 133 命令边界已从真 PTY 走通（[ADR-0005](../../adr/0005-vendored-alacritty-term.md) 为此 fork 了 vte），
  `shell_semantics.rs` 甚至已能算出 `command_output_range()`，但屏幕上看不出一次命令的起止、
  耗时、退出码。
- `session.rs` 只存结构；ADR-0006 自己承认「布局在说这里什么都没发生过，而这是假的」，且状态
  仍是 proposed。

### 非目标

- 命令块折叠 / 块级复制（Warp 式 Blocks）—— 明确延后，不是遗漏
- 任何模型调用、自然语言、"解释这个错误"（ADR-0008 禁止）
- 跨机器同步台账、workflow / snippet 库、团队协作
- 把性能当卖点讲

### 成功标准

1. 回到窗口后**不切 tab** 就能知道哪个 pane 需要我（目标 < 2 秒）。
2. 通知消失后信息仍在：半小时前那次失败仍可查、仍可跳回。
3. 重启后能回答「上次最后跑的是什么、结果如何」。
4. `run_ledger: off` 时行为与本设计实现前**完全一致** —— 整条功能可回退。

## 2. 领域模型

新增术语，写入 `docs/glossary.md`（沿用其「避免」体例）：

**Run（运行）**：一次命令执行，从 OSC 133 `C`（开始执行）到 `D`（结束，带退出码）。恰好属于
一个 Pane。字段：命令行文本（脱敏后）、cwd、开始时间（wall clock）、耗时（单调时钟）、
退出码、状态、来源（window / tab / pane id）、Anchor、是否推断、是否已看过。
_避免_：Task（本仓库已用于 Zed 派生的 `SpawnInTerminal` / `TaskState`）、Command（命令是文本，
Run 是一次执行）、Block、Job。

**Ledger（台账）**：全部 Run 的有序集合，跨 Window / Tab / Pane，进程内唯一，并持久化到磁盘。
是唯一事实来源；UI 只读它的快照。
_避免_：History（shell history 是命令文本历史）、Log、Timeline。

**Anchor（锚点）**：Run 在所属 Pane scrollback 中的位置，复用 `Osc133Marker.line/column` 与
`command_output_range()`，用于「跳回那次输出」。**只在进程生命周期内有效** —— 重启后
scrollback 不存在，锚点随之失效。
_避免_：Mark（Mark 指裸的 OSC 133 标记）、Bookmark。

**Attention（待看）**：已结束但用户还没看过的 Run 集合。「看过」只有两种定义：focus 了那个
Pane，或在 Ledger 面板里点过它。Attention 驱动徽标与通知，是本设计的核心概念。
_避免_：Unread、Badge（badge 是它的一种渲染）、Alert。

### Run 状态机

**主路径（有 OSC 133）**

```
Idle --133 B--> Composing --133 C（读取 B..C 之间的网格文本作为命令行）--> Running
Running --133 D status=0--> Succeeded
Running --133 D status≠0--> Failed
Running --133 D 无 status--> Unknown
```

**降级路径（无 OSC 133：ssh 远端、未注入的 shell、`shell -c`）**

```
busy_since + foreground_process_command_name --> Running（推断）--前台进程消失--> Unknown
```

命令行仅为进程名（如 `ssh` / `vim`），无退出码。台账中明确标注「推断」，不冒充精确值。

**异常终止**

```
Running --pane 关闭 / 进程被杀 / 应用退出--> Abandoned
```

Abandoned 在重启后显示为「未完成」，**永不显示成成功**。

### 数据流

```
PTY 字节
  → vte fork（osc_dispatch，ADR-0005）
  → terminal crate（Osc133Scanner / busy 探测，均已存在）
  → RunEvent { Started, Finished, Abandoned }（新增窄接口）
  → run_ledger crate（纯数据 + 状态机，不依赖 gpui）
  → 三个消费者：Chrome 徽标 / Ledger 面板 / 持久化 writer
```

### 架构决策

1. **`run_ledger` 是独立 crate，不依赖 gpui。** 状态机、脱敏、保留策略、排序、Attention 计算
   全为纯函数与纯数据结构，可用普通单测覆盖，无需跑 UI。`terminal` 只负责发 `RunEvent` ——
   它不知道 Ledger 存在；UI 只读快照。
2. **顺手拆 `app_shell.rs`。** 该文件已 4150 行（chrome 渲染、tab 拖拽、快捷键、rename、
   session 混在一起）。本设计要往 chrome 和面板加东西，不拆会直奔 5000 行。范围**严格限定
   为本次要碰的部分**：tab strip 渲染独立成模块、Ledger 面板独立成模块。不做无关重构。

## 3. 界面

四个可见面，全部位于 chrome / overlay 层，终端网格不变。

### 3.1 Tab / Pane 状态徽标（宏观：谁需要我）

- 四态：**● 在跑**（带 `mm:ss` 计时，tabular numerals）/ **✓ 完成未看** / **✗ 失败未看** /
  无徽标（没有待看）。
- **看过即淡出** —— 徽标是 Attention 的投影，不是永久装饰。
- 多 pane 的 tab 按 **失败 > 在跑 > 成功** 聚合，显示最紧急的一个 + 计数（`✗2`）。
- 非活动 pane 在自身角落显示同一个点（复用已有 unfocused dim 机制）。
- Dock / 任务栏 badge = 全部窗口的待看失败数 —— **P2**。

### 3.2 Pane gutter 标记（微观：刚才发生了什么）

- 命令起始行画一个小三角，颜色 = 结果。**绘制在 overlay 层，不占字符网格宽度** ——
  因此不影响选区复制。
- hover 出 tooltip：命令行 + 耗时 + 退出码；点击 = 在 Ledger 面板中定位该 Run。
- 进入 alt screen（vim / 全屏 TUI）时整列隐藏 —— 那里没有 prompt 概念。

### 3.3 Ledger 面板

- 唤起：`⌘⇧L`（macOS）/ `Ctrl+Shift+J`（Windows/Linux）。两者在当前 `keymap.rs` 中均未占用。
- **形态：浮层**，自右侧覆盖滑入，宽约 290–320px，**不挤压终端布局、不触发 PTY reflow**。
  「钉住为常驻侧栏」列为 **P2**，待台账价值被验证后再评估 reflow 复杂度。
- 内容：跨全部窗口；一行一条 Run（状态图标、命令行截断、来源 window/tab/pane、耗时、
  相对时间）；分组为 **进行中 / 待看 / 今天 / 更早**。
- 交互：`↑↓` 选择、`Enter` 跳转（激活对应 window + tab + pane 并滚到 Anchor，跳转即算已看过）、
  `Esc` 关闭；type-to-filter 沿用主题选择器的过滤模式。
- 推断出的 Run 打「推断」标；重启前的记录标「锚点已失效」，灰显、不可跳转，命令行可复制。
- 同时进命令面板与应用菜单；新增 action `toggle_run_ledger` 与 `clear_run_ledger`。

### 3.4 Tombstone

重启后每个恢复的 pane 顶部显示一行灰色只读横幅：

> 上次这里跑过 5 条命令，最后一条 `npm test` 失败（exit 1 · 昨天 23:41） 　查看台账 ⌘⇧L

打字即消失。这落地了 ADR-0006 的意图，但使用**元数据而非 scrollback**，因此不踩其凭据泄露
红线。ADR-0006 状态从 proposed 改为 accepted。

### 3.5 通知重构

现状：5 秒阈值 + 未聚焦 → 系统通知，消失即信息丢失。

改为：**通知只是 Attention 的一种渲染** —— 点通知直接跳到那条 Run；错过通知也不丢信息
（徽标和台账都还在）。`notify_on_command_finish_secs` / `_mode` 两个设置保持不变，语义变为
「什么时候顺便弹一条」。

## 4. 持久化与隐私

| 项 | 决定 |
|---|---|
| 位置 | `~/.config/sleipnir/runs.json`（Windows `%APPDATA%\sleipnir\runs.json`），与 `session.json` 同级 |
| 格式 | 带 `version` 字段的 JSON，沿用 `SESSION_VERSION` 模式；原子写（temp + rename） |
| 写入时机 | Run 结束后去抖 ~2s 落盘；应用退出时落盘 |
| 上限 | 双约束：**7 天** 且 **最多 500 条**，先到先裁 |
| 权限 | Unix 下 `0600` |
| 绝不存储 | scrollback、任何输出内容、环境变量值 |

### 脱敏

写盘前执行，纯函数位于 `run_ledger::redact`：

1. `KEY=VALUE` 前缀 → 保留 KEY，值替换为 `…`（`AWS_SECRET_ACCESS_KEY=…`）
2. 已知敏感 flag（`--token` / `--password` / `--api-key` / `--secret` / `-H Authorization:`）→ 值替换
3. 高熵长串（>20 字符的 base64 / hex 样貌）→ 替换
4. URL 中的 `user:pass@` 与 `token=` / `key=` / `sig=` query → 替换
5. 命令行超过 256 字符截断

**诚实性声明**：脱敏是启发式，不是保证。文档必须如此表述，不得暗示"安全"。

### 默认值（含破坏性变更）

- **持久化默认开。** 依据：zsh / bash 本来就把完整命令行明文写进 `~/.zsh_history`，本设计
  写入的是其脱敏子集 + 元数据，**风险不高于现状**；而 tombstone 与「跨重启还记得」必须默认
  可见才有价值。配套要求（三者缺一不可）：首次写入时明确告知一次、设置中一眼可关、
  `clear_run_ledger` 一键清空。
- **`terminal.inject_osc133` 默认值从 `false` 改为 `true`。** 这是台账质量的前提（不注入就只有
  推断路径）。现有逻辑已会在「其他终端已注入」时跳过。保留 `false` 逃生口，并在 CHANGELOG
  中标记为**破坏性默认值变更**。

### 设置键

| 键 | 取值 | 默认 |
|---|---|---|
| `run_ledger` | `off` / `memory` / `persist` | `persist` |
| `run_ledger_retention_days` | 整数 | `7` |
| `run_ledger_max_runs` | 整数 | `500` |
| `run_ledger_redact` | bool | `true` |
| `terminal.inject_osc133` | bool | `true`（**变更**） |

## 5. 降级与错误处理

每种情况都有确定行为，不留未定义状态。

| 情况 | 行为 |
|---|---|
| 无 OSC 133（ssh 远端 / 未注入 shell / `shell -c`） | 走推断路径，标「推断」，无退出码 |
| B..C 之间读不出命令行（多行输入、粘贴大段） | 存「(无法识别的命令)」，时间与退出码照存；多行取首行 + `…` |
| `runs.json` 损坏 / 版本不认 | 重命名为 `.bak`，从空台账启动，**绝不阻塞启动** |
| 磁盘写失败 | 降级为内存台账 + 一次性提示，不崩 |
| 休眠 / 时钟回跳 | 耗时用单调时钟计算；落盘只存 wall-clock 起始时间 |
| `run_ledger: off` | 徽标、gutter、面板、tombstone 全部消失，行为与实现前一致 |

## 6. 测试策略

- **`run_ledger` 纯单测**（无需 UI）：状态机三条路径 + marker 乱序 / 缺失；**脱敏黄金样例集**
  （`AWS_SECRET=x cmd`、`curl -H "Authorization: Bearer …"`、`https://u:p@host?token=…`）；
  保留裁剪（时间窗与条数双约束）；Attention 计算；损坏文件恢复。
- **`terminal`**：喂字节流 → 断言 `RunEvent` 序列（扩展现有 `osc133.rs` 单测）。
- **UI**：徽标聚合规则（失败 > 在跑 > 成功 + 计数）抽为纯函数并单测；面板不做快照测试。
- **手动验收**：清单直接对齐第 1 节的 4 条成功标准。

## 7. 代码落点

| 位置 | 内容 |
|---|---|
| `crates/run_ledger`（新，无 gpui） | `Run` / `RunState` / `Ledger` / `RunEvent` / `redact.rs` / `store.rs` |
| `crates/terminal` | 在已有 osc133 + busy 路径上发 `RunEvent`；不感知 Ledger |
| `crates/sleipnir_ui` | `chrome/tab_strip.rs`（从 `app_shell.rs` 拆出）、`run_ledger_panel.rs`、tombstone 横幅 |
| `crates/sleipnir_settings` | 上表设置键 |
| `docs/adr/0009-run-ledger-persistence.md`（新） | 持久化 + 脱敏 + 默认值决策 |
| `docs/adr/0006-*` | 状态 proposed → accepted |
| `docs/glossary.md` | 新增 Run / Ledger / Anchor / Attention |
| `README.md` / `website/` | 叙事改写（性能降级为底线要求） |

## 8. 分期

- **P0**：`run_ledger` crate（状态机 + 脱敏 + 存储）、`RunEvent` 打通、Tab/Pane 徽标、
  `inject_osc133` 默认开、设置键、ADR-0009。
- **P1**：Ledger 面板（浮层）+ 跳转、gutter 标记、tombstone 横幅、通知重构、叙事改写。
- **P2**：面板可钉住为常驻侧栏、Dock / 任务栏 badge。
