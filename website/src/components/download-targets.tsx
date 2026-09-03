import { ArrowDownToLine } from 'lucide-react'
import type { LatestRelease } from '@/lib/release'
import { FALLBACK_RELEASES_URL } from '@/lib/release'
import { cn } from '@/lib/utils'

type Target = {
  triple: string
  pkg: string
  note: string
  url: string | null
}

function targets(release: LatestRelease | null): Target[] {
  return [
    {
      triple: 'aarch64-apple-darwin',
      pkg: '.dmg',
      note: 'macOS 14+ · metal',
      url: release?.dmgUrl ?? null,
    },
    {
      triple: 'x86_64-pc-windows-msvc',
      pkg: '.exe',
      note: 'windows 10 1809+ · d3d11',
      url: release?.windowsExeUrl ?? null,
    },
    {
      triple: 'x86_64-unknown-linux-gnu',
      pkg: '.deb',
      note: 'ubuntu 22.04+ · vulkan',
      url: release?.linuxX64DebUrl ?? null,
    },
    {
      triple: 'x86_64-unknown-linux-gnu',
      pkg: '.tar.gz',
      note: 'portable',
      url: release?.linuxX64TarUrl ?? null,
    },
    {
      triple: 'aarch64-unknown-linux-gnu',
      pkg: '.deb',
      note: 'ubuntu 22.04+ · vulkan',
      url: release?.linuxArm64DebUrl ?? null,
    },
    {
      triple: 'aarch64-unknown-linux-gnu',
      pkg: '.tar.gz',
      note: 'portable',
      url: release?.linuxArm64TarUrl ?? null,
    },
  ]
}

/**
 * rustup-style target matrix. Each row is a build target with its package
 * format and a direct download link; falls back to the releases page.
 */
export function DownloadTargets({ release }: { release: LatestRelease | null }) {
  return (
    <div className="overflow-hidden rounded-md border border-border bg-card">
      <div className="flex items-center justify-between border-b border-border bg-muted/60 px-4 py-2 text-[11px] text-muted-foreground">
        <span>$ sleipnir target --list-all</span>
        <span className="tabular-nums">{release ? `v${release.version}` : '…'}</span>
      </div>
      <ul>
        {targets(release).map((t) => {
          const href = t.url ?? FALLBACK_RELEASES_URL
          return (
            <li key={`${t.triple}-${t.pkg}`}>
              <a
                href={href}
                target={t.url ? undefined : '_blank'}
                rel={t.url ? undefined : 'noreferrer'}
                className={cn(
                  'group flex items-center gap-3 border-b border-border px-4 py-2.5 text-[12.5px] outline-none transition-colors last:border-b-0',
                  'hover:bg-accent focus-visible:bg-accent',
                )}
              >
                <ArrowDownToLine
                  className="size-3.5 shrink-0 text-muted-foreground transition-colors group-hover:text-ansi-green"
                  aria-hidden
                />
                <span className="min-w-0 flex-1 truncate text-ansi-cyan xl:w-56 xl:flex-none xl:whitespace-nowrap">
                  {t.triple}
                </span>
                <span className="hidden shrink-0 text-foreground/80 sm:inline">{t.pkg}</span>
                <span className="hidden w-40 shrink-0 text-right text-[11px] text-muted-foreground/70 xl:block">
                  {t.note}
                </span>
                <span
                  className={cn(
                    'shrink-0 text-[11px] text-muted-foreground transition-colors group-hover:text-ansi-green',
                  )}
                >
                  [fetch]
                </span>
              </a>
            </li>
          )
        })}
      </ul>
      <div className="border-t border-border bg-muted/40 px-4 py-2 text-[11px] text-muted-foreground">
        <span className="text-ansi-dimgreen">note</span> · every package ships a .sha256
        sidecar ·{' '}
        <a
          href={FALLBACK_RELEASES_URL}
          target="_blank"
          rel="noreferrer"
          className="text-foreground/80 underline decoration-border underline-offset-4 outline-none transition-colors hover:text-ansi-green"
        >
          all releases
        </a>
      </div>
    </div>
  )
}
