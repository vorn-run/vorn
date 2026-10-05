import { readFileSync, rmSync } from 'node:fs'
import path from 'node:path'
import type { SessionHolder, SessionHolders } from '@vornrun/shared/types'

/**
 * vorn-sessiond, the session holder vornd keeps running.
 *
 * vornd finds or starts it and drains one left by an older build. The app
 * reads what vornd reports about the holders, and ends an older one when the
 * person asks it to.
 */

const HEALTH_TIMEOUT_MS = 2_000

/**
 * What the vornd on `port` says about its session holders, or null when it
 * keeps none or cannot be asked.
 */
export async function readSessionHolders(
  port: number,
  fetchImpl: typeof fetch = fetch
): Promise<SessionHolders | null> {
  let body: unknown
  try {
    // vornd answers 503 while the server is down, with the same report.
    const res = await fetchImpl(`http://127.0.0.1:${port}/vornd/health`, {
      signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS)
    })
    body = await res.json()
  } catch {
    return null
  }
  return parseHolders((body as { sessiond?: unknown } | null)?.sessiond)
}

function parseHolder(raw: unknown): SessionHolder | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (typeof r.pid !== 'number' || typeof r.instance !== 'string') return null
  return {
    pid: r.pid,
    instance: r.instance,
    build: typeof r.build === 'string' ? r.build : '',
    proto: typeof r.proto === 'number' ? r.proto : 0,
    sessions: typeof r.sessions === 'number' ? r.sessions : null,
    compatible: r.compatible === true
  }
}

function parseHolders(raw: unknown): SessionHolders | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  return {
    current: parseHolder(r.current),
    older: Array.isArray(r.older)
      ? r.older.map(parseHolder).filter((h): h is SessionHolder => h !== null)
      : [],
    error: typeof r.error === 'string' ? r.error : null
  }
}

/** The pid and instance a holder announced under `home`, or null. */
export function readAnnouncement(
  home: string,
  instance: string,
  readFile: (file: string) => string = (file) => readFileSync(file, 'utf8')
): { pid: number; instance: string } | null {
  if (!/^[0-9a-f]{1,32}$/.test(instance)) return null
  let text: string
  try {
    text = readFile(announcementPath(home, instance))
  } catch {
    return null
  }
  const fields = new Map(
    text
      .split('\n')
      .map((line) => line.split('='))
      .filter((kv) => kv.length === 2)
      .map(([k, v]) => [k!, v!] as const)
  )
  const pid = Number(fields.get('pid'))
  if (!Number.isInteger(pid) || pid <= 0 || fields.get('instance') !== instance) return null
  return { pid, instance }
}

function announcementPath(home: string, instance: string): string {
  return path.join(home, 'run', `sessiond-${instance}.info`)
}

/**
 * End an older session holder, and with it the sessions it still holds.
 *
 * Only one vornd reports as older, and only when its announcement under `home`
 * still names the same process: a pid alone may since belong to something else.
 */
export function endOlderHolder(
  home: string,
  reported: SessionHolders | null,
  instance: string,
  io: {
    readFile?: (file: string) => string
    kill?: (pid: number) => void
    remove?: (file: string) => void
  } = {}
): { ok: true } | { ok: false; detail: string } {
  const older = reported?.older.find((h) => h.instance === instance)
  if (!older) return { ok: false, detail: 'vornd does not report that session holder as older' }
  const announced = readAnnouncement(home, instance, io.readFile)
  if (!announced || announced.pid !== older.pid) {
    return { ok: false, detail: 'that session holder is no longer running' }
  }
  try {
    ;(io.kill ?? ((pid) => process.kill(pid, 'SIGKILL')))(announced.pid)
  } catch (err) {
    return { ok: false, detail: (err as Error).message }
  }
  // It cannot withdraw its own announcement after SIGKILL.
  ;(io.remove ?? ((file) => rmSync(file, { force: true })))(announcementPath(home, instance))
  return { ok: true }
}
