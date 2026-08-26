import fs from 'fs'
import path from 'path'
import { WS_PORT_FILENAME } from '@vornrun/shared/protocol'
import log from './logger'

/**
 * The files this server publishes into its data directory, and who owns them.
 *
 * Two of them — `ws-port` and `local-token` — are how anything else on this
 * machine finds and authenticates to a server. They live at fixed names in a
 * directory that is shared by default: `~/.vorn` is where a packaged Vorn and a
 * `yarn dev` server both land, deliberately, which is what lets `judgeAdoption`
 * gate on build channel rather than on paths.
 *
 * A fixed name shared by several writers needs an owner, or the last writer
 * silently wins. It did: a dev server rewrote the packaged app's credential on
 * start and removed it on stop, so MCP read one server's port beside another
 * server's secret and every call timed out until Vorn was restarted.
 *
 * The rule here is the one the endpoint work needs next, on a smaller thing:
 *
 * > No actor removes or replaces a name it did not create, unless it has proven
 * > the process that did create it is gone.
 *
 * Ownership is decided once, before anything is published, and remembered. It is
 * not re-derived per file, because two files answering the question separately is
 * how they come to disagree.
 */

/**
 * Whether a process is still there.
 *
 * `EPERM` means alive and owned by somebody else — the signal was refused, which
 * is only possible if there was something to refuse it. Reading that as death is
 * how a live server gets treated as absent, and the older inline version of this
 * check did exactly that with a bare try/catch.
 *
 * Signal 0 to a pid of 0 targets this process group and always succeeds, so a
 * zero or negative pid is rejected before it is asked. Mirrors
 * `src/main/server/server-adoption.ts`, which the desktop uses for the same
 * question.
 */
export function isPidAlive(pid: number): boolean {
  if (!Number.isInteger(pid) || pid <= 0) return false
  try {
    process.kill(pid, 0)
    return true
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === 'EPERM'
  }
}

/** What `ws-port` holds. `pid` is absent in records MCP heals for itself. */
interface PortRecord {
  port?: number
  pid?: number
}

function readPortRecord(file: string): PortRecord | null {
  try {
    const raw: unknown = JSON.parse(fs.readFileSync(file, 'utf-8'))
    if (!raw || typeof raw !== 'object') return null
    return raw as PortRecord
  } catch {
    // Absent, unreadable, or the legacy bare-number format. None of them names a
    // living owner, so none of them stops this process claiming the directory.
    return null
  }
}

/**
 * Decide whether this process may publish into `dataDir`, once and for all.
 *
 * Claimed unless `ws-port` names a *live* process that is not us. Everything
 * else — no file, unreadable JSON, a record with no pid, a pid that has since
 * died — is an orphan, and an orphan is claimable. That is deliberate: refusing
 * to publish because of a leftover file would leave a machine with no reachable
 * server at all, which is worse than the collision this prevents.
 *
 * Call before publishing anything, and pass the answer to every publisher.
 */
export function claimPublishedFiles(dataDir: string): boolean {
  const record = readPortRecord(path.join(dataDir, WS_PORT_FILENAME))
  const owner = record?.pid
  if (typeof owner !== 'number' || owner === process.pid) return true
  if (!isPidAlive(owner)) return true

  log.info(
    { owner },
    '[server] another live server owns this data directory; publishing nothing into it'
  )
  return false
}

/**
 * Publish the port, if this process owns the directory.
 *
 * Lives beside the database rather than always in `~/.vorn`, so a server on its
 * own data dir advertises itself there instead of over the desktop's file.
 */
export function writePortFile(dataDir: string, port: number, owned: boolean): void {
  if (!owned) return
  const file = path.join(dataDir, WS_PORT_FILENAME)
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, JSON.stringify({ port, pid: process.pid }), 'utf-8')
  } catch (err) {
    log.warn({ err }, '[server] failed to write ws-port file (MCP discovery will not work)')
  }
}

/**
 * Remove the port file, if it still names this process.
 *
 * Two gates rather than one: the claim taken at startup, and a fresh read
 * proving the record on disk is still ours. The second matters because the file
 * is the thing being removed — trusting a flag decided minutes ago would delete
 * a name that has since become somebody else's.
 */
export function removePortFile(dataDir: string, owned: boolean): void {
  if (!owned) return
  const file = path.join(dataDir, WS_PORT_FILENAME)
  try {
    if (readPortRecord(file)?.pid === process.pid) fs.unlinkSync(file)
  } catch {
    /* best effort — a port file naming a dead process is already harmless */
  }
}
