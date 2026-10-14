import { describe, expect, it } from 'vitest'
import {
  assetNames,
  BUNDLE_ID,
  debControl,
  desktopEntry,
  infoPlist,
  serverArchiveName,
  windowsVersion
} from '../packages/core/scripts/package-native.mjs'
import { electronFeed, feeds, nativeFeed } from '../packages/core/scripts/release-feeds.mjs'

const version = '0.8.0-beta.9'

function fakeFiles() {
  const files: Record<string, { sha256: string; sha512: string; size: number }> = {}
  const names = [
    ...Object.values(assetNames(version, 'darwin', 'arm64')),
    ...Object.values(assetNames(version, 'darwin', 'x64')),
    ...Object.values(assetNames(version, 'win32', 'x64')),
    ...Object.values(assetNames(version, 'linux', 'x64'))
  ]
  names.forEach((n, i) => {
    files[n] = { sha256: `sha256-${i}`, sha512: `sha512-${i}`, size: 100 + i }
  })
  return files
}

describe('native packages', () => {
  it('keeps the Electron release asset names', () => {
    expect(assetNames(version, 'darwin', 'arm64')).toEqual({
      zip: 'Vorn-0.8.0-beta.9-arm64-mac.zip',
      dmg: 'Vorn-0.8.0-beta.9-arm64.dmg'
    })
    expect(assetNames(version, 'darwin', 'x64')).toEqual({
      zip: 'Vorn-0.8.0-beta.9-mac.zip',
      dmg: 'Vorn-0.8.0-beta.9-x64.dmg'
    })
    expect(assetNames(version, 'win32', 'x64')).toEqual({ setup: 'Vorn-Setup-0.8.0-beta.9.exe' })
    expect(assetNames(version, 'linux', 'x64')).toEqual({
      appimage: 'Vorn-0.8.0-beta.9.AppImage',
      deb: 'vorn_0.8.0-beta.9_amd64.deb'
    })
  })

  it('names the server archive install.sh downloads', () => {
    expect(serverArchiveName(version, 'linux', 'arm64')).toBe(
      'vorn-server-0.8.0-beta.9-linux-arm64.tar.gz'
    )
  })

  it('keeps the bundle identity and registers vorn:// links', () => {
    const plist = infoPlist(version)
    expect(BUNDLE_ID).toBe('com.vorn.app')
    expect(plist).toContain('<key>CFBundleIdentifier</key>\n\t<string>com.vorn.app</string>')
    expect(plist).toContain('<key>CFBundleExecutable</key>\n\t<string>Vorn</string>')
    expect(plist).toContain('<key>CFBundleURLSchemes</key>')
    expect(plist).toContain('<string>vorn</string>')
    expect(plist).toContain('<string>0.8.0-beta.9</string>')
  })

  it('gives Windows a four-part version', () => {
    expect(windowsVersion('0.8.0-beta.9')).toBe('0.8.0.9')
    expect(windowsVersion('1.2.3')).toBe('1.2.3.0')
  })

  it('describes the Linux packages', () => {
    expect(desktopEntry('/opt/Vorn/vorn')).toContain('Exec=/opt/Vorn/vorn %U')
    expect(desktopEntry('vorn')).toContain('MimeType=x-scheme-handler/vorn;')
    const control = debControl(version, 1234)
    expect(control).toContain('Package: vorn\n')
    expect(control).toContain('Version: 0.8.0-beta.9\n')
    expect(control).toContain('Installed-Size: 1234\n')
  })
})

describe('release feeds', () => {
  const releaseDate = '2026-10-20T10:00:00.000Z'
  const base = { version, tag: `v${version}`, repo: 'vorn-run/vorn', releaseDate }

  it('lists one self-updating asset per platform in the native feed', () => {
    const feed = nativeFeed({ ...base, files: fakeFiles() })
    expect(feed.version).toBe(version)
    expect(feed.assets.map((a) => [a.key, a.name])).toEqual([
      ['mac-arm64', 'Vorn-0.8.0-beta.9-arm64-mac.zip'],
      ['mac-x64', 'Vorn-0.8.0-beta.9-mac.zip'],
      ['win-x64', 'Vorn-Setup-0.8.0-beta.9.exe'],
      ['linux-x64', 'Vorn-0.8.0-beta.9.AppImage']
    ])
    expect(feed.assets[0].url).toBe(
      'https://github.com/vorn-run/vorn/releases/download/v0.8.0-beta.9/Vorn-0.8.0-beta.9-arm64-mac.zip'
    )
    expect(feed.assets[0]).toMatchObject({ sha256: 'sha256-0', size: 100 })
  })

  it('writes the Electron updater feed in its format', () => {
    const files = fakeFiles()
    const yml = electronFeed({
      version,
      releaseDate,
      names: ['Vorn-Setup-0.8.0-beta.9.exe'],
      files
    })
    const sum = files['Vorn-Setup-0.8.0-beta.9.exe']
    expect(yml).toBe(`version: 0.8.0-beta.9
files:
  - url: Vorn-Setup-0.8.0-beta.9.exe
    sha512: ${sum.sha512}
    size: ${sum.size}
path: Vorn-Setup-0.8.0-beta.9.exe
sha512: ${sum.sha512}
releaseDate: '2026-10-20T10:00:00.000Z'
`)
  })

  it('publishes the beta feeds beside the latest ones for a prerelease only', () => {
    const files = fakeFiles()
    expect(Object.keys(feeds({ ...base, files })).sort()).toEqual([
      'beta-linux.yml',
      'beta-mac.yml',
      'beta.yml',
      'latest-linux.yml',
      'latest-mac.yml',
      'latest.yml',
      'vorn-update.json'
    ])
    const stableFiles: typeof files = {}
    for (const [name, v] of Object.entries(files)) stableFiles[name.replace('-beta.9', '')] = v
    const stable = feeds({ ...base, version: '0.8.0', tag: 'v0.8.0', files: stableFiles })
    expect(Object.keys(stable).some((n) => n.startsWith('beta'))).toBe(false)
  })

  it('puts the arm64 zip first for Macs and keeps the Intel one', () => {
    const mac = feeds({ ...base, files: fakeFiles() })['latest-mac.yml']
    expect(mac).toContain('path: Vorn-0.8.0-beta.9-arm64-mac.zip')
    expect(mac).toContain('  - url: Vorn-0.8.0-beta.9-mac.zip')
  })
})
