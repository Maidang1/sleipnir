import { useEffect, useState } from 'react'
import {
  AppWindow,
  Columns2,
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
import { BootTerminal } from '@/components/boot-terminal'
import { ChangelogPage, LatestChangelog } from '@/components/changelog'
import { DownloadMenu } from '@/components/download-menu'
import { DownloadTargets } from '@/components/download-targets'
import { InstallCommand } from '@/components/install-command'
import { SectionHeading } from '@/components/section-heading'
import { StatusBar } from '@/components/status-bar'
import { TerminalWindow } from '@/components/terminal-window'
import { fetchLatestRelease, GITHUB_URL, type LatestRelease } from '@/lib/release'
import { Link, usePath } from '@/lib/router'

const HIGHLIGHTS = [
  { icon: Zap, label: 'gpu rendering' },
  { icon: Columns2, label: 'tabs & splits' },
  { icon: AppWindow, label: 'multi-window' },
  { icon: Palette, label: 'adaptive themes' },
  { icon: ImageIcon, label: 'smart paste' },
  { icon: Search, label: 'find & palette' },
  { icon: Maximize2, label: 'pane zoom' },
  { icon: Link2, label: 'path links' },
]

const FEATURES = [
  {
    title: 'native down to the frame',
    body: 'Rust and GPUI, the GPU framework behind Zed. Metal on macOS, Direct3D 11 on Windows, Vulkan on Linux. Smooth scrollback, ease-in-out cursor blink, redraw under heavy output. No Electron tax.',
  },
  {
    title: 'tabs, splits & pane zoom',
    body: 'The top tab strip groups tabs by workspace and shows the last two folders of the cwd. Split right or down, jump tabs with ⌘1–9 / Ctrl+Shift+1–9, zoom a pane with ⌘⇧Enter. Inactive splits dim so focus stays clear.',
  },
  {
    title: 'multi-window & font zoom',
    body: '⌘N or Ctrl+Shift+N opens an independent window with its own tabs and shells. Resize the grid with ⌘+ / − / 0. Window-scoped, never written to settings.',
  },
  {
    title: 'themes that follow you',
    body: 'Catppuccin, Tokyo Night, Nord, Gruvbox, Solarized, GitHub Dark/Light. Set theme to auto and match system appearance.',
  },
  {
    title: 'paste that understands files',
    body: 'A screenshot on the clipboard becomes a quoted temp path. File-manager selections paste as quoted paths. ⌃⌘V or Ctrl+Alt+V forces text-only.',
  },
  {
    title: 'keyboard first',
    body: 'Command palette (⌘⇧K / Ctrl+Shift+P), vi mode, configurable key bindings, find in scrollback, and close confirm while a job is running.',
  },
  {
    title: 'paths, links & shell markers',
    body: '⌘-click paths to open them in the default app. Optional system or visual bell. Jump previous/next shell prompt when OSC 133 markers are present.',
  },
  {
    title: 'daily extras without bloat',
    body: 'Core command facts power failure attention and completion notifications. The optional Run Ledger plugin adds a history panel and persistence: install and enable separately. Quick Terminal, Quick Select, and content opacity stay built in.',
  },
  {
    title: 'quietly current',
    body: 'Check for updates when you want (⌘⇧U). macOS in-place updates require a signed manifest and verified download; Windows and Linux open Releases. Every new window starts with a fresh tab; layouts and scrollback are not persisted.',
  },
]

const FAQ = [
  {
    q: 'Is this another Electron app?',
    a: 'No. Sleipnir is a native binary rendered by GPUI (the same stack as Zed). The window is drawn by Metal on macOS, Direct3D 11 on Windows, and Vulkan on Linux, not a browser engine.',
  },
  {
    q: 'Where does config live?',
    a: (
      <>
        <Chip>~/.config/sleipnir/settings.json</Chip> on macOS and Linux, or{' '}
        <Chip>%APPDATA%\sleipnir\settings.json</Chip> on Windows. Terminal keys are
        Zed-compatible; reload settings with ⌘⇧R / Ctrl+Shift+R. Restart the app after
        changing key bindings. Windows start fresh, without restoring prior layouts. See the repo{' '}
        <Chip>docs/settings.example.json</Chip> for keys like <Chip>confirm_close</Chip>,{' '}
        <Chip>path_links</Chip>, and <Chip>background_opacity</Chip>.
      </>
    ),
  },
  {
    q: 'Does it auto-update on launch?',
    a: 'No. Updates are manual via Sleipnir → Check for Updates… (⌘⇧U / Ctrl+Shift+U). macOS can install a verified .dmg in place when the release includes a signed update manifest; otherwise install manually. On Windows and Linux, the action opens GitHub Releases.',
  },
  {
    q: 'macOS says the app is from an unidentified developer.',
    a: 'CI builds are ad-hoc signed. The one-line installer runs xattr -cr after copying to /Applications so Gatekeeper does not quarantine the download. If you installed from a .dmg instead, run xattr -cr /Applications/Sleipnir.app, then open the app.',
  },
  {
    q: 'Which platforms are supported?',
    a: 'macOS 14+ uses Metal. Windows 10 1809+ uses Direct3D 11 and ConPTY. Linux is officially supported on Ubuntu 22.04+ with Vulkan on Wayland or X11; x86_64 and ARM64 .deb packages and portable tarballs are available. Other glibc 2.35+ desktop distributions are best effort. In-place updates stay macOS-only.',
  },
  {
    q: 'How is this different from Terminal.app / iTerm / Warp?',
    a: 'GPU-first rendering via GPUI, a top tab strip grouped by git workspace, an optional Run Ledger plugin for persistent history, Zed-shaped settings, file-manager paste, and light shell integration. No account, no cloud, no built-in AI.',
  },
  {
    q: 'Is there AI built in?',
    a: 'No. Sleipnir stays a clean native terminal: no assistant, no auto-installed shell plugins, no graphics protocol platform.',
  },
]

function Chip({ children }: { children: string }) {
  return (
    <code className="rounded-[2px] border border-border bg-muted px-1 py-0.5 font-mono text-[11.5px] text-ink-soft">
      {children}
    </code>
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
  const path = usePath()

  useEffect(() => {
    void fetchLatestRelease().then(setRelease)
  }, [])

  // Compat: the changelog used to live at the #/changelog hash route.
  useEffect(() => {
    if (window.location.hash === '#/changelog') {
      window.history.replaceState(null, '', '/changelog')
      window.dispatchEvent(new PopStateEvent('popstate'))
    }
  }, [])

  useEffect(() => {
    document.title =
      path === '/changelog' ? 'changelog · sleipnir' : 'sleipnir — gpu-native terminal'
  }, [path])

  if (path === '/changelog') {
    return <ChangelogPage version={release?.version ?? null} />
  }

  return (
    <div id="top" className="min-h-dvh pb-9">
      {/* Title bar: the page itself is a Sleipnir window */}
      <header className="sticky top-0 z-40 flex h-11 items-center gap-3 border-b border-border bg-background/95 px-4 md:px-6">
        <div className="flex items-center gap-1.5" aria-hidden>
          <span className="size-2.5 rounded-full bg-ink-faint/80" />
          <span className="size-2.5 rounded-full bg-ink-mid/80" />
          <span className="size-2.5 rounded-full bg-ink/80" />
        </div>
        <a
          href="#top"
          className="flex items-center gap-2 rounded-[2px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <img
            src="/app-icon.png"
            alt=""
            className="size-5 rounded-[4px]"
            width={20}
            height={20}
          />
          <span className="font-mono text-[12px] text-muted-foreground">
            <span className="text-foreground">sleipnir</span>
            <span className="hidden sm:inline"> — website — zsh</span>
          </span>
        </a>
        <div className="ml-auto flex items-center gap-2">
          <nav className="mr-1 hidden items-center gap-4 font-mono text-[12px] text-muted-foreground md:flex">
            <a href="#features" className="outline-none transition-colors hover:text-foreground focus-visible:text-foreground">
              features
            </a>
            <a href="#download" className="outline-none transition-colors hover:text-foreground focus-visible:text-foreground">
              download
            </a>
            <Link
              to="/changelog"
              className="outline-none transition-colors hover:text-foreground focus-visible:text-foreground"
            >
              changelog
            </Link>
            <a href="#faq" className="outline-none transition-colors hover:text-foreground focus-visible:text-foreground">
              faq
            </a>
          </nav>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            aria-label="GitHub"
            className="flex size-8 items-center justify-center rounded-[2px] text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          >
            <GitHubIcon className="size-4.5" />
          </a>
          <DownloadMenu release={release} size="sm" align="end" />
        </div>
      </header>

      <main className="mx-auto w-full max-w-[1200px]">
        {/* Hero: pitch on the left, a live session on the right */}
        <section className="grid grid-cols-1 items-center gap-12 px-5 pt-14 pb-16 md:px-8 md:pt-20 lg:grid-cols-[1.02fr_1fr]">
          <div className="min-w-0">
            <p className="font-mono text-[12px] tracking-[0.08em] text-ink-dim">
              <span className="text-muted-foreground">&gt;</span> gpu-native terminal
              emulator
            </p>
            <h1 className="mt-5 font-mono text-[1.3rem] leading-[1.16] font-semibold tracking-[-0.022em] text-balance sm:text-[1.9rem] lg:text-[2.2rem]">
              A fast, native terminal{' '}
              <span className="text-ink text-glow mt-1 block">
                for mac, windows &amp; linux.
                <span className="block-cursor" aria-hidden />
              </span>
            </h1>
            <p className="mt-6 max-w-[34rem] font-mono text-[13.5px] leading-relaxed text-pretty text-muted-foreground">
              Rust + GPUI, the stack behind Zed. Tabs, splits, multi-window,
              adaptive themes, and optional plugins. No account, no
              cloud, no Electron.
            </p>
            <div className="mt-8 flex flex-wrap items-center gap-3">
              <DownloadMenu
                release={release}
                size="lg"
                align="start"
                showIcon
                className="h-10 px-4"
              />
              <a
                href={GITHUB_URL}
                target="_blank"
                rel="noreferrer"
                className="inline-flex h-10 items-center gap-1.5 rounded-[2px] border border-input px-4 font-mono text-sm text-muted-foreground outline-none transition-colors hover:border-ink/50 hover:bg-accent hover:text-ink focus-visible:ring-2 focus-visible:ring-ring"
              >
                <GitHubIcon className="size-4" />
                source
              </a>
            </div>
            <InstallCommand className="mt-6" />
          </div>
          <TerminalWindow title="sleipnir — zsh" crt className="min-w-0">
            <BootTerminal />
          </TerminalWindow>
        </section>

        {/* Capability strip, laid out like panes */}
        <section className="px-5 md:px-8">
          <div className="grid grid-cols-2 gap-px overflow-hidden rounded-md border border-border bg-border sm:grid-cols-4">
            {HIGHLIGHTS.map((h) => (
              <div
                key={h.label}
                className="group flex items-center gap-2.5 bg-background px-4 py-3.5 transition-colors hover:bg-card"
              >
                <h.icon
                  className="size-4 shrink-0 text-ink-dim transition-colors group-hover:text-ink"
                  strokeWidth={1.75}
                  aria-hidden
                />
                <span className="truncate font-mono text-[12px] text-muted-foreground transition-colors group-hover:text-foreground">
                  {h.label}
                </span>
              </div>
            ))}
          </div>
        </section>

        {/* The product, framed as the product */}
        <section className="px-5 pt-20 md:px-8">
          <SectionHeading index="00" name="session" meta="claude · codex · kimi · grok" />
          <TerminalWindow
            title="agents — 4 panes"
            className="mt-6"
            bodyClassName="p-0 md:p-0"
          >
            <img
              src="/app-screenshot.jpg"
              alt="Sleipnir with four split panes running Claude Code, OpenAI Codex, Kimi Code, and Grok Build"
              width={3456}
              height={1980}
              loading="lazy"
              className="block h-auto w-full"
            />
          </TerminalWindow>
        </section>

        {/* Features as a split-pane grid */}
        <section id="features" className="scroll-mt-16 px-5 pt-20 md:px-8">
          <SectionHeading index="01" name="features" meta="9 entries" />
          <div className="mt-6 grid grid-cols-1 gap-px overflow-hidden rounded-md border border-border bg-border sm:grid-cols-2 lg:grid-cols-3">
            {FEATURES.map((f, i) => (
              <article
                key={f.title}
                className="group bg-background p-6 transition-colors hover:bg-card"
              >
                <div className="flex items-baseline gap-2.5">
                  <span className="font-mono text-[11px] text-ink-mid tabular-nums">
                    [{String(i + 1).padStart(2, '0')}]
                  </span>
                  <h3 className="font-mono text-[13.5px] font-semibold tracking-tight text-foreground transition-colors group-hover:text-ink">
                    {f.title}
                  </h3>
                </div>
                <p className="mt-3 font-mono text-[12px] leading-[1.75] text-muted-foreground">
                  {f.body}
                </p>
              </article>
            ))}
          </div>
        </section>

        {/* Download: one-liner plus the target matrix */}
        <section id="download" className="scroll-mt-16 px-5 pt-20 md:px-8">
          <SectionHeading
            index="02"
            name="download"
            meta={release ? `latest: v${release.version}` : 'latest: …'}
          />
          <div className="mt-6 grid grid-cols-1 gap-8 lg:grid-cols-[1fr_1.1fr]">
            <div>
              <p className="max-w-md font-mono text-[12.5px] leading-relaxed text-muted-foreground">
                Free and open source. The installer detects macOS or Linux,
                verifies SHA-256, and drops the binary in place. Windows builds
                ship from GitHub Releases.
              </p>
              <InstallCommand className="mt-6" />
            </div>
            <DownloadTargets release={release} />
          </div>
        </section>

        {/* Changelog, straight from the repo */}
        <section id="changelog" className="scroll-mt-16 px-5 pt-20 md:px-8">
          <SectionHeading
            index="03"
            name="changelog"
            meta={release ? `latest: v${release.version}` : 'from CHANGELOG.md'}
          />
          <LatestChangelog />
        </section>

        {/* FAQ, man-page style */}
        <section id="faq" className="scroll-mt-16 px-5 pt-20 pb-24 md:px-8">
          <SectionHeading index="04" name="faq" meta="man sleipnir" />
          <div className="mt-6 max-w-3xl border-t border-border">
            <Accordion items={FAQ} />
          </div>
        </section>
      </main>

      <footer className="mx-auto flex w-full max-w-[1200px] items-center gap-2 border-t border-border px-5 py-6 font-mono text-[11px] text-muted-foreground md:px-8">
        <img
          src="/app-icon.png"
          alt=""
          className="size-4 rounded-[3px] opacity-80"
          width={16}
          height={16}
        />
        <span>© {new Date().getFullYear()} sleipnir</span>
        <span className="text-border">·</span>
        <span>GPL-3.0-or-later</span>
        <span className="text-border">·</span>
        <a
          href={GITHUB_URL}
          target="_blank"
          rel="noreferrer"
          className="outline-none transition-colors hover:text-ink focus-visible:text-ink"
        >
          github
        </a>
        <span className="text-border">·</span>
        <Link
          to="/changelog"
          className="outline-none transition-colors hover:text-ink focus-visible:text-ink"
        >
          changelog
        </Link>
        <span className="ml-auto hidden sm:inline">exit 0</span>
      </footer>

      <StatusBar version={release?.version ?? null} />
    </div>
  )
}
