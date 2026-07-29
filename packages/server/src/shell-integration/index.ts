import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { getDefaultShell, getSafeEnv } from '../process-utils'
import log from '../logger'
import { detectShellFamily } from './protocol'
import { resetShimCache } from './shim'
import { zshSetup } from './zsh'
import { bashSetup } from './bash'
import { fishSetup } from './fish'
import { powershellSetup } from './powershell'
import { cmdSetup } from './cmd'
import type { ShellSetup } from './types'

/**
 * Shell integration: command-boundary markers for the terminal.
 *
 * See ./protocol.ts for the wire format and what each shell can report. This
 * module only picks the right one and hands back what the pty needs to launch.
 */

export { detectShellFamily, CAPABILITIES } from './protocol'
export type { ShellFamily, ShellCapabilities } from './protocol'
export type { ShellSetup } from './types'

const NONE: ShellSetup = { env: {}, args: null }

/**
 * How to launch a local pty with shell integration enabled.
 *
 * Returns empty env and null args for a shell we have no integration for, in
 * which case the session runs exactly as it would have.
 */
export function getShellIntegration(
  opts: { minimalPrompt?: boolean; shell?: string } = {}
): ShellSetup {
  // Must be the shell actually being spawned: integrating with one shell while
  // launching another writes a shim nothing reads, or launch arguments the
  // real shell rejects.
  const shell = opts.shell ?? getDefaultShell()
  const family = detectShellFamily(shell)
  if (!family) return NONE

  // Default on: the terminal is drawn as command blocks, and the shell's own
  // prompt fights that. The setting exists for anyone who wants theirs back.
  const options = {
    minimalPrompt: opts.minimalPrompt !== false,
    userZdotdir: getSafeEnv().ZDOTDIR ?? os.homedir()
  }

  try {
    switch (family) {
      case 'zsh':
        return zshSetup(options)
      case 'bash':
        return bashSetup(options)
      case 'fish':
        return fishSetup(options, getSafeEnv().XDG_DATA_DIRS ?? '')
      case 'powershell':
        return powershellSetup(options)
      case 'cmd':
        return cmdSetup(options, getSafeEnv().PROMPT ?? '')
    }
  } catch (err) {
    log.warn(`[shell-integration] ${family} setup failed, running without: ${String(err)}`)
    return NONE
  }
}

/**
 * Env-only view, for callers that cannot change launch arguments.
 * bash and PowerShell need arguments, so this is not enough for them.
 */
export function getShellIntegrationEnv(
  opts: { minimalPrompt?: boolean; shell?: string } = {}
): Record<string, string> {
  return getShellIntegration(opts).env
}

/** Test hook: forget the cached shim state. */
export function resetShellIntegrationCache(): void {
  resetShimCache()
  executableCache = null
}

// --- Executable listing (intent bar command completion) ---

let executableCache: { names: string[]; at: number } | null = null
const EXECUTABLE_CACHE_MS = 60_000

/**
 * Names of every file on PATH, deduplicated and sorted. Used by the intent
 * bar to complete the command token. The scan result is cached: PATH contents
 * change rarely, and readdir over a dozen bin dirs is not free.
 */
export function listShellExecutables(): string[] {
  if (executableCache && Date.now() - executableCache.at < EXECUTABLE_CACHE_MS) {
    return executableCache.names
  }
  const pathVar = getSafeEnv().PATH ?? process.env.PATH ?? ''
  const names = new Set<string>()
  for (const dir of pathVar.split(path.delimiter)) {
    if (!dir) continue
    try {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) continue
        names.add(entry.name)
      }
    } catch {
      // unreadable PATH entry — skip
    }
  }
  const sorted = [...names].sort()
  executableCache = { names: sorted, at: Date.now() }
  return sorted
}
