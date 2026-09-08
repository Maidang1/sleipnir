export interface ChangelogSection {
  /** lowercased section name, e.g. "features", "fixes" */
  name: string
  items: string[]
}

export interface ChangelogRelease {
  /** "0.6.2", or "unreleased" for the pending section */
  version: string
  sections: ChangelogSection[]
}

/**
 * Parse the repo CHANGELOG.md into structured releases.
 *
 * Expected shape: `## <version>` starts a release (`## Unreleased` maps to
 * "unreleased"), `### <Section>` groups entries, `- ` lines are items.
 * Anything outside that shape (the `# Changelog` title, blank lines, prose)
 * is skipped. Releases without any section are dropped.
 */
export function parseChangelog(markdown: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = []
  let release: ChangelogRelease | null = null
  let section: ChangelogSection | null = null

  for (const line of markdown.split('\n')) {
    const h2 = /^##\s+(.+?)\s*$/.exec(line)
    if (h2) {
      const title = h2[1]
      release = {
        version: title.toLowerCase() === 'unreleased' ? 'unreleased' : title.replace(/^v/, ''),
        sections: [],
      }
      releases.push(release)
      section = null
      continue
    }
    if (!release) continue

    const h3 = /^###\s+(.+?)\s*$/.exec(line)
    if (h3) {
      section = { name: h3[1].toLowerCase(), items: [] }
      release.sections.push(section)
      continue
    }

    const item = /^-\s+(.+?)\s*$/.exec(line)
    if (item && section) {
      section.items.push(item[1])
    }
  }

  return releases.filter((r) => r.sections.length > 0)
}

export function countEntries(release: ChangelogRelease): number {
  return release.sections.reduce((n, s) => n + s.items.length, 0)
}
