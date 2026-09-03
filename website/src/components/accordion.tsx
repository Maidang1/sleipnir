import { useState, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

export type FaqItem = { q: string; a: ReactNode }

/**
 * FAQ styled like querying a man page: `? question` rows that expand into
 * indented answers. `+`/`-` markers instead of chevrons.
 */
export function Accordion({ items }: { items: FaqItem[] }) {
  const [open, setOpen] = useState<string | null>(null)

  return (
    <div className="flex w-full flex-col">
      {items.map((item) => {
        const isOpen = open === item.q
        return (
          <div key={item.q} className="not-last:border-b border-border">
            <button
              type="button"
              aria-expanded={isOpen}
              onClick={() => setOpen(isOpen ? null : item.q)}
              className="group flex w-full items-baseline justify-between gap-4 rounded-[2px] px-1 py-3 text-left font-mono text-[13.5px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className="min-w-0">
                <span
                  className={cn(
                    'mr-2 transition-colors',
                    isOpen ? 'text-ansi-amber' : 'text-ansi-dimgreen group-hover:text-ansi-amber',
                  )}
                  aria-hidden
                >
                  ?
                </span>
                <span
                  className={cn(
                    'transition-colors',
                    isOpen ? 'text-foreground' : 'text-foreground/85 group-hover:text-foreground',
                  )}
                >
                  {item.q}
                </span>
              </span>
              <span
                className={cn(
                  'shrink-0 font-mono text-[13px] transition-colors',
                  isOpen ? 'text-ansi-green' : 'text-muted-foreground group-hover:text-ansi-green',
                )}
                aria-hidden
              >
                {isOpen ? '[-]' : '[+]'}
              </span>
            </button>
            <div
              className={cn(
                'grid transition-[grid-template-rows] duration-200 ease-out',
                isOpen ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]',
              )}
            >
              <div className="overflow-hidden">
                <div className="max-w-[42rem] pb-4 pl-6 text-[13px] leading-relaxed text-muted-foreground">
                  {item.a}
                </div>
              </div>
            </div>
          </div>
        )
      })}
    </div>
  )
}
