import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

import { HOOK_OWNER_FILE, mayReleaseHooks, parseHookOwner } from './hook-ownership'

const CLAUDE_SETTINGS_PATH = path.join(os.homedir(), '.claude', 'settings.json')
const VORN_HEADER = 'X-Vorn'

const HOOK_EVENTS = [
  'PreToolUse',
  'PostToolUse',
  'PostToolUseFailure',
  'Stop',
  'Notification',
  'PermissionRequest',
  'SessionStart',
  'SessionEnd'
]

function makeHookEntry(port: number, token: string) {
  return {
    type: 'http',
    url: `http://localhost:${port}/hooks`,
    headers: { [VORN_HEADER]: 'true', Authorization: `Bearer ${token}` },
    timeout: 30
  }
}

function isVornHook(hook: Record<string, unknown>): boolean {
  const headers = hook.headers as Record<string, string> | undefined
  return headers?.[VORN_HEADER] === 'true'
}

export function installHooks(port: number, token: string): void {
  installedPort = port
  let settings: Record<string, unknown> = {}

  // Read existing settings
  try {
    if (fs.existsSync(CLAUDE_SETTINGS_PATH)) {
      settings = JSON.parse(fs.readFileSync(CLAUDE_SETTINGS_PATH, 'utf-8'))
    }
  } catch {
    settings = {}
  }

  const hooks = (settings.hooks ?? {}) as Record<string, unknown[]>

  for (const event of HOOK_EVENTS) {
    const existing = (hooks[event] ?? []) as Record<string, unknown>[]

    // Remove old Vorn hooks
    const filtered = existing.filter((entry) => {
      const entryHooks = (entry.hooks ?? []) as Record<string, unknown>[]
      const nonVG = entryHooks.filter((h) => !isVornHook(h))
      if (nonVG.length === 0 && entryHooks.some((h) => isVornHook(h))) {
        return false // Remove entire entry if it only had VG hooks
      }
      entry.hooks = nonVG
      return true
    })

    // Add new Vorn hook
    filtered.push({
      hooks: [makeHookEntry(port, token)]
    })

    hooks[event] = filtered
  }

  settings.hooks = hooks

  // Write back
  const dir = path.dirname(CLAUDE_SETTINGS_PATH)
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true })
  }
  fs.writeFileSync(CLAUDE_SETTINGS_PATH, JSON.stringify(settings, null, 2), 'utf-8')
}

let installedPort: number | null = null

export function uninstallHooks(): void {
  try {
    if (!fs.existsSync(CLAUDE_SETTINGS_PATH)) return

    // Only a registration we wrote. The old check compared ports, and only when
    // a port had been recorded — so an instance that never installed anything
    // fell straight through and removed the running app's entries on its way out.
    let raw: string | null = null
    try {
      raw = fs.readFileSync(HOOK_OWNER_FILE, 'utf-8')
    } catch {
      // Gone, which is what our own shutdown leaves behind.
    }

    if (
      !mayReleaseHooks({
        owner: parseHookOwner(raw),
        selfPid: process.pid,
        installed: installedPort !== null
      })
    ) {
      return
    }

    const settings = JSON.parse(fs.readFileSync(CLAUDE_SETTINGS_PATH, 'utf-8'))
    const hooks = (settings.hooks ?? {}) as Record<string, unknown[]>

    for (const event of HOOK_EVENTS) {
      const existing = (hooks[event] ?? []) as Record<string, unknown>[]
      hooks[event] = existing.filter((entry) => {
        const entryHooks = (entry.hooks ?? []) as Record<string, unknown>[]
        const nonVG = entryHooks.filter((h) => !isVornHook(h))
        if (nonVG.length === 0) return false
        entry.hooks = nonVG
        return true
      })
      if ((hooks[event] as unknown[]).length === 0) {
        delete hooks[event]
      }
    }

    settings.hooks = hooks
    fs.writeFileSync(CLAUDE_SETTINGS_PATH, JSON.stringify(settings, null, 2), 'utf-8')
  } catch {
    /* ignore */
  }
}
