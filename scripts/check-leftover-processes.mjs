#!/usr/bin/env node
// Fails when a test run left a process behind: a vornd, vorn-sessiond, vorn,
// recovery program or Tailscale CLI whose command line or environment names
// one of the given paths (a test's temporary root, the core's target dir).
// Processes that name none of them, such as a running app's, are not looked at.
//
// usage: node scripts/check-leftover-processes.mjs [--wait SECONDS] [--kill] PATH...

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const NAMES = new Set([
  'vornd',
  'vorn-sessiond',
  'vorn',
  'recovery-emit',
  'recovery-subject',
  'tailscale'
])

function parseArgs(argv) {
  const opts = { wait: 10, kill: false, roots: [] }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--wait') opts.wait = Number(argv[++i])
    else if (a === '--kill') opts.kill = true
    else opts.roots.push(a)
  }
  if (opts.roots.length === 0 || !Number.isFinite(opts.wait)) {
    console.error('usage: check-leftover-processes.mjs [--wait SECONDS] [--kill] PATH...')
    process.exit(2)
  }
  return opts
}

/** Each path as given and resolved, so /tmp and /private/tmp both match. */
function spellings(roots) {
  const out = new Set()
  for (const r of roots) {
    const abs = path.resolve(r)
    out.add(abs)
    try {
      out.add(fs.realpathSync(abs))
    } catch {
      // A prefix such as /tmp/vorn names no directory of its own.
    }
  }
  return [...out].filter((r) => r.length > 1)
}

/** Every process: its pid, program name, command line, and the text (arguments and environment) to search. */
function processes() {
  if (process.platform === 'linux') {
    const out = []
    for (const entry of fs.readdirSync('/proc')) {
      if (!/^\d+$/.test(entry)) continue
      try {
        const argv = fs.readFileSync(`/proc/${entry}/cmdline`, 'utf8').split('\0')
        let env = ''
        try {
          env = fs.readFileSync(`/proc/${entry}/environ`, 'utf8').replaceAll('\0', ' ')
        } catch {
          // Another user's process.
        }
        const label = argv.join(' ').trim()
        out.push({ pid: Number(entry), argv0: argv[0] ?? '', label, text: `${label} ${env}` })
      } catch {
        // Gone while listing.
      }
    }
    return out
  }
  if (process.platform === 'win32') {
    const json = execFileSync(
      'powershell',
      [
        '-NoProfile',
        '-Command',
        'Get-CimInstance Win32_Process | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress'
      ],
      { encoding: 'utf8', maxBuffer: 64 << 20 }
    )
    return [JSON.parse(json)].flat().map((p) => ({
      pid: p.ProcessId,
      argv0: p.ExecutablePath ?? '',
      label: p.CommandLine ?? p.ExecutablePath ?? '',
      text: `${p.ExecutablePath ?? ''} ${p.CommandLine ?? ''}`
    }))
  }
  // -E appends each process's environment to its command, for this user's processes.
  const ps = (flags) =>
    execFileSync('ps', [...flags, '-o', 'pid=,command='], { encoding: 'utf8', maxBuffer: 64 << 20 })
      .split('\n')
      .map((line) => line.trim().match(/^(\d+)\s+(\S+)(.*)$/))
      .filter(Boolean)
  const labels = new Map(
    ps(['-axww']).map(([, pid, argv0, rest]) => [Number(pid), `${argv0}${rest}`])
  )
  return ps(['-axww', '-E']).map(([, pid, argv0, rest]) => ({
    pid: Number(pid),
    argv0,
    label: labels.get(Number(pid)) ?? argv0,
    text: `${argv0}${rest}`
  }))
}

// Windows paths match whatever their separators and case.
const fold = process.platform === 'win32' ? (s) => s.replaceAll('\\', '/').toLowerCase() : (s) => s

function leftovers(roots) {
  return processes().filter(
    (p) =>
      p.pid !== process.pid &&
      NAMES.has(
        path
          .basename(p.argv0.replaceAll('\\', '/'))
          .replace(/\.exe$/i, '')
          .toLowerCase()
      ) &&
      roots.some((r) => fold(p.text).includes(r))
  )
}

const opts = parseArgs(process.argv.slice(2))
const roots = spellings(opts.roots).map(fold)
// Processes told to end with the test run take a moment to notice it ended.
const deadline = Date.now() + opts.wait * 1000
let left = leftovers(roots)
while (left.length > 0 && Date.now() < deadline) {
  await new Promise((r) => setTimeout(r, 500))
  left = leftovers(roots)
}
if (left.length === 0) {
  console.log(`No processes left under ${roots.join(', ')}`)
  process.exit(0)
}
console.error(`${left.length} processes outlived the tests:`)
for (const p of left) console.error(`  ${p.pid} ${p.label}`)
if (opts.kill) {
  for (const p of left) {
    try {
      process.kill(p.pid, 'SIGKILL')
    } catch {
      // Already gone.
    }
  }
}
process.exit(1)
