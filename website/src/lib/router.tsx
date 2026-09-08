import {
  useEffect,
  useState,
  type AnchorHTMLAttributes,
  type MouseEvent,
  type ReactNode,
} from 'react'

/**
 * Minimal History-API router. Routes are real paths (`/`, `/changelog`);
 * in-page anchors (`#features`) are untouched. Cloudflare Pages serves
 * index.html for unknown paths via public/_redirects.
 */
export function usePath(): string {
  const [path, setPath] = useState(() => window.location.pathname)
  useEffect(() => {
    const onPop = () => setPath(window.location.pathname)
    window.addEventListener('popstate', onPop)
    return () => window.removeEventListener('popstate', onPop)
  }, [])
  return path
}

export function navigate(to: string) {
  if (window.location.pathname === to) return
  window.history.pushState(null, '', to)
  window.dispatchEvent(new PopStateEvent('popstate'))
  window.scrollTo(0, 0)
}

interface LinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  to: string
  children: ReactNode
}

/** Client-side navigation link; falls back to a plain anchor for new-tab clicks. */
export function Link({ to, children, onClick, ...rest }: LinkProps) {
  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event)
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return
    }
    event.preventDefault()
    navigate(to)
  }
  return (
    <a href={to} onClick={handleClick} {...rest}>
      {children}
    </a>
  )
}
