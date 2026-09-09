<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir 应用图标" width="160" height="160" />

# Sleipnir

面向 macOS、Windows 和 Linux 的原生终端。

基于 GPUI 的 GPU 渲染，支持标签页、分屏和多窗口会话，滚动与重绘保持流畅。

[功能](#功能) · [安装](#安装) · [构建](#构建) · [配置](#配置)

</div>

---

Sleipnir 是一个独立终端应用，基于 [GPUI](https://gpui.rs) 构建，并采用 fork 的终端后端来提供原生 PTY / ConPTY 行为。它强调响应速度、布局灵活性，以及让终端工作流像原生应用一样自然。

## 功能

- GPU 渲染终端，滚动和重绘更流畅
- 标签页、分屏和多窗口会话
- 原生 PTY / ConPTY 支持；每个新窗口从一个新标签页开始
- 智能粘贴、路径链接以及跟随系统的主题
- 滚动历史搜索、Diff 检查和内存中的命令运行状态
- 兼容 Zed 的 `terminal.*` 设置，支持热重载
- 可选的进程外插件：扩展面板、滚动历史内嵌内容和命令面板，默认关闭

重启后不恢复窗口布局或终端滚动历史。持久化命令历史和 Run Ledger 面板由
[可选的 Run Ledger 插件](crates/sleipnir_plugin_runledger/README.md)提供，需要单独安装并启用。

## 安装

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

### Windows

从 [GitHub Releases](https://github.com/Maidang1/sleipnir/releases) 下载最新的 `Sleipnir-<ver>-windows-x64.exe`（便携二进制）或 `Sleipnir-<ver>-windows-x64.zip`（便携压缩包），然后运行即可。

Windows 构建目前未做代码签名，首次运行可能出现 SmartScreen 提示 —— 点击「更多信息 → 仍要运行」即可继续。

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

Linux 发布版包含 x86_64 和 ARM64 的 `.deb` 安装包以及便携版压缩包。

各平台的更新机制不同：macOS 上的「Check for Updates」会在应用内原地自动更新；Windows 和 Linux 上的「Check for Updates」会打开 Releases 页面，需要手动下载最新版本。

## 构建

```bash
cargo run -p sleipnir
```

如果需要构建发布版二进制：

```bash
cargo build --release -p sleipnir
```

## 配置

配置文件位于：

- macOS / Linux：`~/.config/sleipnir/settings.json`
- Windows：`%APPDATA%\sleipnir\settings.json`

常用配置项包括：

- `theme` / `custom_theme`
- `confirm_close`
- `key_bindings`
- `terminal.bell`
- `notify_on_command_finish_secs`
- `run_ledger`

完整示例配置请参考 [`docs/settings.example.json`](docs/settings.example.json)，当前行为和已移除配置见
[`docs/settings.md`](docs/settings.md)。`run_ledger: "memory"` 在核心中采集命令事实；兼容旧配置的
`"persist"` 值也只保存在内存中，磁盘历史由可选插件管理。
插件开发、示例和本地未沙箱化的信任模型见 [`docs/plugins.md`](docs/plugins.md)。

## 快捷键

- 新建窗口：`⌘N` / `Ctrl+Shift+N`
- 新建标签页：`⌘T` / `Ctrl+Shift+T`
- 命令面板：`⌘⇧K` / `Ctrl+Shift+P`
- 搜索滚动历史：`⌘F` / `Ctrl+Shift+F`
- 主题重载：`⌘⇧R` / `Ctrl+Shift+R`

## 许可证

应用和终端相关 crate 声明为 `GPL-3.0-or-later`；本地 `gpui_platform` crate 和上游 GPUI 使用
Apache 2.0。这是不同组件各自的许可证，不代表整个应用可在两种许可证之间任选。
详情见 [LICENSE-GPL](LICENSE-GPL)、[LICENSE-APACHE](LICENSE-APACHE)、各 crate 的清单和
[UPSTREAM.md](UPSTREAM.md)。
