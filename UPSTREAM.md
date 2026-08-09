# Upstream sync (ad-hoc)

jiajia-term copies selected crates from [Zed](https://github.com/zed-industries/zed).
There is **no** git submodule / subtree of the Zed monorepo.

## When to port

Only when you hit a bug fixed upstream, or intentionally refresh GPUI/terminal.

## Checklist

1. Temporary clone or use an existing Zed checkout (outside this repo).
2. Diff focus paths:
   - `crates/gpui/**`, `crates/gpui_macos/**`, `crates/gpui_platform/**`
   - Later: `crates/terminal/**`
3. Manually apply patches into this repo's `crates/`.
4. `cargo build -p jiajia_term` and smoke-run.
5. Note the approximate Zed commit/date and `alacritty_terminal` rev in the commit message.

## alacrity pin

See root `Cargo.toml` / terminal crate when M1 lands. Planned:

```toml
alacritty_terminal = { git = "https://github.com/zed-industries/alacritty", rev = "..." }
```

(not a dependency of the Zed monorepo itself)
