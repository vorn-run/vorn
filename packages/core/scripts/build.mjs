#!/usr/bin/env node
// Builds the crate and copies the cdylib to ./vorn_core.node, the one name the
// server and electron-builder look for on every platform.
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

const built = spawnSync('cargo', cargoArgs, { cwd: root, stdio: 'inherit' })
if (built.error) {
  console.error(`could not run cargo: ${built.error.message}`)
  process.exit(1)
}
if (built.status !== 0) process.exit(built.status ?? 1)

const library =
  {
    darwin: 'libvorn_core.dylib',
    win32: 'vorn_core.dll'
  }[process.platform] ?? 'libvorn_core.so'

const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(process.env.CARGO_TARGET_DIR)
  : path.join(root, 'target')
const from = path.join(targetDir, profile, library)
if (!existsSync(from)) {
  console.error(`cargo succeeded but ${from} is missing`)
  process.exit(1)
}
const to = path.join(root, 'vorn_core.node')
copyFileSync(from, to)
console.log(`built ${path.relative(process.cwd(), to)}`)
