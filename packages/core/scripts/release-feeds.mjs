#!/usr/bin/env node
// Writes a release's update feed from its assets, after every platform has
// packaged (package-native.mjs):
//
//   vorn-update.json   per platform, the asset with its sha256 and size, and
//                      `checksums`: every release file there by name, which
//                      install.sh and install.ps1 check their download against
//
// sign-feed.mjs then signs it; the app trusts the feed only with that signature.
//
//   --dir <dir>      where the assets are, and where the feeds go (default: dist-native)
//   --version <v>    default: the root package.json's
//   --repo <o/r>     default: vorn-run/vorn
//   --servers <list> server archives the feed must list, e.g. linux-arm64
import { createHash } from 'node:crypto'
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { assetNames, serverArchiveName } from './package-native.mjs'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')

/** Every asset a release has, by the native feed's key. */
export function releaseAssets(version) {
  const armMac = assetNames(version, 'darwin', 'arm64')
  const intelMac = assetNames(version, 'darwin', 'x64')
  const linux = assetNames(version, 'linux', 'x64')
  return {
    'mac-arm64': armMac.zip,
    'mac-x64': intelMac.zip,
    'win-x64': assetNames(version, 'win32', 'x64').setup,
    'linux-x64': linux.appimage,
    'mac-arm64-dmg': armMac.dmg,
    'mac-x64-dmg': intelMac.dmg,
    'linux-x64-deb': linux.deb
  }
}

/** The native feed's keys: the assets an install updates itself from. */
export const NATIVE_KEYS = ['mac-arm64', 'mac-x64', 'win-x64', 'linux-x64']

/** The native app's feed; `files` maps an asset's name to its { sha256, size }. */
export function nativeFeed({ version, tag, repo: slug, releaseDate, files }) {
  const assets = releaseAssets(version)
  const checksums = Object.fromEntries(
    Object.keys(files)
      .sort()
      .map((name) => [name, files[name].sha256])
  )
  return {
    version,
    releaseDate,
    checksums,
    assets: NATIVE_KEYS.map((key) => {
      const name = assets[key]
      return {
        key,
        name,
        url: `https://github.com/${slug}/releases/download/${tag}/${name}`,
        sha256: files[name].sha256,
        size: files[name].size
      }
    })
  }
}

/** Every feed file a release publishes, by name. */
export function feeds({ version, tag, repo: slug, releaseDate, files }) {
  return {
    'vorn-update.json':
      JSON.stringify(nativeFeed({ version, tag, repo: slug, releaseDate, files }), null, 2) + '\n'
  }
}

function digest(file) {
  const data = readFileSync(file)
  return {
    sha256: createHash('sha256').update(data).digest('hex'),
    size: statSync(file).size
  }
}

function main() {
  const args = {}
  const argv = process.argv.slice(2)
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) {
      throw new Error(`bad argument ${argv[i]}`)
    }
    args[argv[i].slice(2)] = argv[i + 1]
  }
  const dir = path.resolve(args.dir ?? path.join(repo, 'dist-native'))
  const version =
    args.version ?? JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8')).version
  const files = {}
  const missing = []
  for (const name of Object.values(releaseAssets(version))) {
    const file = path.join(dir, name)
    if (existsSync(file)) files[name] = digest(file)
    else missing.push(name)
  }
  if (missing.length) throw new Error(`missing assets: ${missing.join(', ')}`)
  // The server archives install.sh fetches; a release names the ones it must have.
  for (const name of readdirSync(dir)) {
    if (/^vorn-server-.*\.tar\.gz$/.test(name)) files[name] = digest(path.join(dir, name))
  }
  for (const target of (args.servers ?? '').split(',').filter(Boolean)) {
    const name = serverArchiveName(version, ...target.split('-'))
    if (!files[name]) throw new Error(`missing server archive: ${name}`)
  }
  const written = feeds({
    version,
    tag: `v${version}`,
    repo: args.repo ?? 'vorn-run/vorn',
    releaseDate: new Date().toISOString(),
    files
  })
  for (const [name, body] of Object.entries(written)) {
    writeFileSync(path.join(dir, name), body)
    console.log(`wrote ${name}`)
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (e) {
    console.error(`release-feeds: ${e.message}`)
    process.exit(1)
  }
}
