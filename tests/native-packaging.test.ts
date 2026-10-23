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
import { execFileSync } from 'node:child_process'
import { generateKeyPairSync } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { feeds, nativeFeed, releaseAssets } from '../packages/core/scripts/release-feeds.mjs'
import { publicBase64, signFeed, verifyFeed } from '../packages/core/scripts/sign-feed.mjs'
import {
  compareVersions,
  nextPatch,
  previousRelease,
  servedFeed
} from '../packages/core/scripts/update-path.mjs'

const version = '0.8.0-beta.9'

function fakeFiles() {
  const files: Record<string, { sha256: string; size: number }> = {}
  const names = [
    ...Object.values(assetNames(version, 'darwin', 'arm64')),
    ...Object.values(assetNames(version, 'darwin', 'x64')),
    ...Object.values(assetNames(version, 'win32', 'x64')),
    ...Object.values(assetNames(version, 'linux', 'x64'))
  ]
  names.forEach((n, i) => {
    files[n] = { sha256: `sha256-${i}`, size: 100 + i }
  })
  return files
}

describe('native packages', () => {
  it('names the release assets', () => {
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

  it('names Vorn and why in every privacy prompt', () => {
    const plist = infoPlist(version)
    const asks = 'A program running in Vorn wants to'
    for (const key of [
      'NSAppleEventsUsageDescription',
      'NSCameraUsageDescription',
      'NSMicrophoneUsageDescription',
      'NSAudioCaptureUsageDescription',
      'NSContactsUsageDescription',
      'NSCalendarsUsageDescription',
      'NSCalendarsFullAccessUsageDescription',
      'NSRemindersUsageDescription',
      'NSRemindersFullAccessUsageDescription',
      'NSPhotoLibraryUsageDescription',
      'NSLocationUsageDescription',
      'NSLocationWhenInUseUsageDescription',
      'NSLocalNetworkUsageDescription',
      'NSBluetoothAlwaysUsageDescription',
      'NSSpeechRecognitionUsageDescription',
      'NSMotionUsageDescription',
      'NSSystemAdministrationUsageDescription'
    ]) {
      const said = new RegExp(`<key>${key}</key>\\n\\t<string>${asks} [^<]+\\.</string>`)
      expect(plist).toMatch(said)
    }
    expect(plist).toContain(
      `<key>NSCameraUsageDescription</key>\n\t<string>${asks} use the camera.</string>`
    )
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

  it('publishes only the native feed', () => {
    expect(Object.keys(feeds({ ...base, files: fakeFiles() }))).toEqual(['vorn-update.json'])
  })

  it('lists every release file with its SHA-256 for the install scripts', () => {
    const files = fakeFiles()
    const feed = nativeFeed({ ...base, files })
    expect(Object.keys(feed.checksums).sort()).toEqual(Object.keys(files).sort())
    expect(feed.checksums['Vorn-0.8.0-beta.9-arm64.dmg']).toMatch(/^sha256-/)
    // install.sh reads one `"name": "hash"` pair a line.
    const text = feeds({ ...base, files })['vorn-update.json']
    expect(text).toContain('"Vorn-0.8.0-beta.9.AppImage": "')
  })

  it('refuses to write a release feed without the server archives it must list', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-feed-'))
    for (const name of Object.values(releaseAssets('9.9.9')) as string[]) {
      fs.writeFileSync(path.join(dir, name), 'x')
    }
    const script = path.resolve(__dirname, '../packages/core/scripts/release-feeds.mjs')
    const args = [script, '--dir', dir, '--version', '9.9.9', '--servers', 'linux-arm64']
    const write = () => execFileSync('node', args, { stdio: 'pipe' })
    expect(write).toThrow()
    fs.writeFileSync(path.join(dir, 'vorn-server-9.9.9-linux-arm64.tar.gz'), 'y')
    write()
    const feed = JSON.parse(fs.readFileSync(path.join(dir, 'vorn-update.json'), 'utf8'))
    expect(feed.checksums['vorn-server-9.9.9-linux-arm64.tar.gz']).toMatch(/^[0-9a-f]{64}$/)
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('signs the feed so only its own key verifies it', () => {
    const { privateKey } = generateKeyPairSync('ed25519')
    const other = generateKeyPairSync('ed25519').privateKey
    const body = Buffer.from(feeds({ ...base, files: fakeFiles() })['vorn-update.json'])
    const signature = signFeed(body, privateKey)
    const key = publicBase64(privateKey)
    expect(Buffer.from(key, 'base64')).toHaveLength(32)
    expect(verifyFeed(body, signature, key)).toBe(true)
    expect(verifyFeed(Buffer.concat([body, Buffer.from(' ')]), signature, key)).toBe(false)
    expect(verifyFeed(body, signFeed(body, other), key)).toBe(false)
  })
})

describe('update path', () => {
  it('orders versions as semver does', () => {
    const sorted = ['0.7.4', '0.8.0-beta.2', '0.8.0-beta.10', '0.8.0-rc.1', '0.8.0', '0.8.1']
    for (let i = 1; i < sorted.length; i++) {
      expect(compareVersions(sorted[i - 1], sorted[i])).toBeLessThan(0)
      expect(compareVersions(sorted[i], sorted[i - 1])).toBeGreaterThan(0)
    }
    expect(compareVersions('v0.8.0-beta.9', '0.8.0-beta.9')).toBe(0)
    expect(() => compareVersions('junk', '0.8.0')).toThrow()
  })

  it('rehearses under a version above any prerelease of the build', () => {
    expect(nextPatch('0.8.0-beta.9')).toBe('0.8.1')
    expect(compareVersions(nextPatch('0.8.0-beta.9'), '0.8.0')).toBeGreaterThan(0)
    expect(nextPatch('1.2.3')).toBe('1.2.4')
  })

  it('starts from the newest published release before the build', () => {
    const releases = [
      { tag_name: 'v0.8.0-beta.11', draft: true },
      { tag_name: 'v0.8.0-beta.10', draft: false },
      { tag_name: 'v0.8.0-beta.9', draft: false },
      { tag_name: 'v0.7.4', draft: false },
      { tag_name: 'nightly', draft: false }
    ]
    expect(previousRelease(releases, '0.8.0-beta.11')?.tag_name).toBe('v0.8.0-beta.10')
    expect(previousRelease(releases, '0.8.0-beta.10')?.tag_name).toBe('v0.8.0-beta.9')
    expect(previousRelease(releases, '0.7.4')).toBeNull()
  })

  it('serves every asset locally under the version asked for', () => {
    const feed = nativeFeed({
      version,
      tag: `v${version}`,
      repo: 'vorn-run/vorn',
      releaseDate: '2026-01-01T00:00:00.000Z',
      files: fakeFiles()
    })
    const served = servedFeed(feed, 'http://127.0.0.1:9', '0.8.1')
    expect(served.version).toBe('0.8.1')
    expect(served.assets.map((a: { url: string }) => a.url)).toContain(
      'http://127.0.0.1:9/Vorn-0.8.0-beta.9-arm64-mac.zip'
    )
    expect(served.assets.map((a: { sha256: string }) => a.sha256)).toEqual(
      feed.assets.map((a: { sha256: string }) => a.sha256)
    )
  })
})
