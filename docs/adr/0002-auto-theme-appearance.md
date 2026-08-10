# Auto theme follows system Appearance via a dark/light theme pair

**Status:** accepted

Users want Harbor to follow the macOS system Appearance. Rather than making every
Theme internally appearance-aware, we introduce an **Auto Theme**: a user choice
that binds one dark Theme and one light Theme as a pair and swaps between them
when the system Appearance changes. Selecting a fixed Theme (e.g. `mocha`)
continues to ignore Appearance.

This changes the theme model: theme selection is no longer a single flat
`ThemeName`, but either a fixed Theme or an Auto pairing of two Themes.

## Consequences

- Settings must be able to express an Auto choice (a dark/light pair), not just
  one `ThemeName`.
- The app must observe system Appearance changes at runtime and re-resolve the
  active Palette without a settings reload.
- `cmd-shift-t` (previously "cycle theme") is freed for the conventional
  "reopen closed tab"; theme switching moves to a picker UI. (Reversible; not an
  ADR on its own.)
