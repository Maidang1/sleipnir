import type { ReactNode } from 'react'
import changelogRaw from '../../../CHANGELOG.md?raw'
import { countEntries, parseChangelog, type ChangelogRelease } from '@/lib/changelog'
import { GITHUB_URL } from '@/lib/release'

const RELEASES = parseChangelog(changelogRaw)

/** ANSI accent per changelog section, matching the site's palette. */
const SECTION_TONE: Record<string, string> = {
  features: 'text-ansi-green border-ansi-green/30',
  changes: 'text-ansi-cyan border-ansi-cyan/30',
  fixes: 'text-ansi-amber border-ansi-amber/30',
  documentation: 'text-ansi-purple border-ansi-purple/30',
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
          className="rounded-[2px] border border-border bg-muted px-1 py-0.5 text-[11px] text-ansi-cyan"
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
          className="text-ansi-cyan underline decoration-ansi-cyan/40 underline-offset-2 outline-none transition-colors hover:text-ansi-green focus-visible:text-ansi-green"
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
                <span className="shrink-0 text-ansi-dimgreen" aria-hidden>
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
      <span className="font-mono text-[13px] font-semibold text-ansi-green">
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
        className="font-mono text-[11px] text-muted-foreground/70 outline-none transition-colors hover:text-ansi-green focus-visible:text-ansi-green"
      >
        github ↗
      </a>
    </>
  )
}

/**
 * Repo CHANGELOG.md, rendered as a release log. The latest release is always
 * expanded; older ones collapse into native <details> rows.
 */
export function Changelog() {
  if (RELEASES.length === 0) return null
  const [latest, ...older] = RELEASES

  return (
    <div className="mt-6 max-w-3xl space-y-3">
      <div className="overflow-hidden rounded-md border border-border bg-card/40">
        <div className="flex items-baseline gap-3 border-b border-border px-4 py-3">
          <ReleaseHeader release={latest} />
        </div>
        <ReleaseBody release={latest} />
      </div>

      {older.map((release) => (
        <details
          key={release.version}
          className="group overflow-hidden rounded-md border border-border bg-background open:bg-card/40"
        >
          <summary className="flex cursor-pointer list-none items-baseline gap-3 px-4 py-3 outline-none transition-colors hover:bg-card focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
            <span
              className="font-mono text-[11px] text-ansi-dimgreen transition-transform group-open:rotate-90"
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
  )
}
