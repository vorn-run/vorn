#!/usr/bin/env node
// Builds the crate and copies the cdylib to ./vorn_core.node, the one name the
// server and electron-builder look for on every platform. Then builds vornd and
// copies it to ./vornd (./vornd.exe on Windows), where the app looks for it.
//
//   --debug        unoptimized build
//   --no-ghostty   skip libghostty-vt, for a machine without Zig 0.15.2
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = new Set(process.argv.slice(2))
const profile = args.has('--debug') ? 'debug' : 'release'

const cargoArgs = ['build', '--locked']
if (profile === 'release') cargoArgs.push('--release')
if (args.has('--no-ghostty')) cargoArgs.push('--no-default-features')

function cargo(argv) {
  const built = spawnSync('cargo', argv, { cwd: root, stdio: 'inherit' })
  if (built.error) {
    console.error(`could not run cargo: ${built.error.message}`)
    process.exit(1)
  }
  if (built.status !== 0) process.exit(built.status ?? 1)
}

cargo(cargoArgs)

const library =
  {
    darwin: 'libvorn_core.dylib',
    win32: 'vorn_core.dll'
  }[process.platform] ?? 'libvorn_core.so'

// Relative to the crate, as cargo (which runs there) resolves it.
const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(root, process.env.CARGO_TARGET_DIR)
  : path.join(root, 'target')

function copyOut(name, as) {
  const from = path.join(targetDir, profile, name)
  if (!existsSync(from)) {
    console.error(`cargo succeeded but ${from} is missing`)
    process.exit(1)
  }
  const to = path.join(root, as)
  copyFileSync(from, to)
  console.log(`built ${path.relative(process.cwd(), to)}`)
}

copyOut(library, 'vorn_core.node')

// Its own build: vornd has no ghostty feature for --no-ghostty to turn off.
const daemon = process.platform === 'win32' ? 'vornd.exe' : 'vornd'
const daemonArgs = ['build', '--locked', '-p', 'vornd']
if (profile === 'release') daemonArgs.push('--release')
cargo(daemonArgs)
copyOut(daemon, daemon)
