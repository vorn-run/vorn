import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

/**
 * Which instance owns the hook registration, when more than one is running.
 *
 * Vorn writes its hook endpoint into `~/.claude/settings.json`, and the entry
 * carries a port and a bearer token the hook server generates per process. There
 * is exactly one such registration and any number of servers that might write it:
 * a dev server started beside the packaged app shares `~/.vorn` and `~/.claude`
 * with it, the way it already shares the database.
 *
 * Unowned, that ends badly in three ways, all of them observed rather than
 * imagined. The second instance overwrote the registration with its own port and
 * token, so the app the person was actually using stopped receiving hooks. Killed
 * before it could tidy up, it left the settings pointing at a dead port with a
 * token no live server would accept — and the owning token is a `randomUUID` held
 * in memory, so nothing on disk could repair it. And on a clean exit it removed
 * the registration wholesale, including entries it had never written.
 *
 * The rule is the one `startServer` already applies to the `ws-port` file: a
 * record naming a live process that is not us means we keep our hands off. A
 * record naming a dead one is stale and can be taken.
 *
 * Kept in its own file, beside the port file rather than inside it, because the
 * port file's format is not ours to change: `copilot-hook-installer` bakes
 * `+readFileSync(portFile).trim()` into scripts that are written out to another
 * tool's configuration, where an old one goes on reading a file a new Vorn wrote.
 */
export interface HookOwner {
  port: number
  pid: number
}

export const HOOK_OWNER_FILE = path.join(os.homedir(), '.vorn', 'hook-owner')

/** The record, or null for absent, unreadable or malformed — all "unowned". */
export function parseHookOwner(raw: string | null | undefined): HookOwner | null {
  if (!raw) return null
  try {
    const parsed = JSON.parse(raw) as Partial<HookOwner>
    if (typeof parsed?.port !== 'number' || typeof parsed?.pid !== 'number') return null
    return { port: parsed.port, pid: parsed.pid }
  } catch {
    return null
  }
}

/** The record on disk, or null for absent, unreadable or malformed. */
export function readHookOwnerFile(): HookOwner | null {
  try {
    return parseHookOwner(fs.readFileSync(HOOK_OWNER_FILE, 'utf-8'))
  } catch {
    return null
  }
}

/**
 * Whether a pid belongs to a process that still exists.
 *
 * `EPERM` means it does and we are not allowed to signal it — a Vorn running as
 * another user. Reading that as "dead", which any bare try/catch around
 * `process.kill` does, is the one error here that loses data: it would claim the
 * registration out from under a live instance, which is the whole failure this
 * file exists to prevent.
 */
export function pidIsAlive(pid: number): boolean {
  try {
    process.kill(pid, 0) // a probe, not a signal
    return true
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === 'EPERM'
  }
}

/**
 * Whether this process may write the hook registration.
 *
 * `isAlive` is injected rather than called here so the decision can be tested
 * without spawning anything — and because probing a pid is `process.kill(pid, 0)`,
 * which reads as sending a signal and deserves to be named at the call site.
 */
export function mayClaimHooks(input: {
  owner: HookOwner | null
  selfPid: number
  isAlive: (pid: number) => boolean
}): boolean {
  const { owner, selfPid, isAlive } = input
  if (!owner) return true
  if (owner.pid === selfPid) return true
  return !isAlive(owner.pid)
}

/**
 * Whether this process may remove the hook registration on the way out.
 *
 * `installed` is the half that was missing and is why a second instance used to
 * strip the registration it had never written: the old check only compared ports
 * when a port had been recorded, and fell through to removing everything when one
 * had not.
 */
export function mayReleaseHooks(input: {
  owner: HookOwner | null
  selfPid: number
  installed: boolean
}): boolean {
  const { owner, selfPid, installed } = input
  if (!installed) return false
  // Our own `stop()` deletes the record before this runs, so an absent one is the
  // ordinary shutdown order rather than evidence of somebody else.
  if (!owner) return true
  return owner.pid === selfPid
}
