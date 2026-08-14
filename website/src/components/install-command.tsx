import { useId, useState } from 'react'
import { Check, Copy } from 'lucide-react'
import {
  INSTALL_COMMANDS,
  INSTALL_HINTS,
  type InstallPlatform,
} from '@/lib/release'
import { cn } from '@/lib/utils'

const PLATFORMS: { id: InstallPlatform; label: string }[] = [
  { id: 'macos', label: 'macOS' },
  { id: 'windows', label: 'Windows' },
  { id: 'linux', label: 'Linux' },
]

export function InstallCommand({
  className,
  platform,
  onPlatformChange,
}: {
  className?: string
  platform: InstallPlatform
  onPlatformChange: (next: InstallPlatform) => void
}) {
  const [copied, setCopied] = useState(false)
  const tablistId = useId()
  const command = INSTALL_COMMANDS[platform]
  const hint = INSTALL_HINTS[platform]

  function select(next: InstallPlatform) {
    onPlatformChange(next)
    setCopied(false)
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(command)
    } catch {
      const field = document.createElement('textarea')
      field.value = command
      field.setAttribute('readonly', '')
      field.style.position = 'fixed'
      field.style.left = '-9999px'
      document.body.appendChild(field)
      field.select()
      document.execCommand('copy')
      document.body.removeChild(field)
    }
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }

  return (
    <div className={cn('flex max-w-2xl flex-col gap-2', className)}>
      <div
        role="tablist"
        aria-label="Install platform"
        className="inline-flex w-fit rounded-lg border border-border bg-muted/70 p-0.5"
        onKeyDown={(e) => {
          const i = PLATFORMS.findIndex((p) => p.id === platform)
          if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
            e.preventDefault()
            const dir = e.key === 'ArrowRight' ? 1 : -1
            const next = PLATFORMS[(i + dir + PLATFORMS.length) % PLATFORMS.length]
            select(next.id)
            document.getElementById(`${tablistId}-${next.id}`)?.focus()
          }
        }}
      >
        {PLATFORMS.map((p) => {
          const selected = platform === p.id
          return (
            <button
              key={p.id}
              id={`${tablistId}-${p.id}`}
              type="button"
              role="tab"
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              onClick={() => select(p.id)}
              className={cn(
                'h-8 min-w-16 rounded-md px-3 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/60',
                selected
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              {p.label}
            </button>
          )
        })}
      </div>
      <div
        role="tabpanel"
        id={`${tablistId}-panel`}
        aria-labelledby={`${tablistId}-${platform}`}
        className="flex items-center gap-2 rounded-lg border border-border bg-muted/70 px-3 py-2"
      >
        <pre
          tabIndex={0}
          className="m-0 min-w-0 flex-1 overflow-x-auto whitespace-nowrap font-mono text-[12.5px] leading-6 text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          <code>{command}</code>
        </pre>
        <button
          type="button"
          onClick={() => void copy()}
          aria-label={copied ? 'Copied install command' : 'Copy install command'}
          className="inline-flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-background hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          {copied ? (
            <Check className="size-3.5" strokeWidth={1.75} />
          ) : (
            <Copy className="size-3.5" strokeWidth={1.75} />
          )}
        </button>
        <span className="sr-only" aria-live="polite">
          {copied ? 'Copied to clipboard' : ''}
        </span>
      </div>
      <p className="max-w-xl text-xs leading-relaxed text-muted-foreground">{hint}</p>
    </div>
  )
}
