export interface LatestRelease {
  version: string
  dmgUrl: string | null
  windowsExeUrl: string | null
  linuxX64DebUrl: string | null
  linuxX64TarUrl: string | null
  linuxArm64DebUrl: string | null
  linuxArm64TarUrl: string | null
  htmlUrl: string
}

export interface GitHubReleasePayload {
  tag_name?: string
  html_url?: string
  assets?: Array<{ name: string; browser_download_url: string }>
}

export const GITHUB_REPO = 'Maidang1/sleipnir'
export const GITHUB_URL = `https://github.com/${GITHUB_REPO}`
export const FALLBACK_RELEASES_URL = `${GITHUB_URL}/releases`
export const INSTALL_SCRIPT_URL = `https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/install.sh`
export const INSTALL_COMMAND = `curl -fsSL ${INSTALL_SCRIPT_URL} | bash`
export const INSTALL_HINT =
  'Detects macOS or Linux, verifies SHA-256, and installs the matching release.'

export function parseLatestRelease(data: GitHubReleasePayload): LatestRelease | null {
  const version = (data.tag_name ?? '').replace(/^v/, '')
  if (!version) return null

  const assets = data.assets ?? []
  const assetUrl = (suffix: string) =>
    assets.find((asset) => asset.name.endsWith(suffix) && !asset.name.endsWith('.sha256'))
      ?.browser_download_url ?? null

  return {
    version,
    dmgUrl: assetUrl('-macos.dmg'),
    windowsExeUrl: assetUrl('-windows-x64.exe'),
    linuxX64DebUrl: assetUrl('_amd64.deb'),
    linuxX64TarUrl: assetUrl('-linux-x86_64.tar.gz'),
    linuxArm64DebUrl: assetUrl('_arm64.deb'),
    linuxArm64TarUrl: assetUrl('-linux-aarch64.tar.gz'),
    htmlUrl: data.html_url ?? FALLBACK_RELEASES_URL,
  }
}

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
    return parseLatestRelease((await res.json()) as GitHubReleasePayload)
  } catch {
    return null
  }
}
