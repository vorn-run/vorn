import fs from 'node:fs'
import path from 'node:path'
import type { TerminalSession } from '@vornrun/shared/types'
import { HANDOFF_MANIFEST_VERSION } from '@vornrun/shared/protocol'

/**
 * What a server tells its replacement about the terminals it is handing over.
 * The descriptors travel separately, in the replacement's stdio array.
 */
export interface HandoffPane {
  /** The stdio index the master descriptor arrives on. */
  slot: number
  session: TerminalSession
  /** Reparents to init when the donor exits, so it can be signalled but never reaped. */
  pid: number
  cols: number
  rows: number
}

export interface HandoffManifest {
  version: number
  donorPid: number
  createdAt: number
  panes: HandoffPane[]
}

/** 0-2 are the replacement's own stdio and 3 is its IPC channel. */
export const FIRST_PTY_SLOT = 4

export function manifestPath(dataDir: string, donorPid: number): string {
  return path.join(dataDir, `handoff-${donorPid}.json`)
}

export function writeManifest(target: string, manifest: HandoffManifest): void {
  // Renamed into place: a half-read manifest would adopt some terminals and drop the rest.
  const scratch = `${target}.partial`
  fs.writeFileSync(scratch, JSON.stringify(manifest), { mode: 0o600 })
  fs.renameSync(scratch, target)
}

/** Null means "cannot be trusted", never "there were no terminals". */
export function readManifest(source: string): HandoffManifest | null {
  try {
    const parsed = JSON.parse(fs.readFileSync(source, 'utf-8')) as unknown
    return isManifest(parsed) ? parsed : null
  } catch {
    return null
  }
}

function isManifest(value: unknown): value is HandoffManifest {
  if (!value || typeof value !== 'object') return false
  const m = value as Partial<HandoffManifest>
  if (m.version !== HANDOFF_MANIFEST_VERSION) return false
  if (!Number.isInteger(m.donorPid) || !Array.isArray(m.panes)) return false
  return m.panes.every(isPane)
}

function isPane(value: unknown): value is HandoffPane {
  if (!value || typeof value !== 'object') return false
  const p = value as Partial<HandoffPane>
  return (
    Number.isInteger(p.slot) &&
    (p.slot as number) >= FIRST_PTY_SLOT &&
    Number.isInteger(p.pid) &&
    (p.pid as number) > 0 &&
    Number.isInteger(p.cols) &&
    (p.cols as number) > 0 &&
    Number.isInteger(p.rows) &&
    (p.rows as number) > 0 &&
    !!p.session &&
    typeof p.session === 'object' &&
    typeof (p.session as TerminalSession).id === 'string'
  )
}

export function discardManifest(target: string): void {
  try {
    fs.rmSync(target, { force: true })
  } catch {
    // Left behind at worst, named by the pid that wrote it.
  }
}
