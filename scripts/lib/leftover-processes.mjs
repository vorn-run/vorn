// Finding the vornd, vorn-sessiond, vorn, recovery and Tailscale processes a
// test run left behind. Shared by the CI check (check-leftover-processes.mjs)
// and the vitest teardown, so both look at processes the same way.

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

export const NAMES = new Set([
  'vornd',
  'vorn-sessiond',
  'vorn',
  'recovery-emit',
  'recovery-subject',
  'tailscale'
])

/** Each path as given and resolved, so /tmp and /private/tmp both match. */
export function spellings(roots) {
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
export function processes() {
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
export const fold =
  process.platform === 'win32' ? (s) => s.replaceAll('\\', '/').toLowerCase() : (s) => s

/** The test daemons among the processes, whose command line or environment satisfies `named`. */
export function leftoversMatching(named) {
  return processes().filter(
    (p) =>
      p.pid !== process.pid &&
      NAMES.has(
        path
          .basename(p.argv0.replaceAll('\\', '/'))
          .replace(/\.exe$/i, '')
          .toLowerCase()
      ) &&
      named(fold(p.text))
  )
}

/** The test daemons whose command line or environment names one of `roots` (see {@link spellings}). */
export function leftovers(roots) {
  return leftoversMatching((text) => roots.some((r) => text.includes(r)))
}

/** Waits up to `seconds` for processes told to end with the test run to notice it ended. */
export async function leftAfter(find, seconds) {
  const deadline = Date.now() + seconds * 1000
  let left = find()
  while (left.length > 0 && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 500))
    left = find()
  }
  return left
}

/** Ends each of `left` and the programs it holds: SIGTERM first, which a test-tied holder takes as its end, then SIGKILL. */
export async function endAll(left) {
  for (const sig of ['SIGTERM', 'SIGKILL']) {
    for (const p of left) {
      try {
        process.kill(p.pid, sig)
      } catch {
        // Already gone.
      }
    }
    if (sig === 'SIGTERM') await new Promise((r) => setTimeout(r, 1000))
  }
}
