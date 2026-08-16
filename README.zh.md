<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir 应用图标" width="160" height="160" />

# Sleipnir

**面向 macOS 的原生终端：GPU 绘制，支持标签和分屏。**

基于 [GPUI](https://gpui.rs)（[Zed](https://github.com/zed-industries/zed) 的 UI 框架）
和一份 fork 过的终端后端。

[功能](#功能) · [安装](#安装) · [配置](#配置) · [快捷键](#快捷键)

[English](README.md) · [中文](README.zh.md)

</div>

---

## 简介

Sleipnir 是独立的 macOS 终端，通过 GPUI 在 GPU 上绘制，大量输出时滚动和重绘也能跟上。
自带真实主机 PTY、输入法、多标签和分屏、多窗口、跟随系统外观的主题，以及会把粘贴的图片写成带引号路径的剪贴板。

预编译包是 macOS 的 `.dmg` / `.zip`，在
[GitHub Releases](https://github.com/Maidang1/sleipnir/releases)。
不支持 Windows 和 Linux。

名字来自北欧神话：奥丁的八腿坐骑，九界里最快的马。图标把这个抽象成马头标记，压在一段终端提示符上。

## 功能

- **GPU 绘制**，滚动和重绘走 GPUI（Metal）；光标缓入缓出闪烁。
- **标签、分屏与窗格**，默认左侧标签轨，也可以改成顶部标签条（`tab_placement: top`，设置里的 Tab placement，View → Toggle Tab Placement，或命令面板）。两种布局都按 git 工作区静默分组，不画分组标题。侧栏是两行：标题，下一行是分支和脏计数 `+N −M`。顶部条只显示窗格 cwd 的最后两级（`myself/harbor`），不显示分支和脏计数。右键重命名仍可覆盖。可以向右 / 向下分屏，跳转标签，移动焦点；组内拖动重排，拖到终端区域会拆成新窗口；把后台标签拖到当前窗格上会合并成分屏；把窗格把手拖到标签列表会抽成新标签。支持窗格放大，未聚焦窗格会变暗。识别到的编码 Agent 会显示字母标记（`claude` → `C`，`codex` → `X` 等）；普通 shell 没有占位符。有失败 Attention 的标签整块淡红，不画运行中 / 成功圆点。Mark Tab as Seen 只清 Attention，不删 Run 记录。
- **多窗口**，`⌘N` 打开独立窗口，标签和 shell 各自一套。
- **字体缩放**，`⌘+`（以及 `-` / `0`）只改当前窗口的格子大小，不写进设置。
- **自适应主题**，Catppuccin 各口味，加上 Tokyo Night、Nord、Gruvbox、Solarized、GitHub Dark/Light、Dracula、One Dark；`auto` 跟随系统浅色 / 深色。额外调色板放配置目录的 `themes.json`（`"theme": "kanagawa"`，见 `docs/themes.example.json`）。
- **智能粘贴**，粘贴图片会得到带引号的临时文件路径；Finder 选中的文件粘成带引号路径；需要纯文本时可以强制。
- **兼容 Zed 配置**，`terminal.*` 可以直接复用；`⌘⇧R` 热重载。
- **vi 模式**，用键盘做选择和导航。
- **辅助功能**，可见屏幕作为只读无障碍值暴露出去，VoiceOver 能读当前输出，和 Ghostty 的只读 AX 一样。
- **会话恢复**，重启后标签、分屏和工作目录还在。
- **命令面板**，`⌘⇧K` 找动作；设置里可以覆盖快捷键。`keybinding_preset: tmux` 加上 `ctrl-b` 的标签 / 窗格组合键。**Pane Facts**（View 菜单）显示当前窗格的目录、进程树和监听端口。
- **Run Ledger**，`⌘⇧L` 打开脱敏后的命令运行记录；点一行跳到对应窗格和 OSC 133 Anchor。默认写入 `runs.json`（`run_ledger`：`off` / `memory` / `persist`）。窗格装订线三角形标出命令起止（备用屏幕上隐藏）。恢复会话后，界面上的 tombstone 横幅会写出上次启动里最后一条命令（不是滚动历史；一打字就消失；`show_tombstone: false` 可关掉）。
- **控制面**，默认关闭。`control_surface: true` 或 `SLEIPNIR_CONTROL=1` 会绑定 `~/.config/sleipnir/control.sock`；`sleipnir-ctl ls|capture|send|wait` 驱动活着的窗格（[ADR-0011](docs/adr/0011-control-surface.md)）。
- **滚动历史搜索**，`⌘F` 高亮匹配，可切正则（`.*`）和区分大小写（`Aa`）；**Shell → Export Scrollback…** 把滚动历史写到文件并用默认编辑器打开。
- **路径链接与铃**，⌘-点击路径用默认应用打开；悬停预览 URL / 路径；可选系统铃或视觉闪。
- **关闭确认**，非 shell 任务在跑时会问（`confirm_close`：`dirty` / `always` / `never`）。
- **和 shell 协作**，OSC 133 提示符跳转；新标签继承工作区的 git 根（分屏继承当前窗格的 cwd）；失焦时长时间命令结束可以发 macOS 通知。**Shell → Search Shell History**（`⌘⇧;`）对 `HISTFILE` / `~/.zsh_history` 做模糊搜索。Send Selection / Send Git Diff 写进当前窗格；`pipe_selection_command` 把选区交给外部命令。
- **Quick Terminal / Quick Select**，快速开一个备用窗口；面向链接的选择模式。
- **Attention**，菜单栏条目（`show_tray_icon`，默认开）和 Dock 上的失败 Attention 计数。和标签淡红是两套东西。

## 安装

Sleipnir **只支持 macOS 14+**。装最新 GitHub Release：

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

脚本会下载 `Sleipnir-<ver>-macos.zip`，对照发布的 `.zip.sha256` 校验，复制到 `/Applications`，再跑 `xattr -cr` 去掉隔离标记。CI 构建是 ad-hoc 签名（没有 Developer ID），不做这一步的话，第一次打开 Gatekeeper 会报 unidentified developer。

想装到别的目录，或装完不要自动打开：

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh \
  | PREFIX="$HOME/Applications" SLEIPNIR_NO_OPEN=1 bash
```

也可以从 [Releases](https://github.com/Maidang1/sleipnir/releases) 拿 `.dmg` / `.zip`。
如果 macOS 还是拦，执行：

```bash
xattr -cr /Applications/Sleipnir.app
```

## 环境

跑发行版需要：

- macOS 14.0+（Sonoma）

从源码编译还需要：

- Rust **1.95.0**（见 `rust-toolchain.toml`）
- Xcode 和 Metal Toolchain（`xcodebuild -downloadComponent MetalToolchain`）

这个 crate 在 Windows 和 Linux 上编不过。

## 编译和运行

```bash
cargo run -p sleipnir
# 二进制：target/debug/sleipnir
```

## 配置

兼容 Zed 的 `terminal.*`，再加上 Sleipnir 自己的键。设置在
`~/.config/sleipnir/settings.json`，会话恢复在 `session.json`。
默认字体是 **Menlo**。

| 键 | 含义 |
|-----|---------|
| `theme` | `auto` / `mocha` / …（见示例）；`auto` 跟随系统外观；`themes.json` 里的名字也能用 |
| `custom_theme` | 可选十六进制调色板（`background` / `foreground` / `ansi` 等），会覆盖 `theme` |
| `restore_session` | 启动时恢复标签 / 分屏 / cwd（默认 `true`） |
| `confirm_close` | `dirty` / `always` / `never`，忙窗格关闭前提示（默认 `dirty`） |
| `path_links` | ⌘-点击打开路径类目标（默认 `true`） |
| `key_bindings` | 额外组合键（`{ "key": "cmd-alt-t", "action": "new_tab" }`）。动作：`new_tab`、`close_tab`、`next_tab`、`prev_tab`、`split_right`、`split_down`、`new_window`、`open_settings`、`reload_settings`、`cycle_theme`、`find`、`toggle_command_palette`、`increase_font_size`、`decrease_font_size`、`reset_font_size`、`toggle_pane_zoom`、`toggle_broadcast`、`jump_prev_prompt`、`jump_next_prompt`、`toggle_quick_select`、`open_quick_terminal`、`export_scrollback`、`check_for_updates`、`clear_run_ledger`、`toggle_run_ledger`、`mark_tab_seen`、`toggle_pane_facts`、`send_selection`、`pipe_selection`、`send_git_diff`、`toggle_diff`、`toggle_history_search`、`toggle_tab_placement`。可选 `context`：`AppShell` / `Terminal`。改完要重启才生效。 |
| `terminal.font_ligatures` | 打开 OpenType 连字（默认 `false`） |
| `terminal.copy_on_select` | 鼠标松开即复制（默认 `false`；设置里可切） |
| `terminal.bell` | `off` / `system` / `visual`（默认 `off`） |
| `background_opacity` | 内容不透明度 0.15–1.0（默认 `1.0` 不透明） |
| `notify_on_command_finish_secs` | 长任务通知阈值，单位秒（默认 `5`；`0` 关掉） |
| `notify_on_command_finish_mode` | `never` / `unfocused` / `always`（默认 `unfocused`） |
| `run_ledger` | `off` / `memory` / `persist`，采集并显示 Run Ledger（默认 `persist`） |
| `run_ledger_retention_days` | 持久化记录保留天数（默认 `7`） |
| `run_ledger_max_runs` | 持久化条数上限，先丢最旧的（默认 `500`） |
| `run_ledger_redact` | 采集时脱敏命令行（默认 `true`；启发式，不是保证） |
| `tab_placement` | `side`（默认）左侧轨，标题 + 分支/`+N −M` / `top` 顶部条，只显示 cwd 路径。静默分组和组内拖动一样 |
| `sidebar_width` | 左侧轨宽度，单位 px，160–320（默认 `200`）。不是实时拖动的分割条 |
| `agent_icons` | 已知编码 Agent 的字母标记（默认 `true`） |
| `control_surface` | 绑定本地控制套接字（默认 `false`）。`SLEIPNIR_CONTROL=1` 也会打开 |
| `show_tray_icon` | 菜单栏 Attention 条目（默认 `true`）。Dock 角标是另一回事 |
| `pipe_selection_command` | 接收当前选区的外部命令；空字符串表示关掉 |
| `keybinding_preset` | `default` / `tmux`（`ctrl-b` 再跟 `c` / `%` / `"` / 方向键 / `z`） |
| `show_tombstone` | 用上次启动的 Run 元数据画恢复横幅（默认 `true`） |
| `terminal.inject_osc133` | 向 zsh / bash / fish 注入 OSC 133 A/B/C/D（默认 `true`；以前是 `false`） |

`⌘,` 打开应用内主题选择器，也可以改文件后用 `⌘⇧R` 重载
（快捷键覆盖要下次启动才生效）。
见 [`docs/settings.example.json`](docs/settings.example.json)。

## 粘贴

| 剪贴板 | 粘贴行为（`⌘V` / `⌃⇧V`） |
|-----------|----------------|
| 图片（截图等） | 写到临时文件，粘贴带引号的绝对 POSIX 路径 |
| Finder 文件路径 | 空格分隔的带引号路径 |
| 文本 | 普通粘贴（应用开启时走 bracketed paste） |

强制纯文本粘贴（跳过图片转路径）用 `⌃⌘V`。

## 快捷键

| 动作 | 快捷键 |
|--------|----------|
| 复制 | `⌘C` / `⌃⇧C` |
| 粘贴（图片 → 路径） | `⌘V` / `⌃⇧V` |
| 只粘贴文本 | `⌃⌘V` |
| 全选 | `⌘A` |
| 清屏 | `⌘K` |
| 新标签 / 关窗格 | `⌘T` / `⌘W` |
| 新窗口 | `⌘N` |
| 跳到第 N 个标签 | `⌘1`…`⌘9` |
| 向右 / 向下分屏 | `⌘D` / `⌘⇧D` |
| 在窗格间移动焦点 | `⌘⌥←↑↓→` |
| 下一个 / 上一个标签 | `⌃Tab` / `⌃⇧Tab` |
| 下一个 / 上一个标签（备用） | `⌘⇧]` / `⌘⇧[` |
| 放大 / 缩小 / 重置字体 | `⌘+` `⌘=` `⌘-` `⌘0` |
| 切换窗格放大 | `⌘⇧Enter` |
| 切换广播输入 | `⌘⇧B` |
| 跳到上一个 / 下一个提示符 | `⌘⇧↑` / `⌘⇧↓` |
| Quick Select | `⌘⇧O` |
| Quick Terminal | `⌘⇧N` |
| 设置 | `⌘,` |
| 重载设置 | `⌘⇧R` |
| 循环主题 | `⌘⇧P` |
| 命令面板 | `⌘⇧K` |
| Run Ledger | `⌘⇧L` |
| 搜索 shell 历史 | `⌘⇧;` |
| 滚动历史搜索 | `⌘F` |
| 下一个 / 上一个匹配 | `⌘G` / `⌘⇧G` |
| 搜索：区分大小写 / 正则 | `⌥⌘C` / `⌥⌘R` |
| 检查更新 | `⌘⇧U` |
| 按页滚动 | `⌘↑` / `⌘↓` |
| 按行滚动 | `⇧↑` / `⇧↓` |
| 行首 / 行尾 | `⌘←` / `⌘→` |
| 按词后退 / 前进 | `⌥←` / `⌥→` |
| 清除当前行 | `⌘⌫` |
| 删到行尾 | `⌘⌦` |
| 字符面板 | `⌃⌘Space` |
| 切换 vi 模式 | `⌃⇧Space` |
| 退出 | `⌘Q` |

备用屏幕（全屏 TUI）上忽略滚动快捷键。

## 自动更新

Sleipnir 可以从 [GitHub Releases](https://github.com/Maidang1/sleipnir/releases) 自己更新。

- 用 **Sleipnir → Check for Updates…**（`⌘⇧U`）打开更新对话框。
  启动时**不会**自动查更新。
- 发现新版本后，选 **Download & Install** 会拉 `Sleipnir-<ver>-macos.zip`，
  对照发布的 `.zip.sha256` 校验再暂存。CI 构建是 ad-hoc 签名（没有 Apple
  Developer 证书），完整性靠这次 SHA-256 检查，对不上就拒绝下载。
- **Restart & Update** 原地替换正在跑的 `.app` 再启动。如果包所在位置写不进去
  （比如别人拥有的受保护 `/Applications` 安装），会退回去打开发布页，让你手动装。

## 边界

Sleipnir 是给**在终端里跑编码 Agent 的人**用的：人是用户，Agent 是负载。因此：

- **没有内置 AI。** 不调模型，没有聊天面板，不管 API key。Sleipnir 是 Agent
  *跑在里面* 的终端，它自己不是 Agent。想在终端里跟 AI 说话，用
  [Warp](https://warp.dev) 或 [Wave](https://waveterm.dev)。
  （[ADR-0008](docs/adr/0008-no-builtin-ai.md)）
- **不持久化滚动历史。** 会话恢复只带回标签、分屏和工作目录，从不写终端输出，
  因为输出里经常有 token 和密码。要留底稿用 **Shell → Export Scrollback…**。
- **不恢复进程。** 重启不会把正在跑的命令救回来；那是 `tmux` / `zellij` 的事，
  Sleipnir 不重做一遍。
- **没有插件系统。** 扩展点是配置、快捷键，以及把内容交给外部命令。

## 现状

M15 以及尚未发版的 chrome、Run Ledger、控制面都已经在树上，见
[CHANGELOG](CHANGELOG.md)。还没做的：滚动历史字节预算 / 压缩、更完整的
VoiceOver 树、kitty graphics（跟踪但不实现：
[ADR-0004](docs/adr/0004-kitty-graphics-track-not-implement.md)）。

性能是量出来的，不是口头说的：方法在
[`scripts/bench/README.md`](scripts/bench/README.md)，数字在
[`scripts/bench/results.md`](scripts/bench/results.md)。输入延迟，以及和
Ghostty / kitty / Alacritty 的同机对比还没做，吞吐量数字只当内部基线，
不要当成竞品结论。

## 上游

GPUI 栈**没有** vendoring。根目录 `Cargo.toml` 把 `zed-industries/zed` 钉在固定
`rev`（`gpui`、`gpui_macos`、`collections`、`util` 等）。本地 fork 是
`terminal`、一份瘦的 `gpui_platform`，以及 Sleipnir 自己的 crate。升 pin 看
[`UPSTREAM.md`](UPSTREAM.md)。

```bash
./scripts/upstream-diff.sh /path/to/zed
```

## 打包和发布

### 本地构建

```bash
# 打成 .app + .zip（macOS）
./scripts/make-app.sh

# 用开发者证书签名（macOS）
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)"

# 再打 .dmg（需要签名）
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)" --dmg
```

macOS 包用 [`resources/AppIcon.icns`](resources/AppIcon.icns)，从
[`resources/appicon.svg`](resources/appicon.svg) 生成。

### 发到 GitHub Releases（`gh` CLI）

```bash
# 打 tag 并发布
git tag v0.2.0
git push origin v0.2.0

# 或者先构建再发布
./scripts/make-app.sh --sign "..." --dmg
./scripts/publish-release.sh 0.2.0 ./build
```

### CI（GitHub Actions）

推 `v*` tag 会自动跑，也支持手动触发。
工作流在 `macos-latest` 上构建和测试，把 `.dmg`、`.zip` 和 `.zip.sha256`
挂到 GitHub Release。

```bash
gh workflow run build-and-release.yml \
  -f version=0.2.0 \
  -f draft=false
```

**需要的 GitHub Secrets**（仓库 Settings → Secrets and variables → Actions）：

| Secret | 说明 |
|--------|-------------|
| `CODE_SIGNING_CERT_P12` | Base64 编码的 `.p12` 证书 |
| `CODE_SIGNING_CERT_PASSWORD` | `.p12` 密码 |
| `APPLE_ID` | 公证用的 Apple ID 邮箱 |
| `APPLE_APP_SPECIFIC_PASSWORD` | 公证用的 App 专用密码 |
| `APPLE_TEAM_ID` | Apple Developer team ID |

完整流水线见 [`.github/workflows/build-and-release.yml`](.github/workflows/build-and-release.yml)。

## 致谢与许可

Sleipnir 复用并改写了 [Zed](https://github.com/zed-industries/zed) 的代码：

| 组件 | 许可 |
|-----------|---------|
| GPUI 及相关 UI crate | Apache-2.0 |
| 终端 crate（M1 起） | GPL-3.0-or-later |

因为包含 GPL 终端代码，**发行这份合在一起的程序按 GPL-3.0-or-later**。
见 [`LICENSE-GPL`](LICENSE-GPL) 和 `crates/` 下各 crate 的许可文件。
