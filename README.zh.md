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
- 原生 PTY / ConPTY 支持与会话恢复
- 智能粘贴、路径链接以及跟随系统的主题
- 滚动历史搜索、Diff 检查和命令运行记录
- 兼容 Zed 的 `terminal.*` 设置，支持热重载

## 安装

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

### Windows

从 [GitHub Releases](https://github.com/Maidang1/sleipnir/releases) 下载最新的 `Sleipnir-<ver>-windows-x64.exe`，然后运行即可。

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

Linux 发布版包含 x86_64 和 ARM64 的 `.deb` 安装包以及便携版压缩包。

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
- `restore_session`
- `confirm_close`
- `key_bindings`
- `terminal.bell`
- `notify_on_command_finish_secs`
- `run_ledger`

完整示例配置请参考 [`docs/settings.example.json`](docs/settings.example.json)。

## 快捷键

- 新建窗口：`⌘N` / `Ctrl+Shift+N`
- 新建标签页：`⌘T` / `Ctrl+Shift+T`
- 命令面板：`⌘⇧K` / `Ctrl+Shift+P`
- 搜索滚动历史：`⌘F` / `Ctrl+Shift+F`
- 主题重载：`⌘⇧R` / `Ctrl+Shift+R`

## 许可证

Sleipnir 采用 Apache 2.0 和 GPL v2 双许可证。详情见 [LICENSE-APACHE](LICENSE-APACHE) 和 [LICENSE-GPL](LICENSE-GPL)。
