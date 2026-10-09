import { app } from 'electron'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import log from './logger'

/**
 * The `vorn` command, installed by the app rather than by `install.sh`.
 *
 * Somebody who dragged Vorn out of the DMG, or installed the cask, never ran
 * that script — and the command it writes is the only way the CLI reaches PATH.
 * The shim written here is the same one, generated from the paths of the app
 * that is actually running, so it points at wherever this copy lives.
 */

function isWritable(dir: string): boolean {
  try {
    fs.accessSync(dir, fs.constants.W_OK)
    return true
  } catch {
    return false
  }
}

/**
 * Whether a shell would find a command in this directory.
 *
 * Case-folded on Windows, where `%PATH%` names the same directory in whatever
 * case it was written in and a literal comparison would call it absent.
 */
export function onPath(dir: string, platform: NodeJS.Platform = process.platform): boolean {
  // The platform's own rules, taken explicitly: separator, delimiter and case.
  const rules = platform === 'win32' ? path.win32 : path.posix
  const fold = (value: string): string =>
    platform === 'win32' ? rules.resolve(value).toLowerCase() : rules.resolve(value)

  const wanted = fold(dir)
  return (process.env.PATH ?? '')
    .split(rules.delimiter)
    .some((entry) => entry !== '' && fold(entry) === wanted)
}

/**
 * Where the command can go.
 *
 * A writable directory the shell already searches, first -- a command installed
 * anywhere else is a file, not a command. `~/.local/bin` is the fallback even
 * when it is neither: it is the one place this process may always create, and
 * the caller says so rather than claiming the command is ready to run.
 */
export function shimDirectory(): string {
  if (process.platform === 'win32') return path.dirname(app.getPath('exe'))

  const userBin = path.join(os.homedir(), '.local', 'bin')
  const candidates = ['/usr/local/bin', userBin].filter(isWritable)
  return candidates.find((dir) => onPath(dir)) ?? candidates[0] ?? userBin
}

export function shimPath(): string {
  return path.join(shimDirectory(), process.platform === 'win32' ? 'vorn.cmd' : 'vorn')
}

export interface ShimPaths {
  /** The app binary, which opens the app and, in an AppImage, reaches inside it. */
  exe: string
  /** Where `vornd/vorn`, the command itself, sits. */
  resources: string
  /** The AppImage this app was started from, when it was. */
  appImage?: string
}

/**
 * The script itself.
 *
 * Pure, and told which platform it is writing for, so a test can read all three
 * shapes rather than only the one it happens to run on. An AppImage cannot be
 * referenced from outside its own mount, which is why that shape resolves the
 * command from `APPDIR` on the inside.
 */
export function shimScript(paths: ShimPaths, platform: NodeJS.Platform = process.platform): string {
  if (platform === 'win32') {
    // `path.win32`, not `path` -- this script may be written from a machine that
    // is not Windows, and a batch file with forward slashes in it is a bug.
    return [
      '@echo off',
      'setlocal',
      'if "%~1"=="" (',
      `  start "" "${paths.exe}"`,
      '  exit /b',
      ')',
      `"${path.win32.join(paths.resources, 'vornd', 'vorn.exe')}" %*`,
      ''
    ].join('\r\n')
  }

  if (paths.appImage) {
    return `#!/bin/sh
APPIMAGE="${paths.appImage}"

if [ "$#" -eq 0 ]; then
  exec "$APPIMAGE"
fi

BOOTSTRAP='var r = require("child_process").spawnSync(process.env.APPDIR + "/resources/vornd/vorn", process.argv.slice(1), { stdio: "inherit" }); process.exit(r.status === null ? 1 : r.status)'

ELECTRON_RUN_AS_NODE=1 exec "$APPIMAGE" -e "$BOOTSTRAP" "$@"
`
  }

  // On macOS the app is opened by its bundle, not by the binary inside it, so
  // it arrives as a normal launch rather than a child of this shell.
  const launch =
    platform === 'darwin'
      ? `exec open -a "${paths.exe.replace(/\/Contents\/MacOS\/.*$/, '')}"`
      : `exec "${paths.exe}"`

  return `#!/bin/sh
# No command: open the app, which is what typing \`vorn\` should mean.
if [ "$#" -eq 0 ]; then
  ${launch}
fi

exec "${paths.resources}/vornd/vorn" "$@"
`
}

export interface ShimStatus {
  /** False while running from source: the paths would point into a dev tree. */
  available: boolean
  installed: boolean
  path: string
  /** Whether a shell would find it there, which decides what the UI can promise. */
  onPath: boolean
}

export function cliShimStatus(): ShimStatus {
  const target = shimPath()
  return {
    available: app.isPackaged,
    installed: fs.existsSync(target),
    path: target,
    onPath: onPath(path.dirname(target))
  }
}

/** Every place this app or the install scripts may have put the command. */
function installedShims(): string[] {
  if (process.platform === 'win32') return [path.join(path.dirname(app.getPath('exe')), 'vorn.cmd')]
  return ['/usr/local/bin', path.join(os.homedir(), '.local', 'bin')].map((dir) =>
    path.join(dir, 'vorn')
  )
}

function currentShim(): string {
  return shimScript({
    exe: app.getPath('exe'),
    resources: process.resourcesPath,
    appImage: process.env.APPIMAGE
  })
}

/** Rewrites a command an older build installed, which ran the command line this build no longer ships. */
export function refreshStaleShims(): string[] {
  if (!app.isPackaged) return []
  const rewritten: string[] = []
  for (const file of installedShims()) {
    try {
      if (!fs.readFileSync(file, 'utf-8').includes('cli.cjs')) continue
      fs.writeFileSync(file, currentShim(), 'utf-8')
      rewritten.push(file)
    } catch {
      // Not there, or not this app's to rewrite.
    }
  }
  return rewritten
}

export function installCliShim(): { ok: true; path: string } | { ok: false; error: string } {
  if (!app.isPackaged) {
    return { ok: false, error: 'Only a packaged Vorn can install the command.' }
  }

  const target = shimPath()
  try {
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, currentShim(), 'utf-8')
    if (process.platform !== 'win32') fs.chmodSync(target, 0o755)
    return { ok: true, path: target }
  } catch (err) {
    log.warn({ err, target }, '[cli] could not install the vorn command')
    return { ok: false, error: err instanceof Error ? err.message : String(err) }
  }
}
