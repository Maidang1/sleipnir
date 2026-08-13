import { useEffect, useRef, useState } from 'react'
import { Download } from 'lucide-react'
import { Button } from '@/components/button'
import type { LatestRelease } from '@/lib/release'
import { FALLBACK_RELEASES_URL, GITHUB_URL } from '@/lib/release'
import { cn } from '@/lib/utils'

export function DownloadMenu({
  release,
  size = 'sm',
  align = 'end',
  showIcon = false,
  className,
}: {
  release: LatestRelease | null
  size?: 'sm' | 'lg'
  align?: 'start' | 'end'
  showIcon?: boolean
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointer = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onPointer)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onPointer)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const dmg = release?.dmgUrl
  const zip = release?.zipUrl
  const windowsZip = release?.windowsZipUrl
  const page = release?.htmlUrl ?? FALLBACK_RELEASES_URL

  return (
    <div ref={rootRef} className="relative inline-flex">
      <Button
        type="button"
        size={size}
        className={className}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {showIcon && <Download />}
        Download
      </Button>
      {open && (
        <div
          role="menu"
          className={cn(
            'absolute top-full z-50 mt-1.5 min-w-56 origin-top rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md',
            align === 'end' ? 'right-0' : 'left-0',
          )}
        >
          {dmg ? (
            <a
              role="menuitem"
              href={dmg}
              className="flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              macOS (.dmg)
            </a>
          ) : null}
          {zip ? (
            <a
              role="menuitem"
              href={zip}
              className="flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              macOS (.zip)
            </a>
          ) : null}
          {windowsZip ? (
            <a
              role="menuitem"
              href={windowsZip}
              className="flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              Windows (.zip)
            </a>
          ) : (
            <a
              role="menuitem"
              href={`${GITHUB_URL}#windows`}
              target="_blank"
              rel="noreferrer"
              className="flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              Windows (build from source)
            </a>
          )}
          {!dmg && !zip && !windowsZip ? (
            <a
              role="menuitem"
              href={page}
              target="_blank"
              rel="noreferrer"
              className="flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              GitHub Releases
            </a>
          ) : (
            <a
              role="menuitem"
              href={page}
              target="_blank"
              rel="noreferrer"
              className="flex h-8 items-center rounded-md px-2.5 text-sm text-muted-foreground outline-none hover:bg-accent hover:text-accent-foreground"
              onClick={() => setOpen(false)}
            >
              All releases
            </a>
          )}
          <div className="my-1 h-px bg-border" />
          <div className="flex h-8 cursor-default items-center rounded-md px-2.5 text-sm text-muted-foreground opacity-45">
            Linux (soon)
          </div>
        </div>
      )}
    </div>
  )
}
