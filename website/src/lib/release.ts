export interface LatestRelease {
  version: string
  dmgUrl: string | null
  zipUrl: string | null
  htmlUrl: string
}

export const GITHUB_REPO = 'Maidang1/sleipnir'
export const GITHUB_URL = `https://github.com/${GITHUB_REPO}`
export const FALLBACK_RELEASES_URL = `${GITHUB_URL}/releases`
export const INSTALL_SCRIPT_URL = `https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/install.sh`
export const INSTALL_COMMAND = `curl -fsSL ${INSTALL_SCRIPT_URL} | bash`

export async function fetchLatestRelease(): Promise<LatestRelease | null> {
  try {
    const res = await fetch(
      `https://api.github.com/repos/${GITHUB_REPO}/releases/latest`,
      {
        headers: { Accept: 'application/vnd.github+json' },
        signal: AbortSignal.timeout(4000),
      },
    )
    if (!res.ok) return null
    const data = (await res.json()) as {
      tag_name?: string
      html_url?: string
      assets?: Array<{ name: string; browser_download_url: string }>
    }
    const version = (data.tag_name ?? '').replace(/^v/, '')
    if (!version) return null
    const assets = data.assets ?? []
    const dmg = assets.find((a) => a.name.endsWith('.dmg'))
    const zip = assets.find(
      (a) => a.name.endsWith('.zip') && !a.name.endsWith('.sha256'),
    )
    return {
      version,
      dmgUrl: dmg?.browser_download_url ?? null,
      zipUrl: zip?.browser_download_url ?? null,
      htmlUrl: data.html_url ?? FALLBACK_RELEASES_URL,
    }
  } catch {
    return null
  }
}
