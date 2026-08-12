import { useState, type ReactNode } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { cn } from '@/lib/utils'

export type FaqItem = { q: string; a: ReactNode }

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
              className="group flex w-full items-start justify-between gap-4 rounded-lg py-2.5 text-left text-[15px] font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <span>{item.q}</span>
              {isOpen ? (
                <ChevronUp className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              ) : (
                <ChevronDown className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              )}
            </button>
            <div
              className={cn(
                'grid transition-[grid-template-rows] duration-200 ease-out',
                isOpen ? 'grid-rows-[1fr]' : 'grid-rows-[0fr]',
              )}
            >
              <div className="overflow-hidden">
                <div className="max-w-[38rem] pb-2.5 text-sm leading-relaxed text-muted-foreground">
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
