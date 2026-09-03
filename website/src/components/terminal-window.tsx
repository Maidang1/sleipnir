import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/**
 * Window chrome that mimics Sleipnir itself: traffic lights, a tab strip
 * showing title + cwd, and a hairline frame. Content is the terminal body.
 */
export function TerminalWindow({
  title,
  children,
  className,
  bodyClassName,
  crt = false,
}: {
  title: string
  children: ReactNode
  className?: string
  bodyClassName?: string
  crt?: boolean
}) {
  return (
    <div
      className={cn(
        'overflow-hidden rounded-md border border-border bg-card shadow-[0_0_80px_oklch(0.87_0.2_150/6%)]',
        className,
      )}
    >
      <div className="flex h-9 items-center gap-3 border-b border-border bg-muted/60 px-3">
        <div className="flex items-center gap-1.5" aria-hidden>
          <span className="size-2.5 rounded-full bg-ansi-red/80" />
          <span className="size-2.5 rounded-full bg-ansi-amber/80" />
          <span className="size-2.5 rounded-full bg-ansi-green/80" />
        </div>
        <div className="flex h-6 items-center gap-1.5 rounded-sm bg-background/70 px-2 text-[11px] text-muted-foreground">
          <span className="inline-block size-1.5 rounded-[1px] bg-ansi-green" aria-hidden />
          {title}
        </div>
        <span className="ml-auto hidden text-[11px] text-muted-foreground/60 sm:block">
          80×24
        </span>
      </div>
      <div
        className={cn(
          'relative p-4 font-mono text-[12.5px] leading-[1.7] md:p-5 md:text-[13px]',
          crt && 'crt',
          bodyClassName,
        )}
      >
        {children}
      </div>
    </div>
  )
}
