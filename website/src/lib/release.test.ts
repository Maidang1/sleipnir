import { describe, expect, it } from 'vitest'
import { downloadItems } from '../components/download-menu'
import { FALLBACK_RELEASES_URL, parseLatestRelease } from './release'

describe('parseLatestRelease', () => {
  it('discovers current Windows and both Linux architecture assets', () => {
    const release = parseLatestRelease({
      tag_name: 'v0.3.0',
      html_url: 'https://example.test/v0.3.0',
      assets: [
        { name: 'Sleipnir-0.3.0-macos.dmg', browser_download_url: 'dmg' },
        { name: 'Sleipnir-0.3.0-windows-x64.exe', browser_download_url: 'exe' },
        { name: 'sleipnir_0.3.0_amd64.deb', browser_download_url: 'deb-x64' },
        { name: 'Sleipnir-0.3.0-linux-x86_64.tar.gz', browser_download_url: 'tar-x64' },
        { name: 'sleipnir_0.3.0_arm64.deb', browser_download_url: 'deb-arm' },
        { name: 'Sleipnir-0.3.0-linux-aarch64.tar.gz', browser_download_url: 'tar-arm' },
      ],
    })

    expect(release).toMatchObject({
      version: '0.3.0',
      dmgUrl: 'dmg',
      windowsExeUrl: 'exe',
      linuxX64DebUrl: 'deb-x64',
      linuxX64TarUrl: 'tar-x64',
      linuxArm64DebUrl: 'deb-arm',
      linuxArm64TarUrl: 'tar-arm',
      htmlUrl: 'https://example.test/v0.3.0',
    })
  })

  it('matches only release assets and never SHA-256 sidecars', () => {
    const release = parseLatestRelease({
      tag_name: '0.3.0',
      assets: [
        { name: 'Sleipnir-0.3.0-macos.dmg.sha256', browser_download_url: 'dmg-sha' },
        { name: 'Sleipnir-0.3.0-windows-x64.exe.sha256', browser_download_url: 'exe-sha' },
        { name: 'sleipnir_0.3.0_amd64.deb.sha256', browser_download_url: 'deb-x64-sha' },
        { name: 'Sleipnir-0.3.0-linux-x86_64.tar.gz.sha256', browser_download_url: 'tar-x64-sha' },
        { name: 'sleipnir_0.3.0_arm64.deb.sha256', browser_download_url: 'deb-arm-sha' },
        { name: 'Sleipnir-0.3.0-linux-aarch64.tar.gz.sha256', browser_download_url: 'tar-arm-sha' },
      ],
    })

    expect(release).toEqual({
      version: '0.3.0',
      dmgUrl: null,
      windowsExeUrl: null,
      linuxX64DebUrl: null,
      linuxX64TarUrl: null,
      linuxArm64DebUrl: null,
      linuxArm64TarUrl: null,
      htmlUrl: FALLBACK_RELEASES_URL,
    })
  })

  it('returns null when the release has no version tag', () => {
    expect(parseLatestRelease({ assets: [] })).toBeNull()
  })

  it('orders every available download with an architecture-labelled menu item', () => {
    const release = parseLatestRelease({
      tag_name: 'v0.3.0',
      html_url: 'https://example.test/v0.3.0',
      assets: [
        { name: 'Sleipnir-0.3.0-macos.dmg', browser_download_url: 'dmg' },
        { name: 'Sleipnir-0.3.0-windows-x64.exe', browser_download_url: 'exe' },
        { name: 'sleipnir_0.3.0_amd64.deb', browser_download_url: 'deb-x64' },
        { name: 'Sleipnir-0.3.0-linux-x86_64.tar.gz', browser_download_url: 'tar-x64' },
        { name: 'sleipnir_0.3.0_arm64.deb', browser_download_url: 'deb-arm' },
        { name: 'Sleipnir-0.3.0-linux-aarch64.tar.gz', browser_download_url: 'tar-arm' },
      ],
    })

    expect(downloadItems(release).map((item) => item.label)).toEqual([
      'macOS (.dmg)',
      'Windows x64 (.exe)',
      'Linux x86_64 (.deb)',
      'Linux x86_64 (.tar.gz)',
      'Linux ARM64 (.deb)',
      'Linux ARM64 (.tar.gz)',
      'All releases',
    ])
  })
})
