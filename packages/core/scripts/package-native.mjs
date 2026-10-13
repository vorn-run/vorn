#!/usr/bin/env node
// Packages the native desktop app, vorn-app, with the vornd and vorn-sessiond
// it starts when none is running, into one release asset per platform:
//
//   macOS    Vorn-Native-<version>-mac-<arch>.zip   "Vorn Native.app"
//   Windows  Vorn-Native-<version>-win-x64.zip      "Vorn Native/vorn-app.exe" and the daemons
//   Linux    Vorn-Native-<version>-linux-x64.tar.gz "vorn-native/vorn-app" and the daemons
//
//   --bin-dir <dir>     where vorn-app is (default: target/release)
//   --daemon-dir <dir>  where vornd and vorn-sessiond are, and on Windows the
//                       ConPTY beside them (default: --bin-dir)
//   --out <dir>         default: dist-native at the repository root
//   --version <v>       default: the root package.json's
//
// On macOS the bundle is signed with the Developer ID identity in the keychain
// (or VORN_SIGN_IDENTITY), and notarized when APPLE_ID,
// APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID are set; without an identity
// it is signed ad hoc, as it is with VORN_SIGN_IDENTITY=-.
import { execFileSync } from 'node:child_process'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync
} from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const core = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repo = path.resolve(core, '..', '..')

export const APP_NAME = 'Vorn Native'
export const BUNDLE_ID = 'com.vorn.native'

/** The asset's file name, the same on every platform but for the suffix. */
export function assetName(version, platform, arch) {
  const os = { darwin: 'mac', win32: 'win', linux: 'linux' }[platform]
  if (!os) throw new Error(`no native package for ${platform}`)
  const ext = platform === 'linux' ? 'tar.gz' : 'zip'
  return `Vorn-Native-${version}-${os}-${arch}.${ext}`
}

/** The bundle's Info.plist. */
export function infoPlist(version) {
  const entries = {
    CFBundleDevelopmentRegion: 'en',
    CFBundleDisplayName: APP_NAME,
    CFBundleExecutable: 'vorn-app',
    CFBundleIconFile: 'icon',
    CFBundleIdentifier: BUNDLE_ID,
    CFBundleInfoDictionaryVersion: '6.0',
    CFBundleName: APP_NAME,
    CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: version,
    CFBundleVersion: version,
    LSApplicationCategoryType: 'public.app-category.developer-tools',
    LSMinimumSystemVersion: '12.0',
    NSHighResolutionCapable: true,
    NSPrincipalClass: 'NSApplication',
    NSSupportsAutomaticGraphicsSwitching: true
  }
  const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  const body = Object.entries(entries)
    .map(([k, v]) => {
      const value = typeof v === 'boolean' ? `<${v}/>` : `<string>${esc(v)}</string>`
      return `\t<key>${k}</key>\n\t${value}`
    })
    .join('\n')
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${body}
</dict>
</plist>
`
}

function parseArgs(argv) {
  const out = {}
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]
    if (!key.startsWith('--') || argv[i + 1] === undefined) throw new Error(`bad argument ${key}`)
    out[key.slice(2)] = argv[i + 1]
  }
  return out
}

function run(cmd, args, opts = {}) {
  console.log(`$ ${cmd} ${args.join(' ')}`)
  return execFileSync(cmd, args, {
    stdio: opts.capture ? 'pipe' : 'inherit',
    encoding: 'utf8',
    ...opts
  })
}

function copyExe(from, to) {
  if (!existsSync(from)) throw new Error(`${from} is missing; build it first`)
  mkdirSync(path.dirname(to), { recursive: true })
  copyFileSync(from, to)
  chmodSync(to, 0o755)
}

/** The Developer ID identity to sign with, or null for ad hoc (also `VORN_SIGN_IDENTITY=-`). */
function signingIdentity() {
  const named = process.env.VORN_SIGN_IDENTITY
  if (named) return named === '-' ? null : named
  try {
    const found = execFileSync('security', ['find-identity', '-v', '-p', 'codesigning'], {
      encoding: 'utf8'
    })
    return found.match(/"(Developer ID Application: [^"]+)"/)?.[1] ?? null
  } catch {
    return null
  }
}

function notarize(app, zip) {
  const { APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID } = process.env
  run('ditto', ['-c', '-k', '--norsrc', '--noextattr', '--keepParent', app, zip])
  const auth = [
    '--apple-id',
    APPLE_ID,
    '--password',
    APPLE_APP_SPECIFIC_PASSWORD,
    '--team-id',
    APPLE_TEAM_ID
  ]
  console.log('$ xcrun notarytool submit … --wait')
  const out = execFileSync(
    'xcrun',
    ['notarytool', 'submit', zip, ...auth, '--wait', '--timeout', '30m', '--output-format', 'json'],
    { encoding: 'utf8' }
  )
  const result = JSON.parse(out)
  if (result.status !== 'Accepted') {
    try {
      execFileSync('xcrun', ['notarytool', 'log', result.id, ...auth], { stdio: 'inherit' })
    } catch {}
    throw new Error(`notarization ${result.status ?? 'failed'}: ${result.message ?? ''}`)
  }
  rmSync(zip)
  run('xcrun', ['stapler', 'staple', app])
}

function packageMac({ binDir, daemonDir, out, version }) {
  const app = path.join(out, `${APP_NAME}.app`)
  rmSync(app, { recursive: true, force: true })
  const contents = path.join(app, 'Contents')
  copyExe(path.join(binDir, 'vorn-app'), path.join(contents, 'MacOS', 'vorn-app'))
  for (const d of ['vornd', 'vorn-sessiond']) {
    copyExe(path.join(daemonDir, d), path.join(contents, 'Resources', 'vornd', d))
  }
  copyFileSync(
    path.join(repo, 'resources', 'icon.icns'),
    path.join(contents, 'Resources', 'icon.icns')
  )
  writeFileSync(path.join(contents, 'Info.plist'), infoPlist(version))
  writeFileSync(path.join(contents, 'PkgInfo'), 'APPL????')

  const identity = signingIdentity()
  const entitlements = path.join(repo, 'resources', 'entitlements.mac.plist')
  const sign = (file) =>
    identity
      ? run('codesign', [
          '--force',
          '--timestamp',
          '--options',
          'runtime',
          '--entitlements',
          entitlements,
          '--sign',
          identity,
          file
        ])
      : run('codesign', ['--force', '--sign', '-', file])
  // Inside out: the daemons, then the bundle, which seals them.
  for (const d of ['vornd', 'vorn-sessiond']) sign(path.join(contents, 'Resources', 'vornd', d))
  sign(app)
  run('codesign', ['--verify', '--deep', '--strict', app])

  const canNotarize =
    identity &&
    ['APPLE_ID', 'APPLE_APP_SPECIFIC_PASSWORD', 'APPLE_TEAM_ID'].every((k) => process.env[k])
  if (canNotarize) notarize(app, path.join(out, 'notarize.zip'))

  const asset = path.join(out, assetName(version, 'darwin', process.arch))
  rmSync(asset, { force: true })
  run('ditto', ['-c', '-k', '--norsrc', '--noextattr', '--keepParent', app, asset])
  return {
    asset,
    signing: identity ? `Developer ID (${identity})` : 'ad hoc',
    notarized: Boolean(canNotarize)
  }
}

function packageWindows({ binDir, daemonDir, out, version }) {
  const dir = path.join(out, APP_NAME)
  rmSync(dir, { recursive: true, force: true })
  copyExe(path.join(binDir, 'vorn-app.exe'), path.join(dir, 'vorn-app.exe'))
  for (const d of [
    'vornd.exe',
    'vorn-sessiond.exe',
    'conpty.dll',
    path.join('x64', 'OpenConsole.exe')
  ]) {
    copyExe(path.join(daemonDir, d), path.join(dir, d))
  }
  const asset = path.join(out, assetName(version, 'win32', 'x64'))
  rmSync(asset, { force: true })
  run('powershell', [
    '-NoProfile',
    '-Command',
    `Compress-Archive -Path '${dir}' -DestinationPath '${asset}'`
  ])
  return { asset, signing: 'unsigned', notarized: false }
}

function packageLinux({ binDir, daemonDir, out, version }) {
  const dir = path.join(out, 'vorn-native')
  rmSync(dir, { recursive: true, force: true })
  copyExe(path.join(binDir, 'vorn-app'), path.join(dir, 'vorn-app'))
  for (const d of ['vornd', 'vorn-sessiond']) copyExe(path.join(daemonDir, d), path.join(dir, d))
  copyFileSync(path.join(repo, 'resources', 'icon.png'), path.join(dir, 'icon.png'))
  const asset = path.join(out, assetName(version, 'linux', 'x64'))
  run('tar', ['-czf', asset, '-C', out, 'vorn-native'])
  return { asset, signing: 'unsigned', notarized: false }
}

function main() {
  const args = parseArgs(process.argv.slice(2))
  const binDir = path.resolve(args['bin-dir'] ?? path.join(core, 'target', 'release'))
  const opts = {
    binDir,
    daemonDir: path.resolve(args['daemon-dir'] ?? binDir),
    out: path.resolve(args.out ?? path.join(repo, 'dist-native')),
    version:
      args.version ?? JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8')).version
  }
  mkdirSync(opts.out, { recursive: true })
  const pack = { darwin: packageMac, win32: packageWindows, linux: packageLinux }[process.platform]
  if (!pack) throw new Error(`no native package for ${process.platform}`)
  const { asset, signing, notarized } = pack(opts)
  const line = `${path.basename(asset)}: signed ${signing}, notarized ${notarized ? 'yes' : 'no'}`
  console.log(line)
  if (process.env.GITHUB_STEP_SUMMARY)
    writeFileSync(process.env.GITHUB_STEP_SUMMARY, `- ${line}\n`, { flag: 'a' })
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (e) {
    console.error(`package-native: ${e.message}`)
    process.exit(1)
  }
}
