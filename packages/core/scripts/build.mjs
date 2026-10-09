#!/usr/bin/env node
// Builds the crate and copies the cdylib to ./vorn_core.node, the one name the
// server and electron-builder look for on every platform. Then builds vornd and
// vorn-sessiond and the vorn command and copies them to ./vornd,
// ./vorn-sessiond and ./vorn (with .exe on Windows), where the app looks for
// them and electron-builder picks them up. On Windows it also fetches the
// ConPTY vorn-sessiond ships with to beside it (scripts/fetch-conpty.mjs).
//
//   --debug        unoptimized build
//   --profile=NAME build with another cargo profile, such as ci (release
//                  without LTO), for a check that needs no shipping binary
//   --no-ghostty   build vornd without its session engine and libghostty-vt,
//                  for a machine without Zig 0.15.2; vorn_core.node and
//                  vorn-sessiond never link Ghostty
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { fetchConpty } from './fetch-conpty.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = new Set(process.argv.slice(2))
const named = [...args].find((a) => a.startsWith('--profile='))?.slice('--profile='.length)
const profile = args.has('--debug') ? 'debug' : (named ?? 'release')

// Cargo builds debug by default and spells release as a flag of its own.
const profileArgs =
  profile === 'debug' ? [] : profile === 'release' ? ['--release'] : ['--profile', profile]
const cargoArgs = ['build', '--locked', ...profileArgs]

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

// Their own build. vornd's session engine parses with libghostty-vt, so
// --no-ghostty builds vornd without it; vorn-sessiond never links Ghostty.
// vornd starts the vorn-sessiond shipped beside it.
const exe = process.platform === 'win32' ? '.exe' : ''
const daemonArgs = [
  'build',
  '--locked',
  ...profileArgs,
  '-p',
  'vornd',
  '-p',
  'vorn-sessiond',
  '-p',
  'vorn-cli'
]
if (args.has('--no-ghostty')) daemonArgs.push('--no-default-features')
cargo(daemonArgs)
for (const daemon of ['vornd', 'vorn-sessiond', 'vorn']) copyOut(daemon + exe, daemon + exe)

// vorn-sessiond hosts Windows sessions in the ConPTY shipped beside it.
if (process.platform === 'win32') {
  try {
    await fetchConpty()
  } catch (e) {
    console.error(`could not fetch ConPTY: ${e.message}`)
    process.exit(1)
  }
}
