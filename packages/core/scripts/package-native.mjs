#!/usr/bin/env node
// Packages Vorn, the native desktop app (vorn-app), with the vornd,
// vorn-sessiond and vorn it ships and the web client vornd serves, into this
// platform's release assets:
//
//   macOS    Vorn-<v>-arm64-mac.zip and Vorn-<v>-arm64.dmg (x64: Vorn-<v>-mac.zip, Vorn-<v>.dmg)
//   Windows  Vorn-Setup-<v>.exe, an NSIS installer (scripts/vorn.nsi)
//   Linux    Vorn-<v>.AppImage and vorn_<v>_amd64.deb
//   server   vorn-server-<v>-<os>-<arch>.tar.gz (--package server), for machines without a desktop
//
// The app's identity (com.vorn.app, "Vorn", its executable and install folder)
// stays the same from release to release, so an install updates in place:
// Vorn.app/Contents/MacOS/Vorn, %LOCALAPPDATA%\Programs\vorn\Vorn.exe and
// /opt/Vorn/vorn, each with resources/vornd and resources/web/dist.
//
//   --bin-dir <dir>     where vorn-app is (default: target/release)
//   --daemon-dir <dir>  where vornd, vorn-sessiond, vorn and, on Windows, the
//                       ConPTY beside them are (default: --bin-dir)
//   --web-dir <dir>     the built web client (default: packages/core/crates/vorn-web/dist)
//   --out <dir>         default: dist-native at the repository root
//   --version <v>       default: the root package.json's
//   --package <kind>    app (default) or server
//
// macOS signs with the Developer ID identity in the keychain (or
// VORN_SIGN_IDENTITY; `-` for ad hoc) and notarizes when APPLE_ID,
// APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID are set. Windows signs when
// WIN_CERTIFICATE (a base64 .pfx) and WIN_CERTIFICATE_PASSWORD are set and is
// unsigned otherwise. Linux needs appimagetool on PATH or in APPIMAGETOOL.
import { execFileSync } from 'node:child_process'
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync
} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const core = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repo = path.resolve(core, '..', '..')

export const APP_NAME = 'Vorn'
export const BUNDLE_ID = 'com.vorn.app'
export const URL_SCHEME = 'vorn'
/** The Windows installer's key for com.vorn.app; installs are found by it, so it never changes. */
export const WINDOWS_GUID = '51b24bfb-ca41-5f10-85db-83383073071d'

/** This platform's release assets, by kind. */
export function assetNames(version, platform, arch) {
  if (platform === 'darwin') {
    const a = arch === 'arm64' ? '-arm64' : ''
    return { zip: `Vorn-${version}${a}-mac.zip`, dmg: `Vorn-${version}-${arch}.dmg` }
  }
  if (platform === 'win32') return { setup: `Vorn-Setup-${version}.exe` }
  if (platform === 'linux') {
    return { appimage: `Vorn-${version}.AppImage`, deb: `vorn_${version}_amd64.deb` }
  }
  throw new Error(`no package for ${platform}`)
}

/** The server-only archive for `platform` and `arch` (Node's names: linux, arm64). */
export function serverArchiveName(version, platform, arch) {
  return `vorn-server-${version}-${platform}-${arch}.tar.gz`
}

/** The four-part version Windows resources take: 0.8.0-beta.9 is 0.8.0.9. */
export function windowsVersion(version) {
  const m = version.match(/^(\d+)\.(\d+)\.(\d+)(?:-\D*(\d+))?/)
  if (!m) throw new Error(`bad version ${version}`)
  return `${m[1]}.${m[2]}.${m[3]}.${m[4] ?? 0}`
}

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function plistValue(v, depth) {
  const pad = '\t'.repeat(depth)
  if (typeof v === 'boolean') return `${pad}<${v}/>`
  if (typeof v === 'string') return `${pad}<string>${esc(v)}</string>`
  if (Array.isArray(v)) {
    return `${pad}<array>\n${v.map((x) => plistValue(x, depth + 1)).join('\n')}\n${pad}</array>`
  }
  const body = Object.entries(v)
    .map(([k, x]) => `${pad}\t<key>${esc(k)}</key>\n${plistValue(x, depth + 1)}`)
    .join('\n')
  return `${pad}<dict>\n${body}\n${pad}</dict>`
}

/** The bundle's Info.plist. */
export function infoPlist(version) {
  const entries = {
    CFBundleDevelopmentRegion: 'en',
    CFBundleDisplayName: APP_NAME,
    CFBundleExecutable: APP_NAME,
    CFBundleIconFile: 'icon',
    CFBundleIdentifier: BUNDLE_ID,
    CFBundleInfoDictionaryVersion: '6.0',
    CFBundleName: APP_NAME,
    CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: version,
    CFBundleVersion: version,
    CFBundleURLTypes: [{ CFBundleURLName: BUNDLE_ID, CFBundleURLSchemes: [URL_SCHEME] }],
    LSApplicationCategoryType: 'public.app-category.developer-tools',
    LSMinimumSystemVersion: '12.0',
    NSHighResolutionCapable: true,
    NSPrincipalClass: 'NSApplication',
    NSSupportsAutomaticGraphicsSwitching: true
  }
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
${plistValue(entries, 0)}
</plist>
`
}

/** The Linux desktop entry that starts the app at `exec`. */
export function desktopEntry(exec) {
  return `[Desktop Entry]
Type=Application
Name=${APP_NAME}
Comment=Run and supervise coding agents
Exec=${exec} %U
Icon=vorn
Terminal=false
Categories=Development;
MimeType=x-scheme-handler/${URL_SCHEME};
StartupWMClass=vorn
`
}

/** The .deb's control file; `size` is the installed size in KiB. */
export function debControl(version, size) {
  return `Package: vorn
Version: ${version}
Architecture: amd64
Maintainer: Javier Canizalez <javier-canizalez@outlook.com>
Installed-Size: ${size}
Depends: libwebkit2gtk-4.1-0, libgtk-3-0
Section: devel
Priority: optional
Homepage: https://github.com/vorn-run/vorn
Description: Run and supervise coding agents
 Vorn runs coding agents in terminals beside your code and keeps their
 sessions alive across restarts.
`
}

// /usr/bin/vorn is a link to the bundled command line, made after the package
// unpacks and removed with it.
const DEB_POSTINST = `#!/bin/sh
set -e
ln -sf /opt/Vorn/resources/vornd/vorn /usr/bin/vorn
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
`

const DEB_POSTRM = `#!/bin/sh
set -e
if [ "$1" = remove ] || [ "$1" = purge ]; then
  if [ "$(readlink /usr/bin/vorn)" = /opt/Vorn/resources/vornd/vorn ]; then
    rm -f /usr/bin/vorn
  fi
fi
`

const APP_RUN = `#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
# VORN_CLI=1 runs the bundled command line instead of the app, and so does a
# vorn link to this image (ARGV0 is the name it was started by).
if [ -z "$VORN_CLI" ] && [ "\${ARGV0##*/}" = vorn ] && [ -L "$(command -v "$ARGV0")" ]; then VORN_CLI=1; fi
if [ -n "$VORN_CLI" ]; then exec "$here/resources/vornd/vorn" "$@"; fi
exec "$here/vorn" "$@"
`

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

/** Runs `fn` up to `times` times, `wait` seconds apart: Apple's services drop connections. */
function retry(times, wait, fn) {
  for (let attempt = 1; ; attempt++) {
    try {
      return fn()
    } catch (e) {
      if (attempt >= times) throw e
      console.log(`attempt ${attempt} of ${times} failed; retrying in ${wait}s`)
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, wait * 1000)
    }
  }
}

/**
 * The app's folder for this platform: the executable, and beside it
 * resources/ with vornd, its helpers, the web client and the notices.
 */
function layOut({ binDir, daemonDir, webDir }, resources, exe, platform) {
  const x = platform === 'win32' ? '.exe' : ''
  copyExe(path.join(binDir, `vorn-app${x}`), exe)
  const helpers = ['vornd', 'vorn-sessiond', 'vorn'].map((d) => d + x)
  if (platform === 'win32') helpers.push('conpty.dll', path.join('x64', 'OpenConsole.exe'))
  for (const d of helpers) copyExe(path.join(daemonDir, d), path.join(resources, 'vornd', d))
  if (!existsSync(path.join(webDir, 'index.html'))) {
    throw new Error(`${webDir} has no index.html; run packages/core/scripts/build-web.sh first`)
  }
  cpSync(webDir, path.join(resources, 'web', 'dist'), { recursive: true })
  for (const f of ['resources/idb.proto', 'THIRD_PARTY_NOTICES.md']) {
    copyFileSync(path.join(repo, f), path.join(resources, path.basename(f)))
  }
  return helpers.map((d) => path.join(resources, 'vornd', d))
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

/** Notarizes `file` (a zip or a dmg) and staples the ticket to `staple`. */
function notarize(file, staple) {
  const { APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID } = process.env
  const auth = [
    '--apple-id',
    APPLE_ID,
    '--password',
    APPLE_APP_SPECIFIC_PASSWORD,
    '--team-id',
    APPLE_TEAM_ID
  ]
  console.log(`$ xcrun notarytool submit ${path.basename(file)} … --wait`)
  const out = retry(3, 30, () =>
    execFileSync(
      'xcrun',
      [
        'notarytool',
        'submit',
        file,
        ...auth,
        '--wait',
        '--timeout',
        '30m',
        '--output-format',
        'json'
      ],
      { encoding: 'utf8' }
    )
  )
  const result = JSON.parse(out)
  if (result.status !== 'Accepted') {
    try {
      execFileSync('xcrun', ['notarytool', 'log', result.id, ...auth], { stdio: 'inherit' })
    } catch {}
    throw new Error(`notarization ${result.status ?? 'failed'}: ${result.message ?? ''}`)
  }
  retry(3, 15, () => run('xcrun', ['stapler', 'staple', staple]))
}

function packageMac(opts) {
  const { out, version, work } = opts
  const app = path.join(work, `${APP_NAME}.app`)
  const contents = path.join(app, 'Contents')
  const resources = path.join(contents, 'Resources')
  const helpers = layOut(opts, resources, path.join(contents, 'MacOS', APP_NAME), 'darwin')
  copyFileSync(path.join(repo, 'resources', 'icon.icns'), path.join(resources, 'icon.icns'))
  writeFileSync(path.join(contents, 'Info.plist'), infoPlist(version))
  writeFileSync(path.join(contents, 'PkgInfo'), 'APPL????')

  const identity = signingIdentity()
  const entitlements = path.join(repo, 'resources', 'entitlements.mac.plist')
  const sign = (file, withEntitlements = true) =>
    identity
      ? run('codesign', [
          '--force',
          '--timestamp',
          '--options',
          'runtime',
          ...(withEntitlements ? ['--entitlements', entitlements] : []),
          '--sign',
          identity,
          file
        ])
      : run('codesign', ['--force', '--sign', '-', file])
  // Inside out: the helpers, then the bundle, which seals them.
  for (const h of helpers) sign(h)
  sign(app)
  run('codesign', ['--verify', '--deep', '--strict', app])

  const canNotarize =
    identity &&
    ['APPLE_ID', 'APPLE_APP_SPECIFIC_PASSWORD', 'APPLE_TEAM_ID'].every((k) => process.env[k])
  const ditto = (to) =>
    run('ditto', ['-c', '-k', '--norsrc', '--noextattr', '--keepParent', app, to])
  if (canNotarize) {
    const submit = path.join(work, 'notarize.zip')
    ditto(submit)
    notarize(submit, app)
    rmSync(submit)
  }

  const names = assetNames(version, 'darwin', process.arch)
  const zip = path.join(out, names.zip)
  rmSync(zip, { force: true })
  ditto(zip)

  const stage = path.join(work, 'dmg')
  mkdirSync(stage, { recursive: true })
  run('ditto', [app, path.join(stage, `${APP_NAME}.app`)])
  symlinkSync('/Applications', path.join(stage, 'Applications'))
  const dmg = path.join(out, names.dmg)
  rmSync(dmg, { force: true })
  retry(3, 5, () =>
    run('hdiutil', [
      'create',
      '-volname',
      APP_NAME,
      '-srcfolder',
      stage,
      '-fs',
      'HFS+',
      '-format',
      'UDZO',
      '-ov',
      dmg
    ])
  )
  if (identity) sign(dmg, false)
  if (canNotarize) notarize(dmg, dmg)
  return {
    assets: [zip, dmg],
    signing: identity ? `Developer ID (${identity})` : 'ad hoc',
    notarized: Boolean(canNotarize)
  }
}

function findTool(name, candidates) {
  for (const c of candidates) if (c && existsSync(c)) return c
  try {
    execFileSync(name, ['/VERSION'], { stdio: 'ignore' })
    return name
  } catch {
    throw new Error(`${name} is not installed`)
  }
}

/** Signs Windows executables when a certificate is configured; false when there is none. */
function windowsSigner(work) {
  const { WIN_CERTIFICATE, WIN_CERTIFICATE_PASSWORD } = process.env
  if (!WIN_CERTIFICATE) return null
  const pfx = path.join(work, 'cert.pfx')
  writeFileSync(pfx, Buffer.from(WIN_CERTIFICATE, 'base64'))
  const kits = 'C:\\Program Files (x86)\\Windows Kits\\10\\bin'
  const signtool = existsSync(kits)
    ? readdirSync(kits)
        .filter((v) => /^10\./.test(v))
        .sort()
        .reverse()
        .map((v) => path.join(kits, v, 'x64', 'signtool.exe'))
        .find((p) => existsSync(p))
    : undefined
  return (file) =>
    retry(3, 10, () =>
      run(signtool ?? 'signtool', [
        'sign',
        '/f',
        pfx,
        ...(WIN_CERTIFICATE_PASSWORD ? ['/p', WIN_CERTIFICATE_PASSWORD] : []),
        '/fd',
        'sha256',
        '/tr',
        'http://timestamp.digicert.com',
        '/td',
        'sha256',
        file
      ])
    )
}

function packageWindows(opts) {
  const { out, version, work } = opts
  const dir = path.join(work, 'win-unpacked')
  const exe = path.join(dir, `${APP_NAME}.exe`)
  const helpers = layOut(opts, path.join(dir, 'resources'), exe, 'win32')
  const sign = windowsSigner(work)
  if (sign) for (const f of [exe, ...helpers].filter((f) => f.endsWith('.exe'))) sign(f)

  const setup = path.join(out, assetNames(version, 'win32', 'x64').setup)
  rmSync(setup, { force: true })
  const makensis = findTool('makensis', [
    process.env.MAKENSIS,
    'C:\\Program Files (x86)\\NSIS\\makensis.exe',
    'C:\\Program Files\\NSIS\\makensis.exe'
  ])
  run(makensis, [
    '/V2',
    `/DVERSION=${version}`,
    `/DNUMERIC_VERSION=${windowsVersion(version)}`,
    `/DSOURCE=${dir}`,
    `/DOUTFILE=${setup}`,
    `/DICON=${path.join(repo, 'resources', 'icon.ico')}`,
    `/DGUID=${WINDOWS_GUID}`,
    path.join(core, 'scripts', 'vorn.nsi')
  ])
  if (sign) sign(setup)
  return { assets: [setup], signing: sign ? 'Authenticode' : 'unsigned', notarized: false }
}

/** The installed size of a folder, in KiB, as dpkg counts it. */
function sizeKiB(dir) {
  let total = 0
  for (const e of readdirSync(dir, { withFileTypes: true, recursive: true })) {
    if (e.isFile()) total += statSync(path.join(e.parentPath ?? e.path, e.name)).size
  }
  return Math.ceil(total / 1024)
}

function packageLinux(opts) {
  const { out, version, work } = opts
  const names = assetNames(version, 'linux', 'x64')
  const icon = path.join(repo, 'resources', 'icon.png')

  const root = path.join(work, 'deb')
  const opt = path.join(root, 'opt', APP_NAME)
  layOut(opts, path.join(opt, 'resources'), path.join(opt, 'vorn'), 'linux')
  const share = path.join(root, 'usr', 'share')
  mkdirSync(path.join(share, 'applications'), { recursive: true })
  writeFileSync(
    path.join(share, 'applications', 'vorn.desktop'),
    desktopEntry(`/opt/${APP_NAME}/vorn`)
  )
  // 1024 pixels, which no hicolor size takes; pixmaps takes any.
  mkdirSync(path.join(share, 'pixmaps'), { recursive: true })
  copyFileSync(icon, path.join(share, 'pixmaps', 'vorn.png'))
  const debian = path.join(root, 'DEBIAN')
  mkdirSync(debian, { recursive: true })
  writeFileSync(path.join(debian, 'control'), debControl(version, sizeKiB(path.join(root, 'opt'))))
  for (const [name, body] of [
    ['postinst', DEB_POSTINST],
    ['postrm', DEB_POSTRM]
  ]) {
    writeFileSync(path.join(debian, name), body)
    chmodSync(path.join(debian, name), 0o755)
  }
  const deb = path.join(out, names.deb)
  rmSync(deb, { force: true })
  run('dpkg-deb', ['--root-owner-group', '-Zxz', '--build', root, deb])

  const appDir = path.join(work, `${APP_NAME}.AppDir`)
  cpSync(opt, appDir, { recursive: true })
  writeFileSync(path.join(appDir, 'AppRun'), APP_RUN)
  chmodSync(path.join(appDir, 'AppRun'), 0o755)
  writeFileSync(path.join(appDir, 'vorn.desktop'), desktopEntry('vorn'))
  copyFileSync(icon, path.join(appDir, 'vorn.png'))
  symlinkSync('vorn.png', path.join(appDir, '.DirIcon'))
  const image = path.join(out, names.appimage)
  rmSync(image, { force: true })
  run(process.env.APPIMAGETOOL || 'appimagetool', [appDir, image], {
    env: { ...process.env, ARCH: 'x86_64', APPIMAGE_EXTRACT_AND_RUN: '1' }
  })
  return { assets: [image, deb], signing: 'unsigned', notarized: false }
}

/** The server alone: bin/ with vornd, vorn-sessiond and vorn, and web/dist, which vorn finds above bin/. */
function packageServer({ daemonDir, webDir, out, version, work }) {
  if (process.platform === 'win32') throw new Error('no server archive for Windows')
  const name = serverArchiveName(version, process.platform, os.arch())
  const top = name.replace(/\.tar\.gz$/, '')
  const root = path.join(work, top)
  for (const d of ['vornd', 'vorn-sessiond', 'vorn']) {
    copyExe(path.join(daemonDir, d), path.join(root, 'bin', d))
  }
  if (!existsSync(path.join(webDir, 'index.html'))) {
    throw new Error(`${webDir} has no index.html; run yarn build:web first`)
  }
  cpSync(webDir, path.join(root, 'web', 'dist'), { recursive: true })
  for (const f of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    copyFileSync(path.join(repo, f), path.join(root, f))
  }
  const archive = path.join(out, name)
  rmSync(archive, { force: true })
  run('tar', ['-czf', archive, '-C', work, top])
  return { assets: [archive], signing: 'unsigned', notarized: false }
}

function main() {
  const args = parseArgs(process.argv.slice(2))
  const binDir = path.resolve(args['bin-dir'] ?? path.join(core, 'target', 'release'))
  const out = path.resolve(args.out ?? path.join(repo, 'dist-native'))
  mkdirSync(out, { recursive: true })
  const opts = {
    binDir,
    daemonDir: path.resolve(args['daemon-dir'] ?? binDir),
    webDir: path.resolve(
      args['web-dir'] ?? path.join(repo, 'packages', 'core', 'crates', 'vorn-web', 'dist')
    ),
    out,
    version:
      args.version ?? JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8')).version,
    work: mkdtempSync(path.join(os.tmpdir(), 'vorn-package-'))
  }
  const kind = args.package ?? 'app'
  if (kind !== 'app' && kind !== 'server') throw new Error(`--package ${kind}: app or server`)
  const pack =
    kind === 'server'
      ? packageServer
      : { darwin: packageMac, win32: packageWindows, linux: packageLinux }[process.platform]
  if (!pack) throw new Error(`no package for ${process.platform}`)
  try {
    const { assets, signing, notarized } = pack(opts)
    for (const asset of assets) {
      const line = `${path.basename(asset)}: signed ${signing}, notarized ${notarized ? 'yes' : 'no'}`
      console.log(line)
      if (process.env.GITHUB_STEP_SUMMARY)
        writeFileSync(process.env.GITHUB_STEP_SUMMARY, `- ${line}\n`, { flag: 'a' })
    }
  } finally {
    rmSync(opts.work, { recursive: true, force: true })
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (e) {
    console.error(`package-native: ${e.message}`)
    process.exit(1)
  }
}
