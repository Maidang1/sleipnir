# Sleipnir website

Product landing page for [Sleipnir](https://github.com/Maidang1/sleipnir), designed in the spirit of [waku.sh](https://waku.sh/) (centered column, monochrome tokens, feature grid, FAQ).

Copy tracks the shipped product surface in the monorepo README: a native GPU terminal for macOS, Windows, and Linux (Metal, Direct3D 11, and Vulkan); side tab rail or top strip grouped by git workspace; splits and pane zoom; multi-window; themes; smart paste; session restore; Run Ledger; path links; shell prompt jump; and optional Quick Terminal and opacity. Linux supports Wayland and X11, with Ubuntu 22.04+ officially supported and other glibc 2.35+ desktop distributions available on a best-effort basis.

GitHub Releases provide a macOS `.dmg`, a Windows x64 `.exe`, and Linux packages for both architectures:

- `sleipnir_<ver>_amd64.deb`
- `Sleipnir-<ver>-linux-x86_64.tar.gz`
- `sleipnir_<ver>_arm64.deb`
- `Sleipnir-<ver>-linux-aarch64.tar.gz`

The one-line installer detects macOS or Linux and verifies the selected artifact against its SHA-256 sidecar. In-place updates are macOS-only; Windows and Linux open GitHub Releases for manual updates.

## Develop

```bash
cd website
npm install
npm run dev
```

Open http://localhost:3000.

## Test and build

```bash
npm test
npm run build
# output: dist/
npm run preview
```

Latest release download links are resolved client-side from the GitHub Releases API (`Maidang1/sleipnir`). The pure release parser is covered by Vitest and deliberately ignores `.sha256` sidecars.

## Content map

| Surface | Source |
|---------|--------|
| Feature grid / hero / FAQ | `src/App.tsx` |
| Cross-platform one-line install (`curl \| bash`) | `src/components/install-command.tsx` + `INSTALL_COMMAND` in `src/lib/release.ts` |
| Architecture-labelled download menu | `src/components/download-menu.tsx` + `src/lib/release.ts` |
| Release parser tests | `src/lib/release.test.ts` |
| Meta / OG | `index.html` |
