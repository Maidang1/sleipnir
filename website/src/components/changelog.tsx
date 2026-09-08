import type { ReactNode } from 'react'
import changelogRaw from '../../../CHANGELOG.md?raw'
import { countEntries, parseChangelog, type ChangelogRelease } from '@/lib/changelog'
import { GITHUB_URL } from '@/lib/release'
import { Link } from '@/lib/router'
import { StatusBar } from '@/components/status-bar'

const RELEASES = parseChangelog(changelogRaw)

/** ANSI accent per changelog section, matching the site's palette. */
const SECTION_TONE: Record<string, string> = {
  features: 'text-ink border-ink/30',
  changes: 'text-ink-soft border-ink-soft/30',
  fixes: 'text-ink-mid border-ink-mid/30',
  documentation: 'text-ink-mid border-ink-mid/30',
}

function sectionTone(name: string): string {
  return SECTION_TONE[name] ?? 'text-muted-foreground border-border'
}

/** Render the changelog's tiny inline-markdown subset: `code` and [text](url). */
function renderInline(text: string): ReactNode[] {
  const re = /(`[^`]+`)|(\[[^\]]+\]\([^)]+\))/g
  const out: ReactNode[] = []
  let last = 0
  let key = 0
  for (const match of text.matchAll(re)) {
    const index = match.index ?? 0
    if (index > last) out.push(text.slice(last, index))
    const token = match[0]
    if (token.startsWith('`')) {
      out.push(
        <code
          key={key++}
          className="rounded-[2px] border border-border bg-muted px-1 py-0.5 text-[11px] text-ink-soft"
        >
          {token.slice(1, -1)}
        </code>,
      )
    } else {
      const link = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token)
      out.push(
        <a
          key={key++}
          href={link?.[2]}
          target="_blank"
          rel="noreferrer"
          className="text-ink-soft underline decoration-ink-soft/40 underline-offset-2 outline-none transition-colors hover:text-ink focus-visible:text-ink"
        >
          {link?.[1] ?? token}
        </a>,
      )
    }
    last = index + token.length
  }
  if (last < text.length) out.push(text.slice(last))
  return out
}

function ReleaseBody({ release }: { release: ChangelogRelease }) {
  return (
    <div className="space-y-4 px-4 py-4">
      {release.sections.map((section) => (
        <div key={section.name}>
          <span
            className={`inline-block rounded-[2px] border px-1.5 py-0.5 font-mono text-[10.5px] tracking-[0.08em] ${sectionTone(section.name)}`}
          >
            {section.name}
          </span>
          <ul className="mt-2 space-y-1.5">
            {section.items.map((item, i) => (
              <li
                key={i}
                className="flex gap-2 font-mono text-[12px] leading-[1.7] text-muted-foreground"
              >
                <span className="shrink-0 text-ink-dim" aria-hidden>
                  -
                </span>
                <span>{renderInline(item)}</span>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </div>
  )
}

function ReleaseHeader({ release }: { release: ChangelogRelease }) {
  const entries = countEntries(release)
  const href =
    release.version === 'unreleased'
      ? `${GITHUB_URL}/blob/main/CHANGELOG.md`
      : `${GITHUB_URL}/releases/tag/v${release.version}`
  return (
    <>
      <span className="font-mono text-[13px] font-semibold text-ink">
        {release.version === 'unreleased' ? 'unreleased' : `v${release.version}`}
      </span>
      <span className="h-px flex-1 self-center bg-border" aria-hidden />
      <span className="font-mono text-[11px] text-muted-foreground/70">
        {entries} {entries === 1 ? 'entry' : 'entries'}
      </span>
      <a
        href={href}
        target="_blank"
        rel="noreferrer"
        className="font-mono text-[11px] text-muted-foreground/70 outline-none transition-colors hover:text-ink focus-visible:text-ink"
      >
        github ↗
      </a>
    </>
  )
}

function ReleaseBlock({ release }: { release: ChangelogRelease }) {
  return (
    <div className="overflow-hidden rounded-md border border-border bg-card/40">
      <div className="flex items-baseline gap-3 border-b border-border px-4 py-3">
        <ReleaseHeader release={release} />
      </div>
      <ReleaseBody release={release} />
    </div>
  )
}

/**
 * Home-page teaser: only the latest release, expanded, plus a link to the
 * full changelog page.
 */
export function LatestChangelog() {
  const latest = RELEASES[0]
  if (!latest) return null
  const rest = RELEASES.length - 1
  return (
    <div className="mt-6 max-w-3xl">
      <ReleaseBlock release={latest} />
      {rest > 0 && (
        <Link
          to="/changelog"
          className="mt-3 inline-flex items-center gap-1.5 rounded-[2px] font-mono text-[12px] text-muted-foreground outline-none transition-colors hover:text-ink focus-visible:text-ink"
        >
          <span className="text-ink-dim" aria-hidden>
            &gt;
          </span>
          all releases ({rest} older) →
        </Link>
      )}
    </div>
  )
}

/**
 * Full changelog at /changelog: latest release expanded, older releases in
 * collapsible <details> rows.
 */
export function ChangelogPage({ version }: { version: string | null }) {
  const [latest, ...older] = RELEASES
  return (
    <div className="min-h-dvh pb-9">
      <header className="sticky top-0 z-40 flex h-11 items-center gap-3 border-b border-border bg-background/95 px-4 md:px-6">
        <div className="flex items-center gap-1.5" aria-hidden>
          <span className="size-2.5 rounded-full bg-ink-faint/80" />
          <span className="size-2.5 rounded-full bg-ink-mid/80" />
          <span className="size-2.5 rounded-full bg-ink/80" />
        </div>
        <span className="font-mono text-[12px] text-muted-foreground">
          <span className="text-foreground">sleipnir</span>
          <span className="hidden sm:inline"> — changelog — less</span>
        </span>
        <Link
          to="/"
          className="ml-auto inline-flex h-8 items-center gap-1.5 rounded-[2px] px-2 font-mono text-[12px] text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-ink focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="text-ink-dim" aria-hidden>
            &gt;
          </span>
          cd ..
        </Link>
      </header>

      <main className="mx-auto w-full max-w-[1200px] px-5 pt-14 md:px-8">
        <p className="font-mono text-[12px] tracking-[0.08em] text-ink-dim">
          <span className="text-muted-foreground">&gt;</span> cat CHANGELOG.md
        </p>
        <h1 className="mt-4 font-mono text-[1.3rem] font-semibold tracking-[-0.022em] text-foreground sm:text-[1.6rem]">
          changelog
        </h1>
        <p className="mt-3 max-w-[34rem] font-mono text-[12.5px] leading-relaxed text-muted-foreground">
          Every release, straight from the repo's CHANGELOG.md.{' '}
          {version ? `Latest: v${version}.` : ''}
        </p>

        {RELEASES.length === 0 ? (
          <p className="mt-8 font-mono text-[12px] text-muted-foreground">
            no releases parsed.
          </p>
        ) : (
          <div className="mt-8 max-w-3xl space-y-3 pb-24">
            <ReleaseBlock release={latest} />
            {older.map((release) => (
              <details
                key={release.version}
                className="group overflow-hidden rounded-md border border-border bg-background open:bg-card/40"
              >
                <summary className="flex cursor-pointer list-none items-baseline gap-3 px-4 py-3 outline-none transition-colors hover:bg-card focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
                  <span
                    className="font-mono text-[11px] text-ink-dim transition-transform group-open:rotate-90"
                    aria-hidden
                  >
                    ▸
                  </span>
                  <ReleaseHeader release={release} />
                </summary>
                <div className="border-t border-border">
                  <ReleaseBody release={release} />
                </div>
              </details>
            ))}
          </div>
        )}
      </main>

      <StatusBar version={version} />
    </div>
  )
}
