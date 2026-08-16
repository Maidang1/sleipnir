# Run Ledger P0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把终端已经知道的「一次命令执行」变成 Ledger 里的一等公民，并在 tab / pane 徽标上让「谁在跑、谁跑完了、谁挂了」一眼可见。

**Architecture:** 新增纯 crate `run_ledger`（无 gpui）持有 Run 状态机、脱敏、保留裁剪、Attention/徽标聚合与 `runs.json` 读写；`terminal` 在已有的 OSC 133 与 busy 探测路径上发 `Event::RunStarted/RunFinished`；`TermView` 转发为 `TermViewEvent`，`AppShell` 写入一个 GPUI 全局 Ledger 并渲染徽标。Run 的归属键是持久化在 `session.json` 里的 `pane_key`（UUID），不是会被复用的 pane id。

**Tech Stack:** Rust 1.95（edition 2024）、GPUI（Zed pin）、serde / serde_json、uuid v4、Swift 无关；测试用 `cargo test`（Swift Testing 不适用）。

**Spec:** [`docs/superpowers/specs/2026-08-16-run-ledger-design.md`](../specs/2026-08-16-run-ledger-design.md)

**P0 边界（本计划范围）**：`run_ledger` crate、`RunEvent` 打通、`pane_key`、Tab/Pane 徽标、设置键、`inject_osc133` 默认开、首次写盘告知、`clear_run_ledger`、ADR/glossary/CHANGELOG。
**不在本计划**：Ledger 面板、gutter 标记、tombstone 横幅、通知重构、叙事改写（P1）；钉住侧栏、Dock badge（P2）。

---

## File Structure

| 文件 | 职责 | 状态 |
|---|---|---|
| `crates/run_ledger/Cargo.toml` | 新 crate 清单，**不依赖 gpui** | 创建 |
| `crates/run_ledger/src/run_ledger.rs` | crate 根：`pub mod` 声明 + 重导出 | 创建 |
| `crates/run_ledger/src/run.rs` | `RunId` / `PaneKey` / `LaunchId` / `Run` / `RunState` / `RunEvent` | 创建 |
| `crates/run_ledger/src/ledger.rs` | `Ledger`：`apply()` 状态机、保留裁剪、Attention、徽标聚合 | 创建 |
| `crates/run_ledger/src/redact.rs` | 入账即脱敏（纯函数） | 创建 |
| `crates/run_ledger/src/store.rs` | `runs.json` 读写：版本、原子写、损坏恢复、并发合并 | 创建 |
| `Cargo.toml` | workspace members + `run_ledger` / `uuid` 依赖 | 修改 |
| `crates/sleipnir_settings/src/sleipnir_settings.rs` | 4 个新设置键；`inject_osc133` 默认 `true` | 修改 |
| `docs/settings.example.json` | 同步示例 | 修改 |
| `crates/sleipnir_ui/src/session.rs` | `SessionNode::Leaf.pane_key` | 修改 |
| `crates/sleipnir_ui/src/pane_tree.rs` | `PaneNode::Leaf` 增加 `pane_key: PaneKey` 字段 | 修改 |
| `crates/terminal/src/run_tracker.rs` | 纯状态跟踪：marker/busy → `RunEvent`，**不碰 gpui** | 创建 |
| `crates/terminal/src/terminal.rs` | 接线 `run_tracker`，emit `Event::RunStarted/RunFinished` | 修改 |
| `crates/sleipnir_ui/src/run_ledger_global.rs` | GPUI 全局：持有 `Ledger` + 去抖落盘 + 首次写盘告知 | 创建 |
| `crates/sleipnir_ui/src/chrome/tab_badge.rs` | 徽标渲染 + `format_elapsed` / `badge_label`（纯渲染，聚合逻辑在 run_ledger） | 创建 |
| `crates/sleipnir_ui/src/sleipnir_ui.rs` | `TermViewEvent::Run*` 转发 | 修改 |
| `crates/sleipnir_ui/src/app_shell.rs` | 订阅 → 全局 Ledger；tab chip 渲染徽标；`clear_run_ledger` | 修改 |
| `docs/adr/0009-run-ledger-persistence.md` | 新 ADR | 创建 |
| `docs/adr/0006-tombstone-session-restore.md` | 状态 → superseded by 0009 | 修改 |
| `docs/glossary.md` / `CHANGELOG.md` / `README.md` | 术语、破坏性默认值变更、配置表 | 修改 |

**为什么聚合逻辑放 `run_ledger` 而不是 UI**：徽标优先级（失败 > 在跑 > 成功）、计数、阈值、focus 清空 —— 这些是本设计最容易写错的规则，放在纯 crate 里可以用 `cargo test -p run_ledger` 秒级覆盖，不需要起窗口。

**重要实现说明（来自评审）**：

1. **Tab strip 渲染不是独立函数**：`app_shell.rs` 里的 tab strip 渲染是内联在 `impl Render for AppShell` 中（约 3659–4035 行），依赖私有字段（`hovered_tab`、`rename`、`tab_scroll_handle` 等）。拆分时需要在 `chrome/tab_strip.rs` 中写 `impl AppShell` 方法，并把相关字段改为 `pub(crate)`。
2. **运行时 Pane 不是独立结构体**：`PaneNode::Leaf { id, view }` 在 `pane_tree.rs:34-46`，需要增加 `pane_key: PaneKey` 字段（13 处构造 / match 站点）。
3. **`RunsFile` 需包含 `announced: bool`**：Task 4 的 `RunsFile` 结构体和 API 必须从一开始就包含这个字段，否则 Task 8 无法实现首次写盘告知。
4. **`record_osc133_marker` 没有 `cx` 参数**：解决方案是在 `RunTracker` 内部产出 `TrackerOut`，在有 `cx` 的调用点（`terminal.rs:1726` 的 `TerminalBackendEvent` match）统一 drain 并 emit。display-only 路径只 feed tracker 不 emit。
5. **加载历史时 `Running` 状态的 Run 必须转为 `Abandoned`**：否则 `started_at_mono_ms`（`#[serde(skip)]`）为 0，计时异常并触发虚假的 ● 徽标。
6. **Action 注册在 `command_palette.rs` 的 `CommandId`**：不是 `keymap.rs`。
7. **Settings → Ledger 接线**：`RunLedgerGlobal::init` 必须读 `TerminalSettings::get_global` 并调用 `set_retention` / `set_redact` / `set_success_threshold_secs`；⌘⇧R reload 时也要同步。

---

## Task 1: `run_ledger` crate 骨架 + 领域类型 + 状态机

**Files:**
- Create: `crates/run_ledger/Cargo.toml`, `crates/run_ledger/src/run_ledger.rs`, `crates/run_ledger/src/run.rs`, `crates/run_ledger/src/ledger.rs`
- Modify: `Cargo.toml`（workspace members + deps）

- [ ] **Step 1: 建 crate 骨架并挂进 workspace**

`crates/run_ledger/Cargo.toml`：

```toml
[package]
name = "run_ledger"
version = "0.1.0"
edition.workspace = true
publish = false
license = "GPL-3.0-or-later"

[lints]
workspace = true

[lib]
path = "src/run_ledger.rs"
doctest = false

[dependencies]
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
```

根 `Cargo.toml`：`members` 里加 `"crates/run_ledger"`；`[workspace.dependencies]` 里加

```toml
run_ledger = { path = "crates/run_ledger" }
uuid = { version = "1", features = ["v4", "serde"] }
```

`crates/run_ledger/src/run_ledger.rs`：

```rust
//! Run Ledger: the app's record of "what ran here" (spec 2026-08-16).
//!
//! Pure data + state machine — no gpui, no terminal, no I/O beyond `store`.

pub mod ledger;
pub mod redact;
pub mod run;
pub mod store;

pub use ledger::{Badge, BadgeKind, Ledger};
pub use run::{LaunchId, PaneKey, Run, RunEvent, RunId, RunState};
```

- [ ] **Step 2: 写失败的状态机测试**

`crates/run_ledger/src/ledger.rs`（先只写测试，编译会失败）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{PaneKey, RunEvent, RunState};
    use std::time::Duration;

    fn pane() -> PaneKey {
        PaneKey::new_v4()
    }

    #[test]
    fn exit_zero_succeeds() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "cargo build", None, 1_000));
        ledger.apply(RunEvent::finished(p, Some(0), 42_000));
        let run = ledger.runs().next().unwrap();
        assert_eq!(run.state, RunState::Succeeded);
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.duration, Duration::from_millis(41_000));
        assert_eq!(run.command, "cargo build");
    }

    #[test]
    fn nonzero_exit_fails() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "npm run deploy", None, 0));
        ledger.apply(RunEvent::finished(p, Some(1), 500));
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Failed);
    }

    #[test]
    fn missing_status_is_unknown() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "ssh prod-01", None, 0));
        ledger.apply(RunEvent::finished(p, None, 100));
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Unknown);
    }

    #[test]
    fn inferred_runs_are_marked() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started_inferred(p, "vim", None, 0));
        let run = ledger.runs().next().unwrap();
        assert!(run.inferred, "busy-probe runs must be marked inferred");
    }

    /// 一个 pane 同时只能有一个 Run：新的 Started 让上一条变 Abandoned。
    #[test]
    fn second_start_abandons_the_first() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "first", None, 0));
        ledger.apply(RunEvent::started(p, "second", None, 10));
        let states: Vec<_> = ledger.runs().map(|r| r.state).collect();
        assert!(states.contains(&RunState::Abandoned));
        assert!(states.contains(&RunState::Running));
    }

    /// 没有对应 Started 的 Finished 必须被丢弃，不能造出半条 Run。
    #[test]
    fn orphan_finish_is_ignored() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.apply(RunEvent::finished(pane(), Some(0), 10));
        assert_eq!(ledger.runs().count(), 0);
    }

    #[test]
    fn closing_a_pane_abandons_its_running_run() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "long thing", None, 0));
        ledger.apply(RunEvent::PaneClosed { pane: p, at_ms: 90 });
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Abandoned);
    }
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p run_ledger`
Expected: 编译失败（`Ledger`、`RunEvent` 等未定义）。

- [ ] **Step 4: 实现 `run.rs`**

```rust
//! Domain types for the Run Ledger.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub type RunId = Uuid;
/// Identifies one Pane across restarts; persisted in `session.json`.
pub type PaneKey = Uuid;
/// Identifies one process launch; jumping is only valid within the current one.
pub type LaunchId = Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Succeeded,
    Failed,
    /// Finished without a usable exit code (no OSC 133 `D` status).
    Unknown,
    /// Process/pane/app went away while the Run was still going.
    /// Never rendered as success.
    Abandoned,
}

impl RunState {
    pub fn is_finished(self) -> bool {
        !matches!(self, RunState::Running)
    }
}

/// One command execution. `command` is already redacted (spec §2: redact-at-capture).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub launch_id: LaunchId,
    pub pane: PaneKey,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Wall-clock start, unix millis — for display only.
    pub started_at_unix_ms: u64,
    /// Monotonic start, millis since process start — for duration math.
    #[serde(skip)]
    started_at_mono_ms: u64,
    #[serde(default)]
    pub duration: Duration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub state: RunState,
    /// True when derived from the busy probe instead of OSC 133.
    #[serde(default)]
    pub inferred: bool,
    /// In-memory only (spec §2): Attention never crosses a restart.
    #[serde(skip)]
    pub seen: bool,
}

/// What the terminal reports. Times are monotonic millis since process start.
#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Started {
        pane: PaneKey,
        command: String,
        cwd: Option<String>,
        at_ms: u64,
        inferred: bool,
    },
    Finished {
        pane: PaneKey,
        exit_code: Option<i32>,
        at_ms: u64,
    },
    PaneClosed {
        pane: PaneKey,
        at_ms: u64,
    },
}

impl RunEvent {
    pub fn started(pane: PaneKey, command: &str, cwd: Option<String>, at_ms: u64) -> Self {
        Self::Started { pane, command: command.into(), cwd, at_ms, inferred: false }
    }

    pub fn started_inferred(pane: PaneKey, command: &str, cwd: Option<String>, at_ms: u64) -> Self {
        Self::Started { pane, command: command.into(), cwd, at_ms, inferred: true }
    }

    pub fn finished(pane: PaneKey, exit_code: Option<i32>, at_ms: u64) -> Self {
        Self::Finished { pane, exit_code, at_ms }
    }
}

impl Run {
    pub(crate) fn start(
        launch_id: LaunchId,
        pane: PaneKey,
        command: String,
        cwd: Option<String>,
        mono_ms: u64,
        unix_ms: u64,
        inferred: bool,
    ) -> Self {
        Self {
            id: RunId::new_v4(),
            launch_id,
            pane,
            command,
            cwd,
            started_at_unix_ms: unix_ms,
            started_at_mono_ms: mono_ms,
            duration: Duration::ZERO,
            exit_code: None,
            state: RunState::Running,
            inferred,
            seen: false,
        }
    }

    pub(crate) fn finish(&mut self, exit_code: Option<i32>, mono_ms: u64) {
        self.duration = Duration::from_millis(mono_ms.saturating_sub(self.started_at_mono_ms));
        self.exit_code = exit_code;
        self.state = match exit_code {
            Some(0) => RunState::Succeeded,
            Some(_) => RunState::Failed,
            None => RunState::Unknown,
        };
    }

    pub(crate) fn abandon(&mut self, mono_ms: u64) {
        self.duration = Duration::from_millis(mono_ms.saturating_sub(self.started_at_mono_ms));
        self.state = RunState::Abandoned;
    }
}
```

- [ ] **Step 5: 实现 `ledger.rs` 的状态机部分**

在 `ledger.rs` 顶部（测试模块之前）写：

```rust
//! The Ledger: every Run this app has seen, plus the rules for what to show.

use crate::run::{LaunchId, PaneKey, Run, RunEvent, RunState};

/// Wall-clock source injected by the caller so tests stay deterministic.
pub type UnixMillisFn = fn() -> u64;

pub struct Ledger {
    launch_id: LaunchId,
    /// Oldest first.
    runs: Vec<Run>,
    now_unix_ms: UnixMillisFn,
}

impl Ledger {
    pub fn new(launch_id: LaunchId) -> Self {
        Self { launch_id, runs: Vec::new(), now_unix_ms: default_unix_ms }
    }

    /// Test seam: fixed wall clock.
    pub fn with_clock(launch_id: LaunchId, now_unix_ms: UnixMillisFn) -> Self {
        Self { launch_id, runs: Vec::new(), now_unix_ms }
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.runs.iter()
    }

    pub fn apply(&mut self, event: RunEvent) {
        match event {
            RunEvent::Started { pane, command, cwd, at_ms, inferred } => {
                // One Run per Pane at a time: an unfinished predecessor is Abandoned.
                self.abandon_running_in(pane, at_ms);
                let unix_ms = (self.now_unix_ms)();
                self.runs.push(Run::start(
                    self.launch_id, pane, command, cwd, at_ms, unix_ms, inferred,
                ));
            }
            RunEvent::Finished { pane, exit_code, at_ms } => {
                // An orphan Finished (no Started) is dropped: never invent half a Run.
                if let Some(run) = self.running_in_mut(pane) {
                    run.finish(exit_code, at_ms);
                }
            }
            RunEvent::PaneClosed { pane, at_ms } => self.abandon_running_in(pane, at_ms),
        }
    }

    fn running_in_mut(&mut self, pane: PaneKey) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .rev()
            .find(|r| r.pane == pane && r.state == RunState::Running)
    }

    fn abandon_running_in(&mut self, pane: PaneKey, at_ms: u64) {
        if let Some(run) = self.running_in_mut(pane) {
            run.abandon(at_ms);
        }
    }
}

fn default_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
```

在 `ledger.rs` 的 `mod tests` 里补 `use crate::run::LaunchId;`（测试已引用）。

- [ ] **Step 6: 运行测试**

Run: `cargo test -p run_ledger`
Expected: 7 个测试全 PASS。

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/run_ledger
git commit -m "feat(run_ledger): add Run domain types and the Run state machine"
```

---

## Task 2: 入账即脱敏

**Files:**
- Create: `crates/run_ledger/src/redact.rs`

- [ ] **Step 1: 写失败的黄金样例测试**

```rust
#[cfg(test)]
mod tests {
    use super::redact_command;

    #[test]
    fn env_prefix_keeps_key_drops_value() {
        assert_eq!(
            redact_command("AWS_SECRET_ACCESS_KEY=abc123 aws s3 ls"),
            "AWS_SECRET_ACCESS_KEY=… aws s3 ls"
        );
    }

    #[test]
    fn sensitive_flag_values_are_dropped() {
        assert_eq!(
            redact_command("gh auth login --token ghp_averyrealtoken0000"),
            "gh auth login --token …"
        );
        assert_eq!(
            redact_command("curl -H \"Authorization: Bearer eyJhbGciOiJIUzI1NiJ9\" https://api.example.com"),
            "curl -H \"Authorization: …\" https://api.example.com"
        );
    }

    #[test]
    fn url_userinfo_and_secret_query_are_dropped() {
        assert_eq!(
            redact_command("git clone https://user:hunter2@example.com/x.git"),
            "git clone https://user:…@example.com/x.git"
        );
        assert_eq!(
            redact_command("curl 'https://example.com/a?token=abcdefghijklmnopqrstuvwxyz&x=1'"),
            "curl 'https://example.com/a?token=…&x=1'"
        );
    }

    #[test]
    fn high_entropy_bare_tokens_are_dropped() {
        assert_eq!(
            redact_command("deploy sk-live-4f9a8c2b7e1d6a3f5c8b9e0d1a2b3c4d"),
            "deploy …"
        );
    }

    #[test]
    fn ordinary_commands_are_untouched() {
        for cmd in ["cargo build --release", "npm test", "git commit -m \"fix: thing\""] {
            assert_eq!(redact_command(cmd), cmd, "must not mangle ordinary commands");
        }
    }

    #[test]
    fn long_command_is_truncated_to_256_chars() {
        let long = format!("echo {}", "a".repeat(400));
        let out = redact_command(&long);
        assert!(out.chars().count() <= 256);
        assert!(out.ends_with('…'));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p run_ledger redact`
Expected: FAIL —— `redact_command` 未定义。

- [ ] **Step 3: 实现 `redact_command`**

实现要求（按顺序处理，全部纯字符串处理，不引入 regex 依赖以保持 crate 轻量；如需 regex 则加 `regex.workspace = true`）：

1. 按空白切 token，逐 token 处理，最后用单空格重组前先记录原始分隔（简化：允许重组为单空格，测试用例已按此写）。
2. token 匹配 `^[A-Z][A-Z0-9_]*=` → 保留 `KEY=` 并追加 `…`。
3. 上一个 token 属于 `--token | --password | --api-key | --secret | -p | --pass`（大小写不敏感）→ 当前 token 整体替换为 `…`。
4. token 内含 `Authorization:` → 保留到冒号，其后替换 `…`（保留可能的收尾引号）。
5. token 形如 URL（含 `://`）→ 处理 `user:pass@` 的 pass 段与 query 中 `token=|key=|sig=|secret=` 的值。
6. 剩余 token：长度 > 20 且仅由 `[A-Za-z0-9+/=_-]` 组成且同时含字母与数字 → 替换为 `…`。
7. 最终字符串按**字符**截断到 255 并追加 `…`。

`docs` 注释里必须写明：**这是启发式，不是保证**（spec §4）。

在 `run_ledger.rs` 加 `pub use redact::redact_command;`。

- [ ] **Step 4: 运行测试**

Run: `cargo test -p run_ledger`
Expected: 全部 PASS（13 个）。

- [ ] **Step 5: 在 `Ledger::apply` 里接上脱敏（入账即脱敏）**

`RunEvent::Started` 分支里，`command` 改为 `redact_command(&command)`，并加开关：

```rust
pub struct Ledger {
    // …
    redact: bool,
}
```

`Ledger::new` 默认 `redact: true`；新增 `pub fn set_redact(&mut self, on: bool)`。补一个测试：`redact: false` 时原文保留。

- [ ] **Step 6: 运行测试 + Commit**

```bash
cargo test -p run_ledger
git add crates/run_ledger
git commit -m "feat(run_ledger): redact command lines at capture time"
```

---

## Task 3: 保留裁剪、Attention、徽标聚合

**Files:**
- Modify: `crates/run_ledger/src/ledger.rs`

- [ ] **Step 1: 写失败的测试**

```rust
    fn fixed_clock() -> u64 { 1_700_000_000_000 }

    #[test]
    fn prune_drops_runs_older_than_the_window() { /* 8 天前一条 + 今天一条 → 只剩今天 */ }

    #[test]
    fn prune_caps_total_runs() { /* 塞 600 条，max=500 → 剩最新 500 条 */ }

    #[test]
    fn failed_runs_always_enter_attention() { /* 100ms 的 Failed 也进 */ }

    #[test]
    fn short_success_does_not_enter_attention() { /* 1s 成功，阈值 5s → 不进 */ }

    #[test]
    fn long_success_enters_attention() { /* 6s 成功 → 进 */ }

    #[test]
    fn finishing_in_a_focused_pane_is_seen_immediately() {
        // apply(Finished) 时 focused_pane == 该 pane 且窗口活跃 → 不进 Attention
    }

    #[test]
    fn focus_clears_all_pending_attention_for_that_pane() {
        // 同一 pane 两条 Failed → mark_pane_seen(p) → Attention 为空
    }

    #[test]
    fn badge_prefers_failure_over_running_over_success() {
        // 一个 pane 集合里 1 Failed + 1 Running + 1 Succeeded → BadgeKind::Failed，count = 1
    }

    #[test]
    fn badge_counts_only_its_own_kind() {
        // 2 Failed + 1 Running → Failed, count 2
    }

    #[test]
    fn running_badge_reports_elapsed_of_the_oldest_running_run() { /* 计时取最早那条 */ }

    #[test]
    fn no_badge_when_nothing_pending() { assert!(ledger.badge_for(&[pane()], 0).is_none()); }

    #[test]
    fn loaded_history_is_seen_so_badges_never_resurrect_after_restart() {
        // Ledger::load_history(vec![failed_run]) → Attention 为空
    }

    #[test]
    fn loaded_running_becomes_abandoned() {
        // 磁盘上有一条 state=Running 的 Run（崩溃 / kill -9 造成）
        // load_history 后它变成 Abandoned，不会触发 ● 徽标
        // （原因：started_at_mono_ms 是 #[serde(skip)]，加载后为 0，
        //  若不转 Abandoned 则计时异常）
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p run_ledger ledger`
Expected: FAIL（`prune` / `badge_for` / `mark_pane_seen` / `load_history` 未定义）。

- [ ] **Step 3: 实现**

```rust
/// Retention policy, from settings.
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub days: u64,
    pub max_runs: usize,
}

impl Default for Retention {
    fn default() -> Self { Self { days: 7, max_runs: 500 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind { Running, Succeeded, Failed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    /// How many runs of `kind` this badge stands for (≥ 1).
    pub count: usize,
    /// For `Running`: millis elapsed on the oldest running Run. Else 0.
    pub elapsed_ms: u64,
}

impl Ledger {
    /// Attention 的成功阈值（复用 `notify_on_command_finish_secs`）。
    pub fn set_success_threshold_secs(&mut self, secs: u64);
    pub fn set_retention(&mut self, retention: Retention);

    /// 结束事件发生时，若该 pane 正被 focus 且窗口活跃，则直接标记为已看过。
    pub fn set_focus(&mut self, pane: Option<PaneKey>, window_active: bool);

    /// focus 一个 pane：清空该 pane 的**全部**待看 Run。
    pub fn mark_pane_seen(&mut self, pane: PaneKey);
    /// 面板点击某条 Run 时用。
    pub fn mark_run_seen(&mut self, id: RunId);

    /// 已结束且未看过（Failed 无阈值；Succeeded 需 ≥ 阈值；Unknown 同 Succeeded；
    /// Abandoned 不进 Attention —— 它是进程没了，不是「跑完了等你看」）。
    pub fn attention(&self) -> impl Iterator<Item = &Run>;

    /// 给一组 PaneKey（一个 tab 的全部 pane）算徽标。
    /// 优先级 Failed > Running > Succeeded，count 只数同类。
    pub fn badge_for(&self, panes: &[PaneKey], now_mono_ms: u64) -> Option<Badge>;

    /// 时间窗 + 条数双约束，先到先裁。
    pub fn prune(&mut self);

    /// 启动时载入历史：全部标记 `seen = true`（Attention 不跨重启）；
    /// 状态为 `Running` 的 Run 强制转为 `Abandoned`（spec §3.1：徽标不会在启动时凭历史数据出现）。
    pub fn load_history(&mut self, runs: Vec<Run>);
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p run_ledger`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/run_ledger
git commit -m "feat(run_ledger): add retention, Attention, and badge aggregation"
```

---

## Task 4: `runs.json` 读写

**Files:**
- Create: `crates/run_ledger/src/store.rs`
- Modify: `crates/run_ledger/Cargo.toml`（`[dev-dependencies] tempfile = "3"`，并在根 `Cargo.toml` 的 `[workspace.dependencies]` 加 `tempfile = "3"`）

- [ ] **Step 1: 写失败的测试**

```rust
#[cfg(test)]
mod tests {
    // 1. save_then_load_round_trips
    // 2. load_missing_file_returns_empty      —— 首次启动
    // 3. corrupt_file_is_renamed_to_bak_and_load_is_empty
    //    断言 runs.json.bak 存在，且返回空，且不 panic
    // 4. unknown_version_is_treated_as_corrupt
    // 5. save_merges_with_on_disk_runs_by_id  —— 模拟并发实例：先写 A，再用只含 B 的 ledger save，
    //    重新 load 应同时含 A 和 B，且按 started_at_unix_ms 排序
    // 6. save_applies_retention_before_writing —— 600 条进，磁盘上只剩 500
    // 7. seen_and_anchor_fields_are_not_serialized —— 读 JSON 文本断言不含 "seen"
    // 8. unix_permissions_are_0600（#[cfg(unix)]）
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p run_ledger store`
Expected: FAIL。

- [ ] **Step 3: 实现**

```rust
//! `runs.json` persistence: versioned, atomic, corruption-tolerant.

pub const RUNS_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct RunsFile {
    pub version: u32,
    /// First-persist notice has been shown (Task 8 reads this).
    #[serde(default)]
    pub announced: bool,
    pub runs: Vec<Run>,
}

/// 读入；文件缺失 → (empty, announced=false)；损坏或版本不认 → 重命名为 `.bak` 后返回空。
/// **绝不返回 Err**：启动路径不允许被台账阻塞（spec §5）。
/// 返回 `(runs, announced)`。
pub fn load_runs(path: &Path) -> (Vec<Run>, bool);

/// 写盘：先重读磁盘按 `RunId` 求并集（多实例并发，spec §4）→ 按 `started_at_unix_ms`
/// 排序 → 应用 `Retention` → 写临时文件 → `set_permissions(0o600)`（unix）→ `rename`。
/// `announced` 字段会被保留到磁盘。
pub fn save_runs(path: &Path, runs: &[Run], retention: Retention, announced: bool) -> std::io::Result<()>;

/// 与 `session.json` 同目录。
pub fn default_runs_path(config_dir: &Path) -> PathBuf { config_dir.join("runs.json") }
```

- [ ] **Step 4: 运行测试 + Commit**

```bash
cargo test -p run_ledger
git add Cargo.toml Cargo.lock crates/run_ledger
git commit -m "feat(run_ledger): persist runs.json atomically with merge and retention"
```

---

## Task 5: 设置键与 `inject_osc133` 默认值变更

**Files:**
- Modify: `crates/sleipnir_settings/src/sleipnir_settings.rs`（结构体 ~line 150-185、`Default` ~line 288、file schema ~line 495/526、apply ~line 601/661、测试 ~line 685/922）
- Modify: `docs/settings.example.json:43`
- Modify: `README.md`（Config 表）

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn run_ledger_defaults_to_persist() {
        let s = TerminalSettings::default();
        assert_eq!(s.run_ledger, RunLedgerMode::Persist);
        assert_eq!(s.run_ledger_retention_days, 7);
        assert_eq!(s.run_ledger_max_runs, 500);
        assert!(s.run_ledger_redact);
    }

    #[test]
    fn inject_osc133_defaults_on() {
        assert!(TerminalSettings::default().inject_osc133);
    }

    #[test]
    fn run_ledger_mode_parses_from_file() {
        // {"run_ledger": "memory"} → RunLedgerMode::Memory
        // {"run_ledger": "off"}    → RunLedgerMode::Off
        // 非法值 → 保持默认，且不 panic
    }
```

**注意**：现有测试 `inject_osc133_defaults_off`（`sleipnir_settings.rs:922`）与新默认值冲突，本步把它改写为上面的 `inject_osc133_defaults_on`，并同步 `:685` 附近构造 `Some(false)` 的固件。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p sleipnir_settings`
Expected: FAIL（`run_ledger` 字段不存在；`inject_osc133_defaults_off` 仍在）。

- [ ] **Step 3: 实现**

```rust
/// Where the Run Ledger keeps its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunLedgerMode {
    /// 不采集、无 UI、不读写 runs.json（文件保留）。
    Off,
    /// 采集并显示，但不读写 runs.json（磁盘文件原样保留）。
    Memory,
    #[default]
    /// 采集、显示、读写 runs.json。
    Persist,
}
```

`TerminalSettings` 增加：`run_ledger: RunLedgerMode`、`run_ledger_retention_days: u64`、
`run_ledger_max_runs: usize`、`run_ledger_redact: bool`；`Default` 分别为
`Persist` / `7` / `500` / `true`；`inject_osc133` 默认改 `true`。File schema 与 apply 路径按现有
四处模式同步（`Option<T>` 字段 + `if let Some(v)` 赋值）。

`docs/settings.example.json`：`"inject_osc133": false` → `true`，并加四个新键。
`README.md` Config 表新增四行，`inject_osc133` 一行标注默认值变化。

- [ ] **Step 4: 运行测试**

Run: `cargo test -p sleipnir_settings`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_settings docs/settings.example.json README.md
git commit -m "feat(settings): add run_ledger keys and turn inject_osc133 on by default

BREAKING: terminal.inject_osc133 now defaults to true so the Run Ledger gets
real command boundaries. Set it to false to restore detect-only behavior."
```

---

## Task 6: `session.json` 增加 `pane_key`

**Files:**
- Modify: `crates/sleipnir_ui/src/session.rs`
- Modify: `crates/sleipnir_ui/src/pane_tree.rs`（`PaneNode::Leaf` 增加 `pane_key: PaneKey`，影响 13 处构造/match）
- Modify: `crates/sleipnir_ui/src/app_shell.rs`（`materialize_tree` ~line 852、`snapshot_tree` ~line 3635）
- Modify: `crates/sleipnir_ui/Cargo.toml`（`uuid.workspace = true`）

- [ ] **Step 1: 写失败的测试**（`session.rs` 的 `mod tests`）

```rust
    #[test]
    fn leaf_without_pane_key_still_loads() {
        // 旧格式 JSON（Leaf 只有 id/cwd）→ 反序列化成功，pane_key == None
    }

    #[test]
    fn pane_key_round_trips() {
        // 带 pane_key 的 session 存→读，key 不变
    }

    #[test]
    fn restore_preserves_pane_key_and_new_panes_get_fresh_ones() {
        // 同一 session 连续两次 restore，pane_key 不变；新建 leaf 的 key 与之不同
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p sleipnir_ui session`
Expected: FAIL。

- [ ] **Step 3: 实现**

`SessionNode::Leaf` 增加：

```rust
        /// Stable identity for this Pane across restarts (Run Ledger ownership key).
        /// `None` for sessions written before the Run Ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_key: Option<Uuid>,
```

`SESSION_VERSION` **保持 1**（纯增字段，向后兼容，旧版读新文件会忽略未知字段）。
运行时 `Pane` 结构增加 `pane_key: Uuid`（新建时 `Uuid::new_v4()`，restore 时若
`pane_key.is_none()` 也生成新的 —— 那种 pane 只是拿不到历史归属）。

- [ ] **Step 4: 运行测试**

Run: `cargo test -p sleipnir_ui`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_ui Cargo.lock
git commit -m "feat(session): give each pane a stable pane_key across restarts"
```

---

## Task 7: `terminal` 发 `RunEvent`

**Files:**
- Create: `crates/terminal/src/run_tracker.rs`
- Modify: `crates/terminal/src/terminal.rs`（`Event` 枚举 ~line 702、`record_osc133_marker` ~line 2018、`poll_command_finish` ~line 2130、`TerminalBackendEvent::ChildExit` ~line 1722）
- Modify: `crates/terminal/Cargo.toml`（`run_ledger.workspace = true`）

**为什么单独一个 `run_tracker.rs`**：`terminal.rs` 已 3700+ 行且锁着 gpui `Context`。跟踪逻辑（marker 序列 → 事件、命令行文本抓取的纯部分、busy 降级）是纯状态机，单独文件可脱离 gpui 测试。

- [ ] **Step 1: 写失败的测试**

```rust
#[cfg(test)]
mod tests {
    use super::{RunTracker, TrackerOut};
    use crate::Osc133Kind::*;

    #[test]
    fn c_then_d_yields_start_then_finish() {
        let mut t = RunTracker::default();
        assert_eq!(t.on_marker(CommandStart, 10), None);
        // 命令行文本由调用方在 C 时从网格读出并传入
        assert!(matches!(
            t.on_marker_with_command(CommandExecuted, 11, Some("cargo build".into()), 1_000),
            Some(TrackerOut::Started { .. })
        ));
        assert!(matches!(
            t.on_marker(CommandFinished { status: Some(0) }, 40_000),
            Some(TrackerOut::Finished { exit_code: Some(0), .. })
        ));
    }

    #[test]
    fn d_without_c_is_dropped() { /* 只发 D → None */ }

    #[test]
    fn empty_command_text_falls_back_to_placeholder() {
        // C 时拿不到文本 → Started.command == "(无法识别的命令)"
    }

    #[test]
    fn multiline_command_keeps_first_line_with_ellipsis() {
        // "for i in 1 2 3\ndo echo $i\ndone" → "for i in 1 2 3…"
    }

    #[test]
    fn busy_probe_only_fires_when_osc133_is_silent() {
        // 已经因 OSC 133 处于 Running 时，busy 探测不得再造一条 Run
    }

    #[test]
    fn busy_probe_start_and_stop_are_inferred() {
        // busy=true → Started{inferred:true}；busy=false → Finished{exit_code:None}
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p terminal run_tracker`
Expected: FAIL。

- [ ] **Step 3: 实现 `run_tracker.rs`**

```rust
//! Turn OSC 133 markers (and the busy probe, when markers are absent) into
//! Run start/finish facts. Pure state machine — no gpui, no PTY.

pub enum TrackerOut {
    Started { command: String, inferred: bool, at_ms: u64 },
    Finished { exit_code: Option<i32>, at_ms: u64 },
}

#[derive(Default)]
pub struct RunTracker {
    running: bool,
    /// True while the current Run came from OSC 133 (blocks the busy fallback).
    from_osc133: bool,
}

pub const UNRECOGNIZED_COMMAND: &str = "(无法识别的命令)";

impl RunTracker {
    pub fn on_marker(&mut self, kind: Osc133Kind, at_ms: u64) -> Option<TrackerOut>;
    pub fn on_marker_with_command(
        &mut self, kind: Osc133Kind, at_ms: u64, command: Option<String>, /* … */
    ) -> Option<TrackerOut>;
    /// `busy` 来自 `terminal_looks_busy`；`command` 来自 `foreground_process_command_name`。
    pub fn on_busy_change(&mut self, busy: bool, command: Option<String>, at_ms: u64)
        -> Option<TrackerOut>;
}

/// 命令行文本规范化：取首行，去首尾空白，多行/超长追加 `…`。
pub fn normalize_command(raw: Option<&str>) -> String;
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p terminal run_tracker`
Expected: PASS。

- [ ] **Step 5: 接线到 `terminal.rs`（emit gpui 事件）**

1. `Event` 枚举加：

```rust
    /// A command started in this terminal (Run Ledger).
    RunStarted { command: String, cwd: Option<PathBuf>, inferred: bool },
    /// The current command finished. `exit_code` is `None` when unknown.
    RunFinished { exit_code: Option<i32> },
```

2. `Terminal` 加字段 `run_tracker: RunTracker` 与 `started_at: Instant`（单调时钟基准）。
3. **`record_osc133_marker` 本身没有 `cx` 参数**（display-only 路径 `ingest_osc133` 也没有）。
   解决方案：`RunTracker` 产出 `Option<TrackerOut>` 不 emit；**统一在有 `cx` 的调用点 drain 并 emit**：
   - 真 PTY 路径：`terminal.rs:1726`（`TerminalBackendEvent` match 内，已有 `cx`）—— 在
     `osc_custom` 分支处理完 marker 后，检查 `tracker.take_output()` 并 `cx.emit(...)`。
   - Display-only 路径：只 feed tracker、不 emit（这条路径本来就是备用，不应产生 Run）。
   - `poll_command_finish`：它被从 `sleipnir_ui.rs:289` 用 `terminal.update(cx, |t, _| ...)` 调用，
     里面的 `_` 实际上是一个丢弃的 `&mut Context<Terminal>`。改为不丢弃它，
     调用 `on_busy_change` 后若返回 `Some(TrackerOut)` 则 `cx.emit(...)`。
   - 遇 `CommandExecuted` 时用 `prompt_markers` 里最近的 `CommandStart`（line/column）
     到当前光标位置从网格读出命令行文本，交给 `on_marker_with_command`。
   - 网格读取用 `Term` 已有的行遍历（参考 `terminal.rs` 里导出 scrollback 的实现）。
   - 读失败/为空 → `UNRECOGNIZED_COMMAND`（Task 7 Step 1 已有测试）。
4. `poll_command_finish` 内的 busy 迁移处**追加**调用 `on_busy_change`（保留原返回值语义，
   通知功能不变），并 emit。
5. `TerminalBackendEvent::ChildExit`：若 tracker 仍 `running`，emit `RunFinished { exit_code: None }`。

- [ ] **Step 6: 编译 + 全量测试**

Run: `cargo test -p terminal && cargo build -p sleipnir`
Expected: PASS / 编译通过。

- [ ] **Step 7: Commit**

```bash
git add crates/terminal Cargo.lock
git commit -m "feat(terminal): emit RunStarted/RunFinished from OSC 133 and the busy probe"
```

---

## Task 8: 全局 Ledger + 事件接线 + 落盘

**Files:**
- Create: `crates/sleipnir_ui/src/run_ledger_global.rs`
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs`（`TermViewEvent` + `attach_terminal` 订阅 ~line 275-320）
- Modify: `crates/sleipnir_ui/src/app_shell.rs`（`cx.subscribe_in` ~line 514）

- [ ] **Step 1: 写失败的测试**（`run_ledger_global.rs` 的纯部分）

```rust
    #[test]
    fn mode_off_drops_events() { /* Off 下 apply 不入账 */ }

    #[test]
    fn mode_memory_never_touches_disk() {
        // tempdir 里预置 runs.json → Memory 模式 load 不读、save 不写，文件内容不变
    }

    #[test]
    fn switching_to_off_clears_memory_and_keeps_the_file() { /* spec §4 运行时改值 */ }

    #[test]
    fn switching_to_persist_loads_history_as_seen() { /* 载入即已看过 */ }

    #[test]
    fn first_persist_write_notifies_exactly_once() {
        // 首次落盘置 flag 并返回「需告知」，第二次不再返回
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p sleipnir_ui run_ledger_global`
Expected: FAIL。

- [ ] **Step 3: 实现**

```rust
//! GPUI global that owns the process-wide Ledger and its debounced writer.

pub struct RunLedgerGlobal {
    ledger: Ledger,
    mode: RunLedgerMode,
    path: PathBuf,
    dirty: bool,
    /// Set after the first successful persist so the notice fires once.
    announced: bool,
    _flush: Task<()>,
}

impl Global for RunLedgerGlobal {}

impl RunLedgerGlobal {
    /// 启动时：读 `TerminalSettings::get_global` 并调用 `set_retention` / `set_redact` /
    /// `set_success_threshold_secs`；Persist 读盘并 `load_history`；Memory/Off 不读。
    pub fn init(cx: &mut App);
    pub fn apply(&mut self, event: RunEvent, cx: &mut App);
    pub fn set_mode(&mut self, mode: RunLedgerMode, cx: &mut App);
    /// ⌘⇧R 设置重载时调用：重新读 settings 并同步 retention/redact/threshold/mode。
    pub fn reload_settings(&mut self, cx: &mut App);
    /// 去抖 2s（`cx.background_executor().timer`）；写失败 → 降级为 Memory + 记一条日志。
    fn schedule_flush(&mut self, cx: &mut App);
    pub fn flush_now(&mut self);
    pub fn clear(&mut self, cx: &mut App);
    pub fn badge_for(&self, panes: &[PaneKey], now_ms: u64) -> Option<Badge>;
}
```

首次落盘告知：`announced == false` 且模式为 `Persist` 且本次写盘成功 → 展示一次性提示
（复用现有的 copy-toast / 通知机制，文案：「Sleipnir 开始在 `runs.json` 里记录你跑过的命令
（脱敏后的命令行 + 耗时 + 退出码，不含输出）。设置 `run_ledger` 可关闭。」），并把
`announced` 持久化进 `runs.json` 顶层（`announced: bool`）。

- [ ] **Step 4: 接线事件**

1. `TermViewEvent` 加 `RunStarted { command, cwd, inferred }` / `RunFinished { exit_code }`。
2. `sleipnir_ui.rs::attach_terminal` 的 `match event` 里新增两个分支，转成 `TermViewEvent` 并
   `cx.emit`。
3. `app_shell.rs` 的 `cx.subscribe_in` 里新增分支：用**发事件的那个 pane 的 `pane_key`** 构造
   `RunEvent` 并交给 `RunLedgerGlobal`，然后 `cx.notify()`。
4. 关 pane / 关 tab / 关窗口的现有路径追加 `RunEvent::PaneClosed`。
5. focus 切换处调用 `set_focus` 与 `mark_pane_seen`；窗口 activate 时同样。
6. 应用退出路径（现有 session 保存处）调用 `flush_now()`。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p sleipnir_ui && cargo build -p sleipnir`
Expected: PASS。

- [ ] **Step 6: 手动验证**

```bash
cargo run -p sleipnir
# 在一个 tab 里跑 `sleep 8`，切到另一个 tab，回来
# 关掉应用，检查 runs.json
cat ~/.config/sleipnir/runs.json | head -30
```
Expected: 有一条 `sleep 8`、`state: "succeeded"`、`duration` 约 8s；文件里**没有** `seen` 字段。

- [ ] **Step 7: Commit**

```bash
git add crates/sleipnir_ui
git commit -m "feat(ui): wire terminal Run events into a process-wide Run Ledger"
```

---

## Task 9: Tab / Pane 徽标 + 拆出 tab strip

**Files:**
- Create: `crates/sleipnir_ui/src/chrome/tab_badge.rs`
- Create: `crates/sleipnir_ui/src/chrome/tab_strip.rs`（从 `app_shell.rs:3693-3900` 附近迁出 tab 渲染）
- Modify: `crates/sleipnir_ui/src/chrome/mod.rs`, `crates/sleipnir_ui/src/app_shell.rs`

- [ ] **Step 1: 写失败的测试**（纯映射函数，不渲染）

```rust
    #[test]
    fn badge_colors_come_from_the_palette() {
        // Failed → palette 的 red/accent 语义色，Running → yellow，Succeeded → green
    }

    #[test]
    fn running_badge_formats_elapsed_as_mm_ss() {
        assert_eq!(format_elapsed(134_000), "2:14");
        assert_eq!(format_elapsed(59_000), "0:59");
        assert_eq!(format_elapsed(3_601_000), "60:01");
    }

    #[test]
    fn count_is_hidden_when_one_and_shown_when_many() {
        assert_eq!(badge_label(BadgeKind::Failed, 1), "✗");
        assert_eq!(badge_label(BadgeKind::Failed, 2), "✗2");
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p sleipnir_ui tab_badge`
Expected: FAIL。

- [ ] **Step 3: 实现 `tab_badge.rs`**（`format_elapsed` / `badge_label` / `badge_color`）。

- [ ] **Step 4: 运行测试**

Run: `cargo test -p sleipnir_ui tab_badge`
Expected: PASS。

- [ ] **Step 5: 把 tab strip 渲染迁到 `chrome/tab_strip.rs`**

**不是「纯搬迁」而是「内部提取」**：tab strip 渲染是 `impl Render for AppShell` 里的内联代码
（约 3659–4035 行），依赖私有字段 `hovered_tab` / `rename` / `tab_scroll_handle` /
`drag` / `bell_flash_tabs` 等。做法：
- 在 `chrome/tab_strip.rs` 里写 `impl AppShell { pub(crate) fn render_tab_strip(...) }`。
- 把上述字段从 `pub(super)` / private 改为 `pub(crate)`（先列清单，逐个改）。
- `render()` 里原地替换为 `self.render_tab_strip(window, cx, ...)`。
- 移动前后 `cargo build` 必须都通过，且**不改行为**。
不要顺手重构其它部分（spec §2 架构决策 2：范围只限本次要碰的）。

- [ ] **Step 6: 在 tab chip 里渲染徽标**

标题左侧插入徽标：`RunLedgerGlobal::badge_for(该 tab 全部 pane 的 pane_key)`。
`None` → 不渲染任何东西（不留占位，避免标题跳动：徽标出现时用固定宽度槽位）。
Running 徽标需要每秒重画 —— 复用现有的 blink/timer 机制，**只在存在 Running 徽标时**开启定时器。

- [ ] **Step 7: 编译 + 手动验证**

```bash
cargo build -p sleipnir && cargo run -p sleipnir
```
验收（对齐 spec 成功标准 1）：
- tab A 跑 `sleep 20`，切到 tab B → A 上出现 ● 与递增计时
- `sleep 20` 结束后不切回 A → A 上变 ✓（≥5s 阈值满足）
- tab C 跑 `false` → 立即 ✗（Failed 无阈值）
- 切回 A → A 的徽标消失，C 的还在
- `run_ledger: off` + 重载设置 → 徽标全消失

- [ ] **Step 8: Commit**

```bash
git add crates/sleipnir_ui
git commit -m "feat(chrome): show Run badges on tabs and split the tab strip out of app_shell"
```

---

## Task 10: `clear_run_ledger` action + 命令面板 / 菜单入口

**Files:**
- Modify: `crates/sleipnir_ui/src/keymap.rs`, `crates/sleipnir_ui/src/command_palette.rs`, `crates/sleipnir/src/app_menus.rs`, `README.md`

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn clear_run_ledger_is_a_known_action_name() {
        // command_palette.rs 的 CommandId::from_str 返回 Some
        assert!(CommandId::from_str("clear_run_ledger").is_some());
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p sleipnir_ui command_palette`
Expected: FAIL。

- [ ] **Step 3: 实现**

- `keymap.rs`：注册 `clear_run_ledger`（**不绑默认键位** —— 破坏性操作只走命令面板/菜单）。
- `command_palette.rs`：新增条目「Clear Run Ledger」。
- `app_menus.rs`：macOS 放 Shell 菜单、Windows/Linux 放 File 菜单，与 Export Scrollback 同组。
- 行为：清空内存台账 + 删除 `runs.json`（`Persist` 模式下），并复用现有确认弹窗（`confirm_close`
  用的那套）二次确认。
- `README.md` 的 `key_bindings` 可用 action 列表补 `clear_run_ledger`。

- [ ] **Step 4: 运行测试 + 手动验证**

```bash
cargo test -p sleipnir_ui && cargo run -p sleipnir
# 命令面板 → Clear Run Ledger → 确认 → 徽标清空
ls ~/.config/sleipnir/runs.json   # 应不存在
```

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_ui crates/sleipnir README.md
git commit -m "feat: add a Clear Run Ledger action to the palette and menus"
```

---

## Task 11: ADR-0009、ADR-0006 状态、glossary、CHANGELOG

**Files:**
- Create: `docs/adr/0009-run-ledger-persistence.md`
- Modify: `docs/adr/0006-tombstone-session-restore.md`, `docs/glossary.md`, `CHANGELOG.md`

- [ ] **Step 1: 写 ADR-0009**

必须包含（spec §3.4 / §4）：

- Context：审计结论「系统已知的事实没有出现在界面上」+ ADR-0006 的两条冲突决策。
- Decision：持久化脱敏后的命令行 + 元数据到 `runs.json`；**默认开**，理由是 `~/.zsh_history`
  本来就明文存完整命令行，本设计写的是其脱敏子集；三项配套硬条件（首次告知 / 一眼可关 /
  一键清空）缺一不可。
- **Supersedes ADR-0006** 的两条：grid-line tombstone → chrome 横幅；「完整命令行绝不持久化」。
- 回答 ADR-0006 的 open question：该 pane 历史 Run 数为 0 → 不显示横幅。
- Consequences：脱敏是启发式而非保证；`restore_session: false` 时不写 `pane_key`、tombstone 不可用；
  多实例靠合并写而非单实例锁。

- [ ] **Step 2: 改 ADR-0006 状态**

`**Status:** proposed` → `**Status:** superseded by [ADR-0009](0009-run-ledger-persistence.md)`，
并在开头加一段指向 0009 的说明。**不要标 accepted。**

- [ ] **Step 3: glossary 加四个术语**

`docs/glossary.md` 新增 Run / Ledger / Anchor / Attention，含 spec §2 的「避免」项
（Run 的 _避免_ 必须写明 Task 已被 `SpawnInTerminal` 占用）。

- [ ] **Step 4: CHANGELOG**

`## Unreleased` 下加 Features（Run Ledger、tab 徽标、`clear_run_ledger`）与
**Breaking Changes**（`terminal.inject_osc133` 默认 `true`；新增 `runs.json`，默认写盘、
`run_ledger: "off"` 可关）。

- [ ] **Step 5: 全量验证 + Commit**

```bash
cargo test --workspace && cargo build -p sleipnir
git add docs CHANGELOG.md
git commit -m "docs: record the Run Ledger decisions in ADR-0009 and the glossary"
```

---

## P0 完成条件

- [ ] `cargo test --workspace` 全绿；`cargo build -p sleipnir` 通过（macOS / Windows / Linux CI）
- [ ] 成功标准 1：不切 tab 就能看出哪个 pane 需要我（Task 9 Step 7 手动验收清单）
- [ ] 成功标准 4：`run_ledger: "off"` + `inject_osc133: false` 时行为与实现前一致
- [ ] `runs.json` 里没有 `seen` / `anchor` 字段，权限 `0600`，含脱敏后的命令行
- [ ] 首次落盘的一次性告知出现过一次，且第二次启动不再出现
- [ ] ADR-0006 状态是 `superseded by 0009`，不是 `accepted`
