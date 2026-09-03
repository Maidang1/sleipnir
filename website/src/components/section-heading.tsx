import type { ReactNode } from 'react'

/**
 * Section heading styled as a shell comment with an index and a trailing rule.
 * `[01] # features ──────────────── 9 entries`
 */
export function SectionHeading({
  index,
  name,
  meta,
}: {
  index: string
  name: string
  meta?: ReactNode
}) {
  return (
    <div className="flex items-baseline gap-3 font-mono">
      <span className="text-[12px] text-ansi-amber">[{index}]</span>
      <h2 className="text-[15px] font-semibold tracking-tight text-foreground">
        <span className="mr-1.5 text-ansi-dimgreen">#</span>
        {name}
      </h2>
      <div className="h-px flex-1 self-center bg-border" aria-hidden />
      {meta && <span className="text-[11px] text-muted-foreground/70">{meta}</span>}
    </div>
  )
}
