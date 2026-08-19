import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { Download } from 'lucide-react'
import { Button } from '@/components/button'
import type { LatestRelease } from '@/lib/release'
import { FALLBACK_RELEASES_URL } from '@/lib/release'
import { cn } from '@/lib/utils'

type MenuItem = {
  href: string
  label: string
  muted?: boolean
  external?: boolean
}

function downloadItems(release: LatestRelease | null): MenuItem[] {
  const page = release?.htmlUrl ?? FALLBACK_RELEASES_URL
  const items: MenuItem[] = []
  if (release?.dmgUrl) items.push({ href: release.dmgUrl, label: 'macOS (.dmg)' })
  if (release?.zipUrl) items.push({ href: release.zipUrl, label: 'macOS (.zip)' })
  if (release?.windowsZipUrl) {
    items.push({ href: release.windowsZipUrl, label: 'Windows (.zip)' })
  }
  items.push({ href: page, label: 'All releases', muted: true, external: true })
  return items
}

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
  const [active, setActive] = useState(0)
  const rootRef = useRef<HTMLDivElement>(null)
  const menuId = useId()
  const items = downloadItems(release)

  useEffect(() => {
    if (!open) return
    const onPointer = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false)
    }
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false)
        rootRef.current?.querySelector('button')?.focus()
      }
    }
    document.addEventListener('mousedown', onPointer)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onPointer)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    document.getElementById(`${menuId}-${active}`)?.focus()
  }, [open, active, menuId])

  function onMenuKey(e: ReactKeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActive((i) => (i + 1) % items.length)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActive((i) => (i - 1 + items.length) % items.length)
    } else if (e.key === 'Home') {
      e.preventDefault()
      setActive(0)
    } else if (e.key === 'End') {
      e.preventDefault()
      setActive(items.length - 1)
    }
  }

  return (
    <div ref={rootRef} className="relative inline-flex">
      <Button
        type="button"
        size={size}
        className={className}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        onClick={() => {
          setActive(0)
          setOpen((v) => !v)
        }}
      >
        {showIcon && <Download />}
        Download
      </Button>
      {open && (
        <div
          id={menuId}
          role="menu"
          aria-label="Download Sleipnir"
          onKeyDown={onMenuKey}
          className={cn(
            'absolute top-full z-50 mt-1.5 min-w-56 origin-top rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md',
            align === 'end' ? 'right-0' : 'left-0',
          )}
        >
          {items.map((item, i) => (
            <a
              key={item.label}
              id={`${menuId}-${i}`}
              role="menuitem"
              href={item.href}
              tabIndex={i === active ? 0 : -1}
              target={item.external ? '_blank' : undefined}
              rel={item.external ? 'noreferrer' : undefined}
              className={cn(
                'flex h-8 items-center rounded-md px-2.5 text-sm outline-none hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring/60',
                item.muted && 'text-muted-foreground',
              )}
              onClick={() => setOpen(false)}
            >
              {item.label}
            </a>
          ))}
        </div>
      )}
    </div>
  )
}
