import { describe, expect, it } from 'vitest'
import { countEntries, parseChangelog } from './changelog'

const SAMPLE = `# Changelog

## Unreleased

### Features
- Something not shipped yet.

## 0.6.2

### Fixes
- First fix with \`inline code\` and a [link](https://example.com).
- Second fix.

### Changes
- One change.

## 0.6.1

### Documentation
- Docs only release.

## v0.5.0

### Features
- Legacy v-prefix version.
`

describe('parseChangelog', () => {
  const releases = parseChangelog(SAMPLE)

  it('parses every release in order', () => {
    expect(releases.map((r) => r.version)).toEqual([
      'unreleased',
      '0.6.2',
      '0.6.1',
      '0.5.0',
    ])
  })

  it('strips a legacy v prefix from versions', () => {
    expect(releases[3].version).toBe('0.5.0')
  })

  it('groups items under lowercased sections', () => {
    expect(releases[1].sections).toEqual([
      {
        name: 'fixes',
        items: [
          'First fix with `inline code` and a [link](https://example.com).',
          'Second fix.',
        ],
      },
      { name: 'changes', items: ['One change.'] },
    ])
  })

  it('skips the top-level title and ignores prose', () => {
    expect(releases).toHaveLength(4)
  })

  it('drops releases without sections', () => {
    expect(parseChangelog('## 1.0.0\nno sections here\n')).toEqual([])
  })

  it('handles an empty changelog', () => {
    expect(parseChangelog('')).toEqual([])
  })

  it('counts entries across sections', () => {
    expect(countEntries(releases[1])).toBe(3)
  })
})
