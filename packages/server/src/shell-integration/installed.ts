import fs from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { CAPABILITIES, detectShellFamily, type ShellFamily } from './protocol'
import type { InstalledShell } from '@vornrun/shared/types'

export type { InstalledShell }

/**
 * Which shells are actually on this machine.
 *
 * The point is to let someone choose knowingly: the shells differ in what they
 * can report, and on Windows that difference is the gap between blocks with
 * exit status and blocks without. So each entry carries its capabilities, not
 * just a path.
 */

const LABELS: Record<ShellFamily, string> = {
  zsh: 'zsh',
  bash: 'bash',
  fish: 'fish',
  powershell: 'PowerShell',
  cmd: 'Command Prompt'
}

/** Ordered best-first, so the list reads as a recommendation. */
// PowerShell is cross-platform and worth listing wherever it is installed.
const POSIX_ORDER: ShellFamily[] = ['zsh', 'fish', 'bash', 'powershell']
const WINDOWS_ORDER: ShellFamily[] = ['powershell', 'bash', 'cmd']

const EXECUTABLES: Record<ShellFamily, string[]> = {
  zsh: ['zsh'],
  bash: ['bash'],
  fish: ['fish'],
  // pwsh is PowerShell 7; powershell.exe is the 5.1 that ships with Windows.
  powershell: ['pwsh', 'powershell'],
  cmd: ['cmd']
}

function pathDirs(): string[] {
  return (process.env.PATH ?? '').split(path.delimiter).filter(Boolean)
}

function findOnPath(name: string): string[] {
  const suffixes = process.platform === 'win32' ? ['.exe'] : ['']
  const found: string[] = []
  for (const dir of pathDirs()) {
    for (const suffix of suffixes) {
      const candidate = path.join(dir, name + suffix)
      try {
        if (fs.statSync(candidate).isFile()) found.push(candidate)
      } catch {
        // not there, or unreadable
      }
    }
  }
  return found
}

/** Locations a shell can exist without being on PATH. */
function wellKnown(family: ShellFamily): string[] {
  if (process.platform === 'win32') {
    const systemRoot = process.env.SystemRoot || process.env.windir || 'C:\\Windows'
    if (family === 'powershell') {
      return [path.join(systemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe')]
    }
    if (family === 'cmd') return [path.join(systemRoot, 'System32', 'cmd.exe')]
    // Git for Windows ships bash outside PATH more often than not.
    if (family === 'bash') {
      return [
        'C:\\Program Files\\Git\\bin\\bash.exe',
        'C:\\Program Files (x86)\\Git\\bin\\bash.exe'
      ]
    }
    return []
  }
  if (family === 'zsh') return ['/bin/zsh']
  if (family === 'bash') return ['/bin/bash']
  return []
}

/**
 * Best-effort version string. Bounded and never fatal — a shell that will not
 * answer is still perfectly usable, it just goes unlabelled.
 */
function readVersion(family: ShellFamily, shellPath: string): string | null {
  if (family === 'cmd') return null
  try {
    const out = execFileSync(shellPath, ['--version'], {
      encoding: 'utf-8',
      // A cold PowerShell start loads the .NET runtime first and can take
      // seconds the first time; the others answer immediately.
      timeout: family === 'powershell' ? 6000 : 2000,
      stdio: ['ignore', 'pipe', 'ignore']
    })
    // "zsh 5.9 (arm64-apple-darwin)", "GNU bash, version 3.2.57(1)-release",
    // "fish, version 4.8.1", "PowerShell 7.6.4"
    const match = /(\d+\.\d+(?:\.\d+)?)/.exec(out)
    return match ? match[1] : null
  } catch {
    return null
  }
}

function describe(family: ShellFamily): InstalledShell['blocks'] {
  const caps = CAPABILITIES[family]
  if (caps.executionStart && caps.exitCode && caps.commandText) {
    return { level: 'full', limitation: null }
  }
  if (!caps.exitCode && !caps.commandText) {
    return { level: 'limited', limitation: 'No exit status or command name on blocks' }
  }
  return { level: 'partial', limitation: 'Blocks appear once each command finishes' }
}

let cache: InstalledShell[] | null = null

export function listInstalledShells(): InstalledShell[] {
  if (cache) return cache

  const order = process.platform === 'win32' ? WINDOWS_ORDER : POSIX_ORDER
  const seen = new Set<string>()
  const shells: InstalledShell[] = []

  for (const family of order) {
    const candidates = [
      ...EXECUTABLES[family].flatMap(findOnPath),
      ...wellKnown(family).filter((p) => {
        try {
          return fs.statSync(p).isFile()
        } catch {
          return false
        }
      })
    ]
    for (const candidate of candidates) {
      const key = candidate.toLowerCase()
      if (seen.has(key)) continue
      seen.add(key)
      // Guard against a name that resolved to something else entirely, e.g. a
      // wrapper script called "bash" that is not bash.
      if (detectShellFamily(candidate) !== family) continue
      const version = readVersion(family, candidate)
      shells.push({
        family,
        name: version && family === 'powershell' ? `PowerShell ${version[0]}` : LABELS[family],
        path: candidate,
        version,
        blocks: describe(family)
      })
    }
  }

  cache = shells
  return shells
}

export function resetInstalledShellCache(): void {
  cache = null
}
