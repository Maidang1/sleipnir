import { useEffect, useState, type ReactNode } from 'react'
import {
  AppWindow,
  Columns2,
  Command,
  Download,
  HardDrive,
  Image as ImageIcon,
  Keyboard,
  Link2,
  Maximize2,
  Palette,
  RefreshCw,
  Search,
  Zap,
} from 'lucide-react'
import { Accordion } from '@/components/accordion'
import { DownloadMenu } from '@/components/download-menu'
import { InstallCommand } from '@/components/install-command'
import { fetchLatestRelease, GITHUB_URL, type LatestRelease } from '@/lib/release'

const HIGHLIGHTS = [
  { icon: Zap, label: 'GPU rendering' },
  { icon: Columns2, label: 'Tabs, splits & zoom' },
  { icon: AppWindow, label: 'Multi-window' },
  { icon: Palette, label: 'Adaptive themes' },
  { icon: ImageIcon, label: 'Smart paste' },
  { icon: Search, label: 'Find & palette' },
  { icon: HardDrive, label: 'Session restore' },
  { icon: Link2, label: 'Path links' },
]

const FEATURES = [
  {
    icon: Zap,
    title: 'Native down to the frame',
    body: 'Rust and GPUI, the GPU framework behind Zed. Metal on macOS. Smooth scrollback, ease-in-out cursor blink, and redraw under heavy output. No Electron tax.',
  },
  {
    icon: Columns2,
    title: 'Tabs, splits & pane zoom',
    body: 'Split right or down, jump tabs with ⌘1–9, move focus with ⌘⌥ arrows. Zoom a pane with ⌘⇧Enter; inactive splits dim so focus stays clear.',
  },
  {
    icon: AppWindow,
    title: 'Multi-window & font zoom',
    body: '⌘N opens an independent window with its own tabs and shells. Resize the grid with ⌘+ (and − / 0). Window-scoped, not written to settings.',
  },
  {
    icon: Palette,
    title: 'Themes that follow you',
    body: 'Catppuccin, Tokyo Night, Nord, Gruvbox, Solarized, GitHub Dark/Light. Set theme to auto and match system appearance.',
  },
  {
    icon: ImageIcon,
    title: 'Paste that understands the file manager',
    body: 'A screenshot on the clipboard becomes a quoted temp path. Finder selections paste as quoted paths. ⌃⌘V forces text-only.',
  },
  {
    icon: Keyboard,
    title: 'Keyboard first',
    body: 'Command palette (⌘⇧K), vi mode, configurable key bindings, find in scrollback, and close confirm when a job is running.',
  },
  {
    icon: Link2,
    title: 'Paths, links & shell markers',
    body: '⌘-click paths open in the default app. Optional system or visual bell. Jump previous/next shell prompt when OSC 133 markers are present.',
  },
  {
    icon: Maximize2,
    title: 'Daily extras without bloat',
    body: 'Quick Terminal, Quick Select mode, optional content opacity, and a macOS notification when a long command finishes while you are in another app.',
  },
  {
    icon: RefreshCw,
    title: 'Quietly current',
    body: 'Check for updates when you want (⌘⇧U). Downloads verify against a published SHA-256 sidecar before install. Session layout restores on launch.',
  },
]

const FAQ = [
  {
    q: 'Is this another Electron app?',
    a: 'No. Sleipnir is a native macOS binary rendered by GPUI (the same stack as Zed). The window is drawn by Metal, not a browser engine.',
  },
  {
    q: 'Where does config live?',
    a: (
      <>
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">
          ~/.config/sleipnir/settings.json
        </code>
        . Terminal keys are Zed-compatible; hot-reload with ⌘⇧R.
        Session layout restores from{' '}
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">
          session.json
        </code>{' '}
        in the same folder. See the repo{' '}
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">
          docs/settings.example.json
        </code>{' '}
        for keys like <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">confirm_close</code>,{' '}
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">path_links</code>, and{' '}
        <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">background_opacity</code>.
      </>
    ),
  },
  {
    q: 'Does it auto-update on launch?',
    a: 'No. Updates are manual via Sleipnir → Check for Updates… (⌘⇧U). Artifacts are verified with a .zip.sha256 sidecar before install.',
  },
  {
    q: 'macOS says the app is from an unidentified developer.',
    a: 'CI builds are ad-hoc signed. The one-line installer runs xattr -cr after copying to /Applications so Gatekeeper does not quarantine the download. If you installed from a .dmg or .zip instead, run xattr -cr /Applications/Sleipnir.app, then open the app.',
  },
  {
    q: 'Is Windows or Linux supported?',
    a: 'No. Sleipnir is macOS 14+ only. There are no Windows or Linux builds.',
  },
  {
    q: 'How is this different from Terminal.app / iTerm / Warp?',
    a: 'GPU-first rendering via GPUI, Zed-shaped settings, file-manager paste, multi-window, pane zoom, path links, and light shell integration, without an account, cloud features, or a shell-suite installer.',
  },
  {
    q: 'Is there AI built in?',
    a: 'No. Sleipnir stays a clean native terminal: no assistant, no auto-installed shell plugins, no graphics protocol platform.',
  },
]

function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <div className="font-mono text-[11px] tracking-[0.14em] text-muted-foreground/80 uppercase">
      {children}
    </div>
  )
}

function GitHubIcon({ className }: { className?: string }) {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" className={className}>
      <path
        fill="currentColor"
        d="M12 2A10 10 0 0 0 2 12c0 4.42 2.87 8.17 6.84 9.5c.5.08.66-.23.66-.5v-1.69c-2.77.6-3.36-1.34-3.36-1.34c-.46-1.16-1.11-1.47-1.11-1.47c-.91-.62.07-.6.07-.6c1 .07 1.53 1.03 1.53 1.03c.87 1.52 2.34 1.07 2.91.83c.09-.65.35-1.09.63-1.34c-2.22-.25-4.55-1.11-4.55-4.92c0-1.11.38-2 1.03-2.71c-.1-.25-.45-1.29.1-2.64c0 0 .84-.27 2.75 1.02c.79-.22 1.65-.33 2.5-.33s1.71.11 2.5.33c1.91-1.29 2.75-1.02 2.75-1.02c.55 1.35.2 2.39.1 2.64c.65.71 1.03 1.6 1.03 2.71c0 3.82-2.34 4.66-4.57 4.91c.36.31.69.92.69 1.85V21c0 .27.16.59.67.5C19.14 20.16 22 16.42 22 12A10 10 0 0 0 12 2"
      />
    </svg>
  )
}

export default function App() {
  const [release, setRelease] = useState<LatestRelease | null>(null)

  useEffect(() => {
    void fetchLatestRelease().then(setRelease)
  }, [])

  return (
    <div className="min-h-dvh">
      <div className="mx-auto w-full max-w-[1100px] border-border/70 md:border-x">
        {/* Header */}
        <header className="flex h-16 items-center justify-between px-5 md:px-10">
          <a href="/" className="flex items-center gap-2.5">
            <img
              src="/app-icon.png"
              alt=""
              className="size-8 rounded-[6px]"
              width={32}
              height={32}
            />
            <span className="text-[15px] font-semibold tracking-tight">Sleipnir</span>
          </a>
          <div className="flex items-center gap-5">
            <a
              href={GITHUB_URL}
              target="_blank"
              rel="noreferrer"
              aria-label="GitHub"
              className="rounded-full text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <GitHubIcon className="size-6" />
            </a>
            <DownloadMenu release={release} size="sm" align="end" />
          </div>
        </header>

        <main>
          {/* Hero */}
          <section className="px-5 pt-14 pb-14 md:px-10 md:pt-24">
            <div className="mb-7 inline-flex items-center gap-2 rounded-full border border-border px-3 py-1 text-xs text-muted-foreground">
              <Command className="size-3.5" />
              Built on GPUI · macOS
            </div>
            <h1 className="max-w-4xl text-4xl font-semibold tracking-[-0.03em] text-balance md:text-[3.4rem] md:leading-[1.04]">
              A fast native terminal for macOS.
            </h1>
            <p className="mt-5 max-w-[38rem] text-[17px] leading-relaxed text-pretty text-muted-foreground">
              GPU-rendered through GPUI. Tabs, splits, multi-window, adaptive themes,
              and session restore.
            </p>
            <div className="mt-8 flex flex-wrap items-center gap-x-5 gap-y-3">
              <DownloadMenu
                release={release}
                size="lg"
                align="start"
                showIcon
                className="h-10 px-4"
              />
              {release && (
                <span className="font-mono text-xs text-muted-foreground">
                  v{release.version}
                </span>
              )}
            </div>
            <InstallCommand className="mt-4" />

            <div className="mt-16">
              <SectionLabel>What you get</SectionLabel>
              <div className="mt-4 flex flex-wrap items-center gap-x-7 gap-y-4">
                {HIGHLIGHTS.map((h) => (
                  <div
                    key={h.label}
                    className="flex items-center gap-2 text-muted-foreground/80"
                  >
                    <h.icon className="size-[18px]" strokeWidth={1.75} />
                    <span className="text-sm">{h.label}</span>
                  </div>
                ))}
              </div>
            </div>
          </section>

          {/* Product screenshot */}
          <section>
            <img
              src="/app-screenshot.jpg"
              alt="Sleipnir running Claude Code and lazygit side by side"
              width={2000}
              height={1146}
              className="block h-auto w-full outline outline-1 outline-black/10 dark:outline-white/10"
            />
          </section>

          {/* Features */}
          <section className="border-t border-border">
            <div className="px-5 pt-14 md:px-10">
              <SectionLabel>Why native</SectionLabel>
            </div>
            <div className="mt-8 grid grid-cols-1 gap-px border-t border-border bg-border/70 sm:grid-cols-2 lg:grid-cols-3">
              {FEATURES.map((f) => (
                <div key={f.title} className="bg-background p-6 md:p-8">
                  <div className="flex items-center gap-2.5">
                    <f.icon className="size-4 text-muted-foreground" />
                    <h3 className="text-sm font-medium">{f.title}</h3>
                  </div>
                  <p className="mt-2.5 text-sm leading-relaxed text-muted-foreground">
                    {f.body}
                  </p>
                </div>
              ))}
            </div>
          </section>

          {/* Download */}
          <section id="download" className="border-t border-border px-5 py-16 md:px-10 md:py-20">
            <SectionLabel>Download</SectionLabel>
            <h2 className="mt-3 text-2xl font-semibold tracking-tight">Get Sleipnir</h2>
            <p className="mt-2 max-w-lg text-sm text-muted-foreground">
              Free and open source. One-line install below, or download a .dmg
              or .zip.
            </p>
            <InstallCommand className="mt-6" />
            <div className="mt-6 flex flex-wrap items-center gap-x-5 gap-y-3">
              <DownloadMenu
                release={release}
                size="lg"
                align="start"
                showIcon
                className="h-10 px-4"
              />
              {release && (
                <span className="font-mono text-xs text-muted-foreground">
                  v{release.version}
                </span>
              )}
              <a
                href={GITHUB_URL}
                target="_blank"
                rel="noreferrer"
                className="inline-flex h-10 items-center gap-1.5 rounded-lg px-3 text-sm text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                <Download className="size-4 opacity-0" aria-hidden />
                Source on GitHub
              </a>
            </div>
          </section>

          {/* FAQ */}
          <section className="border-t border-border px-5 py-16 md:px-10">
            <SectionLabel>Questions</SectionLabel>
            <div className="mt-6 max-w-2xl">
              <Accordion items={FAQ} />
            </div>
          </section>
        </main>

        <footer className="flex items-center gap-2 border-t border-border px-5 py-10 text-xs text-muted-foreground md:px-10">
          <img
            src="/app-icon.png"
            alt=""
            className="size-4 rounded-[4px] opacity-80 grayscale"
            width={16}
            height={16}
          />
          <span>© {new Date().getFullYear()} Sleipnir</span>
          <span className="text-border">·</span>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            className="hover:text-foreground"
          >
            GitHub
          </a>
        </footer>
      </div>
    </div>
  )
}
