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
  /** The app binary, which doubles as this bundle's Node. */
  exe: string
  /** Where `server/cli.cjs` and the unpacked native modules sit. */
  resources: string
  /** The AppImage this app was started from, when it was. */
  appImage?: string
}

/**
 * The script itself.
 *
 * Pure, and told which platform it is writing for, so a test can read all three
 * shapes rather than only the one it happens to run on. An AppImage cannot be
 * referenced from outside its own mount, which is why that shape resolves its
 * entry point from `APPDIR` on the inside.
 */
export function shimScript(paths: ShimPaths, platform: NodeJS.Platform = process.platform): string {
  if (platform === 'win32') {
    return [
      '@echo off',
      'setlocal',
      'if "%~1"=="" (',
      `  start "" "${paths.exe}"`,
      '  exit /b',
      ')',
      'set "ELECTRON_RUN_AS_NODE=1"',
      `set "VORN_NATIVE_MODULES_PATH=${path.join(paths.resources, 'app.asar.unpacked', 'node_modules')}"`,
      `set "NODE_PATH=${path.join(paths.resources, 'app.asar', 'node_modules')};%VORN_NATIVE_MODULES_PATH%"`,
      `"${paths.exe}" "${path.join(paths.resources, 'server', 'cli.cjs')}" %*`,
      ''
    ].join('\r\n')
  }

  if (paths.appImage) {
    return `#!/bin/sh
APPIMAGE="${paths.appImage}"

if [ "$#" -eq 0 ]; then
  exec "$APPIMAGE"
fi

BOOTSTRAP='var entry = process.env.APPDIR + "/resources/server/cli.cjs"; process.argv.splice(1, 0, entry); require(entry)'

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
APP_EXE="${paths.exe}"
RESOURCES="${paths.resources}"

# No command: open the app, which is what typing \`vorn\` should mean.
if [ "$#" -eq 0 ]; then
  ${launch}
fi

ELECTRON_RUN_AS_NODE=1
VORN_NATIVE_MODULES_PATH="\${RESOURCES}/app.asar.unpacked/node_modules"
NODE_PATH="\${RESOURCES}/app.asar/node_modules:\${VORN_NATIVE_MODULES_PATH}"
export ELECTRON_RUN_AS_NODE VORN_NATIVE_MODULES_PATH NODE_PATH

exec "$APP_EXE" "\${RESOURCES}/server/cli.cjs" "$@"
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

export function installCliShim(): { ok: true; path: string } | { ok: false; error: string } {
  if (!app.isPackaged) {
    return { ok: false, error: 'Only a packaged Vorn can install the command.' }
  }

  const target = shimPath()
  try {
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(
      target,
      shimScript({
        exe: app.getPath('exe'),
        resources: process.resourcesPath,
        appImage: process.env.APPIMAGE
      }),
      'utf-8'
    )
    if (process.platform !== 'win32') fs.chmodSync(target, 0o755)
    return { ok: true, path: target }
  } catch (err) {
    log.warn({ err, target }, '[cli] could not install the vorn command')
    return { ok: false, error: err instanceof Error ? err.message : String(err) }
  }
}
