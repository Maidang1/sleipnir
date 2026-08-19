# Sleipnir website

Product landing page for [Sleipnir](https://github.com/Maidang1/sleipnir), designed in the spirit of [waku.sh](https://waku.sh/) (centered column, monochrome tokens, feature grid, FAQ).

Copy tracks the shipped product surface in the monorepo README: GPU terminal (Metal on macOS, Direct3D 11 on Windows), side tab rail or top strip grouped by git workspace, splits/pane zoom, multi-window, themes, smart paste, session restore, Run Ledger, path links, shell prompt jump, and optional Quick Terminal / opacity. Prebuilt downloads are macOS .dmg/.zip and a Windows x64 zip from GitHub Releases. Linux is not supported.

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
| One-line install (macOS `curl \| bash`) | `src/components/install-command.tsx` + `INSTALL_COMMAND` in `src/lib/release.ts` |
| Download menu | `src/components/download-menu.tsx` + `src/lib/release.ts` |
| Meta / OG | `index.html` |
| Product docs (canonical detail) | repo root `README.md`, `README.zh.md`, `docs/settings.example.json` |
