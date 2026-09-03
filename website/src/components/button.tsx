import type { ButtonHTMLAttributes, AnchorHTMLAttributes, ReactNode } from 'react'
import { cn } from '@/lib/utils'

const base =
  'inline-flex shrink-0 items-center justify-center gap-1.5 rounded-[2px] border border-transparent font-mono text-sm font-medium whitespace-nowrap transition-[color,background-color,border-color,transform,box-shadow] outline-none select-none focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.96] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4'

const variants = {
  default:
    'bg-ansi-green text-primary-foreground hover:bg-ansi-green/90 hover:shadow-[0_0_24px_oklch(0.87_0.2_150/35%)]',
  outline:
    'border-input bg-transparent text-foreground hover:border-ansi-green/50 hover:bg-accent hover:text-ansi-green',
  ghost: 'text-muted-foreground hover:bg-accent hover:text-foreground',
} as const

const sizes = {
  sm: 'h-7 px-2.5 text-[12px]',
  default: 'h-8 px-2.5 text-[12.5px]',
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
