import { describe, it, expect } from 'vitest'
import {
  mayClaimHooks,
  mayReleaseHooks,
  parseHookOwner,
  pidIsAlive
} from '../packages/server/src/hook-ownership'

/**
 * Which Vorn owns the hook registration when two are running.
 *
 * There is one entry in `~/.claude/settings.json` and any number of servers that
 * might write it: a dev server started beside the packaged app shares `~/.vorn`
 * and `~/.claude` with it, the way it already shares the database.
 *
 * All three failures below were observed on a real machine, not imagined. A dev
 * server overwrote the registration with its own port and token, so the app the
 * person was using stopped receiving hooks. Killed with SIGKILL before it could
 * tidy up, it left the settings naming a dead port and a token no live server
 * would accept — and the owning token is a `randomUUID` held only in memory, so
 * nothing on disk could repair it; the fix was to delete the entries by hand.
 * And on a clean exit it removed the registration wholesale, including entries
 * it had never written.
 */

const ALIVE = () => true
const DEAD = () => false

describe('reading the ownership record', () => {
  it.each([
    ['absent', null],
    ['empty', ''],
    ['not json', 'not json at all'],
    ['json but not a record', '"56432"'],
    ['missing the pid', '{"port":56432}'],
    ['a pid that is not a number', '{"port":56432,"pid":"7116"}']
  ])('reads %s as unowned rather than throwing', (_label, raw) => {
    expect(parseHookOwner(raw)).toBeNull()
  })

  it('reads a well-formed record', () => {
    expect(parseHookOwner('{"port":56432,"pid":7116}')).toEqual({ port: 56432, pid: 7116 })
  })
})

describe('deciding whether a pid is still there', () => {
  it('sees this very process', () => {
    expect(pidIsAlive(process.pid)).toBe(true)
  })

  it('sees through a pid that is gone', () => {
    // Pid 0 has a special meaning to `kill` and is never a process to probe;
    // 2^22 is above every platform's default pid_max, so nothing owns it.
    expect(pidIsAlive(4_194_304)).toBe(false)
  })

  it('counts a process it may not signal as alive', () => {
    // `process.kill` throws EPERM when the process exists and belongs to someone
    // else — a Vorn running as another user. A bare try/catch reads that as dead
    // and claims the registration out from under it, which is the one error here
    // that loses somebody's work. Pid 1 is `launchd`/`init`, owned by root.
    expect(pidIsAlive(1)).toBe(true)
  })
})

describe('claiming the registration', () => {
  it('claims it when nobody holds it', () => {
    expect(mayClaimHooks({ owner: null, selfPid: 100, isAlive: ALIVE })).toBe(true)
  })

  it('leaves a live instance alone', () => {
    // The case that broke a running app: the second server wrote its own port
    // and token over the first's, and the first went on running, unhooked.
    expect(mayClaimHooks({ owner: { port: 56432, pid: 7116 }, selfPid: 100, isAlive: ALIVE })).toBe(
      false
    )
  })

  it('takes over from an instance that died', () => {
    // Otherwise a crash would leave hooks unowned forever, with every later
    // launch politely declining to register.
    expect(mayClaimHooks({ owner: { port: 56432, pid: 7116 }, selfPid: 100, isAlive: DEAD })).toBe(
      true
    )
  })

  it('reclaims its own record across a restart of the server object', () => {
    expect(mayClaimHooks({ owner: { port: 56432, pid: 100 }, selfPid: 100, isAlive: ALIVE })).toBe(
      true
    )
  })
})

describe('releasing the registration', () => {
  it('removes what it installed', () => {
    expect(
      mayReleaseHooks({ owner: { port: 56432, pid: 100 }, selfPid: 100, installed: true })
    ).toBe(true)
  })

  it('removes nothing when it installed nothing', () => {
    // The third failure. The old guard compared ports only when a port had been
    // recorded, so an instance that never installed fell through the check and
    // stripped the running app's entries on its way out.
    expect(mayReleaseHooks({ owner: null, selfPid: 100, installed: false })).toBe(false)
    expect(
      mayReleaseHooks({ owner: { port: 56432, pid: 7116 }, selfPid: 100, installed: false })
    ).toBe(false)
  })

  it('does not remove another live instance’s registration', () => {
    expect(
      mayReleaseHooks({ owner: { port: 56432, pid: 7116 }, selfPid: 100, installed: true })
    ).toBe(false)
  })

  it('still tidies up when the record is already gone', () => {
    // `stop()` deletes the record before `uninstallHooks` runs, so an absent one
    // is the ordinary shutdown order rather than evidence of somebody else.
    expect(mayReleaseHooks({ owner: null, selfPid: 100, installed: true })).toBe(true)
  })
})
