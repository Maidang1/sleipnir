export interface LatestRelease {
  version: string
  dmgUrl: string | null
  zipUrl: string | null
  windowsZipUrl: string | null
  linuxDebUrl: string | null
  linuxTarUrl: string | null
  htmlUrl: string
}

export const GITHUB_REPO = 'Maidang1/sleipnir'
export const GITHUB_URL = `https://github.com/${GITHUB_REPO}`
export const FALLBACK_RELEASES_URL = `${GITHUB_URL}/releases`
export const INSTALL_SCRIPT_URL = `https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/install.sh`
export const INSTALL_PS1_URL = `https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/install.ps1`
export const INSTALL_LINUX_URL = `https://raw.githubusercontent.com/${GITHUB_REPO}/main/scripts/install-linux.sh`
export const INSTALL_COMMAND = `curl -fsSL ${INSTALL_SCRIPT_URL} | bash`

export type InstallPlatform = 'macos' | 'windows' | 'linux'

export const INSTALL_COMMANDS: Record<InstallPlatform, string> = {
  macos: `curl -fsSL ${INSTALL_SCRIPT_URL} | bash`,
  windows: `irm ${INSTALL_PS1_URL} | iex`,
  linux: `curl -fsSL ${INSTALL_LINUX_URL} | bash`,
}

export const INSTALL_HINTS: Record<InstallPlatform, string> = {
  macos:
    'Verifies SHA-256, installs to /Applications, and clears Gatekeeper quarantine with xattr -cr.',
  windows:
    'Verifies SHA-256, installs sleipnir.exe to %LOCALAPPDATA%\\Sleipnir, and launches it.',
  linux:
    'On Ubuntu/Debian, downloads the latest .deb and installs it with apt. Needs a Vulkan driver (libvulkan1 + mesa-vulkan-drivers). Set SLEIPNIR_TARBALL=1 for the portable tarball.',
}

export function detectInstallPlatform(): InstallPlatform {
  if (typeof navigator === 'undefined') return 'macos'
  if (/Windows/i.test(navigator.userAgent)) return 'windows'
  if (/Linux/i.test(navigator.userAgent)) return 'linux'
  return 'macos'
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
      (a) => a.name.endsWith('-macos.zip') && !a.name.endsWith('.sha256'),
    )
    const windowsZip = assets.find(
      (a) =>
        a.name.endsWith('-windows-x64.zip') && !a.name.endsWith('.sha256'),
    )
    const linuxDeb = assets.find((a) => a.name.endsWith('.deb'))
    const linuxTar = assets.find(
      (a) => a.name.includes('-linux-') && a.name.endsWith('.tar.gz'),
    )
    return {
      version,
      dmgUrl: dmg?.browser_download_url ?? null,
      zipUrl: zip?.browser_download_url ?? null,
      windowsZipUrl: windowsZip?.browser_download_url ?? null,
      linuxDebUrl: linuxDeb?.browser_download_url ?? null,
      linuxTarUrl: linuxTar?.browser_download_url ?? null,
      htmlUrl: data.html_url ?? FALLBACK_RELEASES_URL,
    }
  } catch {
    return null
  }
}
