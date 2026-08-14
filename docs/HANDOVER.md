# 交接文档：Sleipnir 路线图实现进度（2026-08-14）

| Field | Value |
|-------|-------|
| **日期** | 2026-08-14 |
| **作者** | 实现 agent（DeepSeek，多轮 goal 会话） |
| **目标** | 按 `docs/superpowers/plans/2026-08-14-post-competitive-research-roadmap.md` 继续实现（第 0 步性能基线已由用户完成） |
| **目标 ID** | `goal-d01031e0-0c23-4342-b205-7614921817d9`（**未完成，active**） |
| **结论** | 第 1 步 ✅、第 2 步约 3.5/4（真实 PTY OSC + shell 语义层已落地；回填压缩仍待做）、第 3 步大部分 ✅。 |

---

## 1. 已完成工作总览（按路线图步骤）

> 每项都经过 `cargo check`（全 workspace）+ `cargo test`（相关 crate）验证；测试计数为最后一次全绿时的数字。

### 第 1 步：高频 UX 缺口 —— ✅ 全部完成

| 功能 | 主要文件 | 说明 |
|------|----------|------|
| 搜索正则/大小写开关 | `crates/sleipnir_ui/src/app_shell.rs`（`find_regex`/`find_match_case`/`find_pattern`，find bar 的 `Aa`/`.*` 按钮，`⌥⌘C`/`⌥⌘R`）；`crates/terminal/src/alacritty.rs`（内联标志测试） | 字面+大小写语义用 `(?i)`/`(?-i)` 覆盖 alacritty smart-case |
| scrollback 导出 | `crates/terminal/src/terminal.rs`（`scrollback_text`）；`app_shell.rs`（`ExportScrollback` action）；`crates/sleipnir/src/app_menus.rs`（Shell/File 菜单）；`command_palette.rs` | 写临时文件并打开默认编辑器 |
| 链接/路径 hover tooltip | `crates/sleipnir_ui/src/sleipnir_ui.rs`（`LinkPreview` + `.tooltip`） | 复用 M11 的 `last_hovered_word` |
| tab 拖拽重排 | `app_shell.rs`（`TabDragPreview`、`on_drag`/`on_drop`、`reorder_tab`、`reorder_insert_index` + 单测） | 拖到另一 tab 前插入 |
| tab 拖到新窗口（detach） | `app_shell.rs`（`wire_term_view` 所有权守卫、`detach_tab_to_new_window`、`open_sleipnir_window_with_tab`、`adopt_tab`、pane-area `.on_drop`） | 拖到终端区 → 新窗口，PTY 继续运行 |

> ⚠️ detach 的 drop 命中测试（终端 canvas 上的 drop 是否稳定命中 `pane-area` 的 `on_drop`）需人工 GUI 验证。

### 第 2 步：深度能力 —— 约 3.5/4

| 功能 | 主要文件 | 状态 |
|------|----------|------|
| 新 tab/split 继承 cwd | `app_shell.rs`（`active_working_directory`，`add_tab`/`split_active` 走 `spawn_term_view_with_cwd`） | ✅ |
| 通知矩阵（阈值+模式） | `crates/sleipnir_settings/src/sleipnir_settings.rs`（`NotifyOnCommandFinish` 枚举 + `notify_on_command_finish_mode`）；`sleipnir_ui.rs`（按 never/unfocused/always 判定） | ✅ |
| OSC 9/777 桌面通知（解析+事件链路） | `crates/terminal/src/osc_notify.rs`（`OscNotifyScanner` + 6 单测）；`terminal.rs`（`Event::Notify`、`ingest_osc_notify` + `TerminalBackendEvent::DesktopNotification`）；`sleipnir_ui.rs`（`Event::Notify` → `notify_message` osascript） | ✅ display-only + 真实 PTY（fork `osc_custom`） |
| shell 语义层（自动注入/点击移光标/三击选） | `shell_semantics.rs` + `terminal.inject_osc133` + `mouse_down` | ✅ 自动注入 zsh/bash/fish（默认关）；Option/Alt-click 移光标；Cmd/Ctrl-三击选命令输出 |
| 回填内存字节预算+压缩 | — | ⏳ alacritty 回填存储是行数语义，压缩需改上游 grid |

### 第 3 步：边界重议与锦上添花 —— 大部分 ✅

| 功能 | 主要文件 | 状态 |
|------|----------|------|
| kitty graphics 评估 | `docs/adr/0004-kitty-graphics-track-not-implement.md` | ✅ 结论「跟踪、暂不上」 |
| 主题扩充（Dracula / One Dark） | `crates/sleipnir_settings/src/themes.rs` | ✅ 内置共 13 套 |
| 自定义调色板 `custom_theme` | `themes.rs`（`parse_hex_color`/`CustomPalette`）；`sleipnir_settings.rs`（`resolve_palette`） | ✅ |
| 名称式主题导入 | `themes.rs`（`ThemeSetting` Builtin/Custom）；`sleipnir_settings.rs`（`themes.json` 用户目录合并）；`app_shell.rs`（选择器列出用户主题） | ✅ |
| 601 套打包目录 + 搜索 | `resources/themes.json`（从 [mbadolato/iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes) 601 套转换，MIT，转换脚本 `scripts/convert-iterm-schemes.py`）；`app_shell.rs`（`theme_query` type-to-filter）；`sleipnir_settings.rs`（`load_user_themes` 合并内置+用户）；`docs/themes.example.json` | ✅ |
| 最小 AppleScript 字典 | `resources/Sleipnir.sdef`（新）、`resources/Info.plist`（`NSAppleScriptEnabled`+`OSAScriptingDefinition`）、`scripts/make-app.sh`（打包拷贝） | ✅（需打包后 osascript 实测） |
| 只读屏幕阅读器 AX | `crates/terminal/src/alacritty.rs`（`visible_screen_text` + 单测）；`sleipnir_ui.rs`（`.role(MultilineTextInput)`+`.aria_value`） | ✅（完整 AX 树 ⏳） |

### 其它产出

- `docs/adr/0004`、`docs/adr/0005`（fork pin）、`UPSTREAM.md` 更新、`README.md`/`CHANGELOG.md` 逐项更新、`docs/settings.example.json`。

---

## 2. 关键发现 / 决策（后续接手者必读）

1. **OSC 133/9/777 曾在真实 PTY 路径被丢弃**（M14「跳 prompt」对真实 shell 不生效的根因）：
   - `osc133.rs`/`osc_notify.rs` 扫描器只接在 display-only 的 `write_output`。
   - 真实 PTY 走 alacritty `EventLoop` → `vte::ansi::Processor`，上游 `osc_dispatch` 只认识 0/2/4/8/10/11/12/52 等。
   - **已修：** Maidang1/alacritty + Maidang1/vte fork pin（ADR-0005）给 Handler 加 `osc_custom`，Term 识别 133/9/777 并 emit 事件，经 `ZedListener` 进入 `record_osc133_marker` / `Event::Notify`。
2. **fork pin 决策**：补丁留在 [Maidang1/alacritty](https://github.com/Maidang1/alacritty) / [Maidang1/vte](https://github.com/Maidang1/vte) 的 `sleipnir-osc-custom` 分支，本仓只 pin `rev`。`EventedPty` tee 方案需自引用，整树 vendor 被否掉（见 ADR-0005）。
3. **主题数据**：`resources/themes.json` 601 套来自 iterm2-color-schemes（MIT），转换脚本已归档为 `scripts/convert-iterm-schemes.py`。

---

## 3. alacritty OSC 接线（fork pin）—— ✅ 已收尾

**已做：**
- [Maidang1/alacritty](https://github.com/Maidang1/alacritty) `sleipnir-osc-custom`：zed fork + 补丁（`Event::Osc133` / `DesktopNotification`，`Handler::osc_custom` 识别 133/9/777）。
- [Maidang1/vte](https://github.com/Maidang1/vte) `sleipnir-osc-custom`：0.15.0 + `Handler::osc_custom` 钩子；根 `Cargo.toml` `[patch.crates-io]` pin 同一 `rev`。
- `crates/terminal`：`ZedListener` 映射新事件；`record_osc133_marker` 被 display-only 扫描器和真实 PTY 共用；`DesktopNotification` → `Event::Notify`。
- 测试：`Osc133Kind::from_payload`、fork `osc_custom` 喂字节、`ZedListener` 转发。
- 转换脚本归档：`scripts/convert-iterm-schemes.py`。

**手动验证（接手者在 GUI 里点一次）：**
- 真实 shell `printf '\e]133;A\a'` 后 `⌘⇧↑` 可跳 prompt。
- `printf '\e]9;hello\a'` 触发 macOS 通知。

**风险/注意：**
- `scripts/upstream-diff.sh` 只对比 Zed 的 alacritty pin；升级 = 在 fork 的 `sleipnir-osc-custom` 上 merge 基线并更新本仓 `rev`（ADR-0005）。
- `Handler` trait 加了带默认实现的方法，现有其它 `Handler` impl（如 `StdSyncHandler`）不受影响。

---

## 4. 遗留事项（roadmap 剩余）

| 项 | 阻塞 | 建议 |
|----|------|------|
| shell 语义层：自动注入 / 点击移光标 / 三击选命令输出 | — | ✅ `inject_osc133`（默认关）+ Option/Alt-click + Cmd/Ctrl-三击 |
| OSC 9/777 真实路径 | — | ✅ 已接线；需 GUI 手动验证通知 |
| 回填内存字节预算+压缩 | alacritty grid 行数语义，需改上游存储 | 大工程，建议先只做「字节预算近似」（按平均行长动态调 `max_scroll_history_lines`） |
| 拖到新窗口 drop 命中 | 需人工 GUI 验证 | 若终端 canvas occlude drop，给 TermElement 补透传 drop listener |
| 完整屏幕阅读器 AX 树 | 大工程 | 只读 AX 已可读屏，交互/光标朗读后续 |
| iterm2-color-schemes 700+ 与 601 的差异 | 数据搬运 | 现目录 601 套已覆盖主流；如需补齐可定期重跑转换脚本 |

---

## 5. 本次会话的验证口径

- 每轮：`cargo check`（全 workspace）+ `cargo test -p sleipnir_settings -p sleipnir_ui -p terminal`。
- 语义层收尾后全绿：sleipnir_settings 21 / sleipnir_ui 50 / terminal 58。

## 6. 交接后建议的第一步

GUI 手动验证：`inject_osc133: true` 后新开 tab 应发出 OSC 133；Option-click 移光标；Cmd-三击选命令输出。回填内存压缩 / 完整 AX / detach drop 仍待做。
