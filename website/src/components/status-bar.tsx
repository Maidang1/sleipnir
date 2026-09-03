import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'

const WINDOWS = [
  { id: 'top', label: 'top' },
  { id: 'features', label: 'features' },
  { id: 'download', label: 'download' },
  { id: 'faq', label: 'faq' },
]

/**
 * tmux-style status bar pinned to the bottom of the viewport.
 * Windows highlight via scroll-spy; clicking jumps to the section.
 */
export function StatusBar({ version }: { version: string | null }) {
  const [active, setActive] = useState('top')

  useEffect(() => {
    const sections = WINDOWS.map((w) => document.getElementById(w.id)).filter(
      (el): el is HTMLElement => el !== null,
    )
    if (sections.length === 0) return
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) setActive(entry.target.id)
        }
      },
      { rootMargin: '-30% 0px -55% 0px' },
    )
    sections.forEach((s) => observer.observe(s))
    return () => observer.disconnect()
  }, [])

  return (
    <nav
      aria-label="Section navigation"
      className="fixed inset-x-0 bottom-0 z-40 flex h-9 items-stretch border-t border-border bg-card/95 text-[11px] text-muted-foreground backdrop-blur-none"
    >
      <div className="flex items-center gap-1.5 bg-ansi-green px-3 font-medium text-primary-foreground">
        <span aria-hidden>[</span>sleipnir<span aria-hidden>]</span>
      </div>
      <div className="flex items-stretch overflow-x-auto">
        {WINDOWS.map((w, i) => {
          const isActive = active === w.id
          return (
            <a
              key={w.id}
              href={`#${w.id}`}
              aria-current={isActive ? 'true' : undefined}
              className={cn(
                'flex items-center gap-1 border-r border-border px-3 outline-none transition-colors focus-visible:bg-accent',
                isActive
                  ? 'bg-accent font-medium text-ansi-green'
                  : 'hover:bg-muted hover:text-foreground',
              )}
            >
              <span className={isActive ? 'text-ansi-amber' : 'text-muted-foreground/60'}>
                {i}:
              </span>
              {w.label}
              {isActive && <span aria-hidden>*</span>}
            </a>
          )
        })}
      </div>
      <div className="ml-auto hidden items-center gap-3 px-3 sm:flex">
        <a
          href="https://github.com/Maidang1/sleipnir"
          target="_blank"
          rel="noreferrer"
          className="outline-none transition-colors hover:text-foreground focus-visible:text-foreground"
        >
          github
        </a>
        <span className="text-border" aria-hidden>
          │
        </span>
        <span className="tabular-nums">{version ? `v${version}` : 'main'}</span>
        <span className="text-border" aria-hidden>
          │
        </span>
        <span>utf-8</span>
      </div>
    </nav>
  )
}
