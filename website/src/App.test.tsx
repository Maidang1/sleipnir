import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from './App'
import metadata from '../index.html?raw'

vi.mock('@/lib/router', () => ({
  usePath: () => '/',
  Link: ({ to, children }: { to: string; children: ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}))

// Historical changelog entries deliberately describe removed features. Keep
// these assertions on current product copy, not the embedded release history.
vi.mock('@/components/changelog', () => ({
  LatestChangelog: () => null,
  ChangelogPage: () => null,
}))

describe('current product claims', () => {
  beforeEach(() => {
    // Reduced motion renders the entire demo instead of waiting on timers,
    // so stale claims in animated output are checked too.
    vi.stubGlobal('window', { matchMedia: () => ({ matches: true }) })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('does not advertise removed session restore or the side rail', () => {
    const html = renderToStaticMarkup(<App />)
    expect(html).not.toMatch(
      /session restore|session\.json|session layout restores|side (?:tab )?rail|windows · restored/i,
    )
    expect(html).toContain('fresh tab')
  })

  it('identifies the Run Ledger as an optional, separately installed plugin', () => {
    const html = renderToStaticMarkup(<App />)
    expect(html).toContain('optional Run Ledger plugin')
    expect(html).toContain('install and enable separately')
    expect(html).not.toContain('⌘⇧L')
  })

  it('does not advertise session restore in search or social metadata', () => {
    expect(metadata).not.toMatch(/session restore/i)
  })

  it('labels the app with the license declared by its crate', () => {
    expect(renderToStaticMarkup(<App />)).toContain('GPL-3.0-or-later')
  })
})
