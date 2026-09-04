import fs from 'node:fs'
import path from 'node:path'
import { app } from 'electron'
import { isPidAlive } from './server/server-adoption'
import log from './logger'

/**
 * Which simulators this machine's Vorn processes are holding.
 *
 * `claimFor` arbitrates against this process's own entries, which is the whole
 * answer while there is one Vorn. There is not: a packaged build and a `yarn
 * dev` run side by side all day, and both would claim one udid and both drive
 * it — two sets of taps interleaved on one screen, which looks exactly like the
 * app under test misbehaving and has nothing on screen to say otherwise. That
 * is the failure the in-process rule exists to prevent, arriving through the
 * one door it does not watch.
 *
 * A file rather than a lock, beside `host.json` in `userData` and for the same
 * reason: it has to be readable before anything else is up. The pid is what
 * makes it safe to trust — an entry naming a dead process is treated as absent,
 * exactly as a stale port file is, because a crashed Vorn has no way to clean up
 * after itself and a record nobody can act on must never wedge a device shut.
 */
interface StoredClaim {
  pid: number
  sessionId: string
}

type Stored = Record<string, StoredClaim>

const FILE = (): string => path.join(app.getPath('userData'), 'device-claims.json')

function read(): Stored {
  let raw: string
  try {
    raw = fs.readFileSync(FILE(), 'utf-8')
  } catch {
    return {}
  }
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Stored = {}
    for (const [udid, value] of Object.entries(parsed as Record<string, unknown>)) {
      const claim = value as Partial<StoredClaim>
      if (!Number.isInteger(claim?.pid) || (claim.pid as number) <= 0) continue
      if (typeof claim.sessionId !== 'string' || !claim.sessionId) continue
      out[udid] = { pid: claim.pid as number, sessionId: claim.sessionId }
    }
    return out
  } catch {
    log.warn('[device-claims] record is unreadable; treating every device as free')
    return {}
  }
}

/**
 * Replace the record in one step, never in place.
 *
 * A torn write is not a harmless corruption here. `read` treats an unparseable
 * file as no claims at all -- which is the right answer for a file that was
 * never written, and the wrong one for a file that was half written, because it
 * reports every device free and lets a second Vorn take one this process is
 * driving. That is precisely the collision this file exists to prevent, arriving
 * through the file itself. Writing beside it and renaming makes the swap atomic,
 * so a reader sees the old record or the new one and never half of either.
 *
 * The scratch name carries this process's pid and nothing finer, which is safe
 * only because both calls below are synchronous: nothing else in this process
 * runs between them, so the file cannot exist for two writes at once. Another
 * Vorn writing at the same moment has a different pid and its own scratch file.
 * If either call is ever made async, the name has to become unique per write.
 *
 * What this does not give is a lock. Two Vorns that read and write in the same
 * instant can still lose one of the two claims, and the loser's device then
 * reads as free to whoever asks next. Closing that needs an exclusive lock file
 * rather than a bigger hammer here, and it is deliberately not done: a lost
 * record degrades to the behaviour before any of this existed -- arbitration
 * within one process only -- rather than to something worse.
 */
function write(claims: Stored): void {
  // Resolved inside the guard, like the read's is: `FILE()` reaches into
  // Electron for the data directory and can throw before any file is touched.
  let tmp: string | undefined
  try {
    const target = FILE()
    tmp = `${target}.${process.pid}.tmp`
    fs.writeFileSync(tmp, JSON.stringify(claims, null, 2))
    fs.renameSync(tmp, target)
  } catch (err) {
    // A record that cannot be written costs cross-process arbitration and
    // nothing else. Refusing the claim over it would make a read-only userData
    // directory look like every simulator being held.
    log.warn({ err }, '[device-claims] could not write the record')
    try {
      if (tmp) fs.unlinkSync(tmp)
    } catch {
      // Nothing to clean up, or nothing we can do about it.
    }
  }
}

/**
 * The live claim another Vorn holds on this device, or null.
 *
 * Null covers absent, ours, and stale — the three that all mean "free to take".
 * Collapsing them is deliberate: only a claim someone can still act on is a
 * reason to refuse, and a dead pid is not one.
 */
export function foreignClaim(udid: string): StoredClaim | null {
  const held = read()[udid]
  if (!held) return null
  if (held.pid === process.pid) return null
  if (!isPidAlive(held.pid)) return null
  return held
}

export function recordClaim(udid: string, sessionId: string): void {
  const claims = read()
  claims[udid] = { pid: process.pid, sessionId }
  write(claims)
}

/** Let go of a device, and of anything a Vorn that is no longer running left behind. */
export function dropClaim(udid: string): void {
  const claims = read()
  const before = Object.keys(claims).length
  if (claims[udid]?.pid === process.pid) delete claims[udid]
  // Swept here rather than on a timer: this is the moment the file is already
  // open, and a record left by a crashed Vorn is otherwise never tidied — it
  // stays harmless, because a dead pid reads as free, but it accumulates.
  for (const [id, claim] of Object.entries(claims)) {
    if (!isPidAlive(claim.pid)) delete claims[id]
  }
  if (Object.keys(claims).length !== before) write(claims)
}

/** Every device this process is recorded as holding, for the way out. */
export function dropAllClaimsForThisProcess(): void {
  const claims = read()
  let changed = false
  for (const [udid, claim] of Object.entries(claims)) {
    if (claim.pid !== process.pid && isPidAlive(claim.pid)) continue
    delete claims[udid]
    changed = true
  }
  if (changed) write(claims)
}
