import type { ButtonHTMLAttributes, AnchorHTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

const base =
  'inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg border border-transparent text-sm font-medium whitespace-nowrap transition-[color,background-color,transform,box-shadow] outline-none select-none focus-visible:ring-2 focus-visible:ring-ring/60 active:scale-[0.96] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4'

const variants = {
  default: 'bg-primary text-primary-foreground hover:bg-primary/80',
  outline:
    'border-border bg-background hover:bg-muted hover:text-foreground dark:border-input',
  ghost: 'hover:bg-muted hover:text-foreground',
} as const

const sizes = {
  sm: 'h-7 px-2.5 text-[0.8rem]',
  default: 'h-8 px-2.5',
  lg: 'h-10 px-4',
} as const

type Variant = keyof typeof variants
type Size = keyof typeof sizes

type Common = {
  variant?: Variant
  size?: Size
  className?: string
  children?: ReactNode
}

export function Button({
  variant = 'default',
  size = 'default',
  className,
  children,
  ...props
}: Common & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={cn(base, variants[variant], sizes[size], className)}
      {...props}
    >
      {children}
    </button>
  )
}

export function ButtonLink({
  variant = 'default',
  size = 'default',
  className,
  children,
  ...props
}: Common & AnchorHTMLAttributes<HTMLAnchorElement>) {
  return (
    <a className={cn(base, variants[variant], sizes[size], className)} {...props}>
      {children}
    </a>
  )
}
