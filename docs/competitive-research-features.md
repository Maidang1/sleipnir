# 竞品功能调研：可借鉴清单

| Field | Value |
|-------|--------|
| **Date** | 2026-08-12 |
| **Status** | Research complete; **implementation not started** (docs only) |
| **Scope** | Sleipnir vs Kaku / Kitty / Ghostty（辅：WezTerm 因 Kaku 同源、iTerm2 作 macOS 习惯对照） |
| **Goal** | 找「值得借鉴」的功能，按与 Sleipnir 产品边界的匹配度排序，而不是功能大而全 |
| **Follow-up plan** | [`docs/superpowers/plans/2026-08-12-post-m10-feature-roadmap.md`](superpowers/plans/2026-08-12-post-m10-feature-roadmap.md) |
| **Milestone notes** | [`M11`](M11.md) · [`M12`](M12.md) (next) · [`M13`](M13.md) · [`M14`](M14.md) · [`M15`](M15.md) |

## 1. Sleipnir 现状基线（M0–M10）

**已有：** GPU 渲染 · Tabs/Splits/PaneTree · 自适应主题 · Smart paste（图→路径 / Finder→quoted）· Zed 兼容 JSON 配置 · vi mode · Session 结构恢复 · Command palette · Find in scrollback · Ligatures · 自动更新 · Tab 重命名 · http(s) 超链接打开 · `copy_on_select` 设置位（后端已接）· `bell` 设置位（事件空处理）

**刻意不做 / 暂缓：** 跨平台 · 内置 AI · Lua/Python 扩展 · Kitty graphics · 原生 AppKit tabbing 作主模型 · 默认半透明内容（HIG doc Non-Goal）

**已 defer：** 每 pane 独立字体（M10）

---

## 2. 功能矩阵（摘要）

| 能力 | Sleipnir | Kaku | Kitty | Ghostty | 借鉴价值 |
|------|:--------:|:----:|:-----:|:-------:|----------|
| Tabs / splits | ✅ | ✅ | ✅ | ✅ | — |
| Session restore（布局+cwd） | ✅ | ✅ 窗口快照 | ✅ session file | ✅ window-save-state | 已齐 |
| Command palette | ✅ | 部分 | ✅ | ✅ | 已齐 |
| Find scrollback | ✅ | ✅ | 外置 pager 更强 | ✅ | 增强见下 |
| Smart paste 图→路径 | ✅ 强 | ? | — | — | **我们领先** |
| Zed/JSON 配置 | ✅ | Lua | conf/Python | conf | 差异化保留 |
| Runtime 字体缩放 ⌘± | ❌ | ✅ | ✅ | ✅ | **P0** |
| 新 OS Window ⌘N | ❌ | ✅ | ✅ | ✅ | **P0** |
| 关窗/关 tab 确认（有进程） | ❌ | ✅ | 可选 | 可选 | **P0** |
| 路径/文件 Cmd-click 打开 | 检测未开 | ✅ | ✅ | ✅ | **P0** |
| URL hover 反馈 | ❌（M11 草案） | 有 | 有 | link-previews | **P0** |
| Bell 真正反馈 | 设置有/无效果 | visual | 有 | 有 | **P0** |
| copy_on_select 默认/体验 | 设置有 | 默认开 | 选区剪贴板 | 配置 | **P0 产品** |
| Pane zoom | ❌ | Wez 有 | Stack layout | ✅ | **P1** |
| 未聚焦 split 变淡 | ❌ | 有 | 有 | unfocused-split-opacity | **P1** |
| Option/Cmd-click 移光标 | ❌ | Option+Click | — | cursor-click-to-move | **P1** |
| Shell integration（跳 prompt） | ❌ | 弱/插件 | ✅ 强 | ✅ 强 | **P1** |
| 命令完成通知 | ❌ | AI 修错更重 | — | notify-on-command-finish | **P1** |
| Quick Terminal 下拉 | ❌ | — | — | ✅ 标志性 | **P1** |
| Pane input broadcast | ❌ | ✅ | 有限 | — | **P1** |
| Quick Select（键盘点选） | ❌ | Wez 有 | hints 类 | — | **P1** |
| Scrollback → pager/editor | ❌ | history peek | ✅ | write_screen_file | **P2** |
| 背景透明/blur | ❌（非默认） | ✅ | ✅ | ✅ | **P2 可选** |
| Tab 拖拽重排 | ❌ | ✅ | ✅ | 原生 tab | **P2** |
| 内置 AI | ❌ | ✅ 核心 | ❌ | ❌ | **不借** |
| Graphics protocol | ❌ | Wez 图 | ✅ | ✅ | **不借 / 远期** |
| Remote control / 脚本 | ❌ | Lua | kitten | AppleScript | **弱借** |
| Shell suite 安装器 | ❌ | ✅ | — | 注入 integration | **不借** |

来源：Kaku 官网/`docs/features.md`；Kitty overview；Ghostty features + config reference；本仓库 README / M3–M10 / settings.example。

---

## 3. 推荐借鉴（按优先级）

### P0 — 低成本、高“日常完成度”，对齐“好用的 macOS 终端”

#### 3.1 Runtime 字体缩放（Kitty / Ghostty / 几乎所有人）

| | |
|--|--|
| **是什么** | `⌘+` / `⌘-` / `⌘0` 临时改字号，不写配置也能在外接屏上调 |
| **我们现状** | 只有 settings 里的 `font_size`，无热键 |
| **怎么借** | 窗口级 runtime override；`⌘0` 回 settings 值；可选择是否持久化 |
| **工作量** | S（1–2 天） |
| **风险** | 低；注意与 reflow/PTY resize 同步 |

#### 3.2 新 OS Window（`⌘N`）

| | |
|--|--|
| **是什么** | 独立 `NSWindow`，各窗口自己的 tab 树（CONTEXT.md 已预留多 Window 语义） |
| **我们现状** | 单窗口 |
| **怎么借** | 复用 `AppShell` 实体；session 可后续扩到多窗口 |
| **工作量** | M |
| **风险** | 中：session 格式、快捷键焦点、更新对话框归属 |

#### 3.3 有前台进程时关闭确认（Kaku close confirmation / iTerm）

| | |
|--|--|
| **是什么** | 关 tab/pane/窗前：若 PTY 仍有 shell 或前台 job，弹确认 |
| **我们现状** | 直接关 + SIGTERM 清理，无 UI 确认 |
| **怎么借** | 读 `PtyProcessInfo`；设置 `confirm_close: always \| dirty \| never` |
| **工作量** | S–M |
| **风险** | 低；默认 `dirty` 即可 |

#### 3.4 路径 / 本地文件 Cmd-click 打开（Kaku / Kitty / Ghostty link）

| | |
|--|--|
| **是什么** | 点击 `src/main.rs:42` 或 `/tmp/x` → `open` / 编辑器 |
| **我们现状** | `MaybeNavigationTarget::PathLike` **已解析**，`TermView` 里 **故意忽略**（M3 scope） |
| **怎么借** | 相对路径用 pane cwd 解析；`line:col` 拼 `editor` URL 或 `open`；设置 `path_links` |
| **工作量** | M（解析边缘 case 多） |
| **风险** | 中：误点代码标识符；Kaku 用 matcher 避免 `df.info()` 类误链，可参考 |

#### 3.5 URL / 链接 hover 反馈 + 预览（Ghostty link-previews；M11 草案）

| | |
|--|--|
| **是什么** | 悬停加下划线；可选状态栏/tooltip 显示完整 URL |
| **我们现状** | 能 open http(s)，无 hover 视觉 |
| **怎么借** | 先做 underline；预览可后置 |
| **工作量** | S–M |
| **风险** | 低 |

#### 3.6 Bell 真正生效（设置已有）

| | |
|--|--|
| **是什么** | `BEL` → 闪 tab / 系统响铃 / 不扰民 badge |
| **我们现状** | `Event::Bell` 空分支；`terminal.bell` 配置存在 |
| **怎么借** | `system` \| `visual` \| `off`；visual = tab 短暂高亮 |
| **工作量** | S |
| **风险** | 低 |

#### 3.7 把 `copy_on_select` 做成“产品默认选项”而不是冷设置

| | |
|--|--|
| **是什么** | 选中即复制（Kaku 默认体验；Ghostty/Kitty 可配） |
| **我们现状** | 后端已实现，默认 `false` |
| **怎么借** | 设置面板显式开关 + README 说明；不必强改默认，但要**可发现** |
| **工作量** | S |
| **风险** | 低（默认改 true 可能踩跨应用剪贴板习惯，建议保持 false + 可见） |

---

### P1 — 明显拉开“日常终端”体验，仍符合独立终端定位

#### 3.8 Pane zoom（Ghostty `toggle_split_zoom` / WezTerm）

| | |
|--|--|
| **是什么** | 临时最大化当前 pane，再切回 split 布局 |
| **适配** | 与现有 PaneTree 契合：zoom 态叠一层全屏 leaf，树结构不变 |
| **工作量** | M |
| **价值** | 分屏写日志/跑 TUI 时极高频 |

#### 3.9 未聚焦 split 变淡（Ghostty `unfocused-split-opacity`）

| | |
|--|--|
| **是什么** | 非 Active Pane 略降透明度或加遮罩 |
| **适配** | `TermElement` 按 `focused` 已有分支，可加 dim overlay |
| **工作量** | S |
| **价值** | 分屏焦点可读性；几乎零心智成本 |

#### 3.10 Option-click 在当前输入行移动光标（Kaku）

| | |
|--|--|
| **是什么** | 点 prompt 行位置 → 发光标移动序列；不跨 hard newline |
| **Ghostty 变体** | `cursor-click-to-move`，依赖 shell integration / OSC 133 |
| **怎么借** | **第一版可不做 shell integration**：只在“最后一行 / 未进 alt screen”时模拟左右箭头（Kaku 路径更轻） |
| **工作量** | M |
| **风险** | 中：readline/zsh 行为差异 |

#### 3.11 轻量 Shell integration（Kitty / Ghostty）

| | |
|--|--|
| **是什么** | OSC 133 标记 prompt/command；跳上一/下一 prompt；可选“上一命令输出” |
| **我们现状** | 无；Zed 终端侧有 cwd 历史等痕迹可研究 |
| **怎么借** | 先做 **detect + 跳 prompt**，不做装一堆 zsh 插件（避免 Kaku shell suite 路线） |
| **工作量** | L |
| **价值** | 长输出后导航；命令完成通知的底座 |

#### 3.12 命令完成通知（Ghostty `notify-on-command-finish`）

| | |
|--|--|
| **是什么** | 长命令结束且窗口失焦 → 系统通知 / bell |
| **依赖** | 理想情况 shell integration；粗糙版可用 title/`cwd` 启发式（不推荐） |
| **工作量** | M（在 3.11 之后 S） |
| **价值** | 编译/测试场景刚需；比 AI 修错轻、更符合我们定位 |

#### 3.13 Quick Terminal（Ghostty macOS 标志能力）

| | |
|--|--|
| **是什么** | 全局热键下拉半屏终端，不打断当前 App |
| **适配** | GPUI 窗口 + `NSPanel`/浮层 + 全局快捷键；与 HIG “原生感”一致 |
| **工作量** | L |
| **风险** | 中：权限、焦点、多屏、与主窗口 session 关系 |
| **价值** | macOS 差异化；Ghostty 用户最常提的“爽点”之一 |

#### 3.14 Pane input broadcast（Kaku / WezTerm）

| | |
|--|--|
| **是什么** | 一次按键发到多个 pane（多机同步运维） |
| **适配** | `AppShell` 维护 broadcast 集合；输入路径 fan-out |
| **工作量** | M |
| **价值** | 小众但专业；和 AI 无关，适合“工具型终端” |

#### 3.15 Quick Select / 键盘超链接点选（WezTerm）

| | |
|--|--|
| **是什么** | 一键高亮 URL/路径/hash，打字母标签复制/打开 |
| **适配** | 已有 hyperlink/path 解析 + command palette 交互模式 |
| **工作量** | L |
| **价值** | 键盘党刚需；与 vi mode 互补 |

---

### P2 — 锦上添花 / 可选美学 / 后置

| 功能 | 参考 | 备注 |
|------|------|------|
| Scrollback 丢进 pager / 外部编辑器 | Kitty `show_scrollback` | 我们已有 in-app find；pager 是增强 |
| 背景 opacity + blur | Ghostty / Kaku / Wez | HIG 文档写明**默认不透明**；可做 opt-in |
| Tab 拖拽重排 / 拖到新窗口 | Kitty / 原生 tab | 与 M11 chrome 可一起做 |
| 滚动条可视 | Kitty / Ghostty | 动量滚动可在 M11 做 |
| History peek（全屏 TUI 时瞥 scrollback） | Kaku | 依赖 alt screen 处理 |
| 更大主题库 / 主题导入 | Ghostty 数百主题 | 非阻塞 |
| AppleScript 最小字典 | Ghostty / Kaku | 自动化友好；表面保持只读+quit |
| 选区 → 外部程序 | Kitty pass selection | 低频 |

---

## 4. 明确不建议借鉴（或极远期）

| 功能 | 为什么不 |
|------|----------|
| **内置 AI 助手 / `#` 自然语言命令** | Kaku 核心差异化；会改产品身份、引入密钥与合规；与「干净终端」定位冲突 |
| **Shell suite 安装器**（z、autosuggest、starship…） | 污染用户 shell；难维护；应让用户用自己的 dotfiles |
| **Kitty graphics / 完整图像协议** | 渲染栈是 GPUI text batch，不是专用 cell+image pipeline；投入产出差 |
| **Remote control / kitten 式插件** | 运维面大；JSON 配置产品不需要平台化 |
| **深度 Lua 配置兼容** | 与 Zed JSON 路线冲突；二选一即可 |
| **跨平台** | 当前明确 macOS-only |

---

## 5. 建议的产品节奏（可执行）

```
M11 视觉 polish（已有草案）
  ├─ cursor fade blink
  ├─ URL hover underline
  └─ （可选）scroll momentum

M12 “日常缺口”补齐  ← 建议下一里程碑
  ├─ ⌘±/⌘0 字体缩放
  ├─ ⌘N 多窗口
  ├─ 关闭确认
  ├─ PathLike 打开
  ├─ Bell visual
  └─ copy_on_select 可发现 + 设置 UI

M13 分屏专业度
  ├─ pane zoom
  ├─ unfocused dim
  └─ （可选）broadcast

M14 shell 协作
  ├─ OSC 133 integration（最小）
  ├─ jump to prompt
  └─ notify on command finish

M15 差异化（可选）
  ├─ Quick Terminal
  └─ Quick Select
```

**原则：** 先补「所有竞品都有、我们没有」的 **P0**，再做 **分屏与 shell 协作**，最后才是 Ghostty 式 **Quick Terminal** 这类形象功能。不要并行开 AI 战线。

---

## 6. 我们已领先、应守住的点

1. **Smart paste（图→路径 / Finder→quoted / 强制纯文本）** — 比多数竞品更贴 CLI 工作流  
2. **Zed 兼容配置 + GPUI 工程杠杆** — 升级路径清晰  
3. **PaneTree + 拖动实时 reflow** — 交互选择正确  
4. **产品边界克制** — 不要为对齐 Kaku 功能表而膨胀  

---

## 7. 结论（一句话）

**最值得借的不是 AI，而是：字体热缩放、多窗口、关进程确认、路径点击打开、链接 hover、真 bell、pane zoom、失焦变淡、轻量 shell integration、命令完成通知，以及可选的 Quick Terminal。**

这些都能在「macOS 原生、干净、可维护」叙事内完成；Kaku 的 AI/shell suite 与 Kitty 的平台化扩展应主动放弃。

---

## 8. 来源

- [Ghostty Features](https://ghostty.org/docs/features) · [Ghostty Config Reference](https://ghostty.org/docs/config/reference)
- [Kitty Overview](https://sw.kovidgoyal.net/kitty/overview/)
- [Kaku](https://kaku.fun/) · [Kaku features.md](https://github.com/tw93/Kaku/blob/main/docs/features.md)
- [WezTerm Quick Select](https://wezterm.org/quickselect.html)
- 本仓库：`README.md` · `docs/M3.md`–`M10.md` · `docs/settings.example.json` · `docs/ui-chrome-hig-redesign.md` · `docs/superpowers/specs/2025-01-20-m11-visual-polish-design.md`
