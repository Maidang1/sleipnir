import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { INSTALL_COMMAND, INSTALL_HINT } from '@/lib/release'
import { cn } from '@/lib/utils'

export function InstallCommand({ className }: { className?: string }) {
  const [copied, setCopied] = useState(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(INSTALL_COMMAND)
    } catch {
      const field = document.createElement('textarea')
      field.value = INSTALL_COMMAND
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
      <div className="flex items-center gap-2 rounded-lg border border-border bg-muted/70 px-3 py-2">
        <pre
          tabIndex={0}
          className="m-0 min-w-0 flex-1 overflow-x-auto whitespace-nowrap font-mono text-[12.5px] leading-6 text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          <code>{INSTALL_COMMAND}</code>
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
      <p className="max-w-xl text-xs leading-relaxed text-muted-foreground">{INSTALL_HINT}</p>
    </div>
  )
}
