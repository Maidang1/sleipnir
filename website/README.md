# Sleipnir website

Product landing page for [Sleipnir](https://github.com/Maidang1/sleipnir), designed in the spirit of [waku.sh](https://waku.sh/) (centered column, monochrome tokens, feature grid, FAQ).

Copy tracks the shipped product surface in the monorepo README (M0–M15): GPU terminal (Metal on macOS, Direct3D on Windows), tabs/splits/pane zoom, multi-window, themes, smart paste, session restore, path links, shell prompt jump, and optional Quick Terminal / opacity. Prebuilt downloads: macOS .dmg/.zip and Windows x64 .zip from GitHub Releases.

## Develop

```bash
cd website
npm install
npm run dev
```

Open http://localhost:3000.

## Build

```bash
npm run build
# output: dist/
npm run preview
```

Latest release download links are resolved client-side from the GitHub Releases API (`Maidang1/sleipnir`).

## Content map

| Surface | Source |
|---------|--------|
| Feature grid / hero / FAQ | `src/App.tsx` |
| One-line install (macOS `curl \| bash` / Windows `irm \| iex`) | `src/components/install-command.tsx` + `INSTALL_COMMANDS` in `src/lib/release.ts` |
| Download menu | `src/components/download-menu.tsx` + `src/lib/release.ts` |
| Meta / OG | `index.html` |
| Product docs (canonical detail) | repo root `README.md`, `docs/M*.md`, `docs/settings.example.json` |
