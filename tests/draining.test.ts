import { describe, it, expect, beforeEach } from 'vitest'
import {
  beginDraining,
  isDraining,
  watchEndpoint,
  resetDrainingForTests,
  DRAINING_MESSAGE
} from '../packages/server/src/draining'

/**
 * Refusing to start work on a name this process no longer holds.
 *
 * The claim ends with a probe and a rename, two syscalls POSIX gives no way to
 * fuse, so the window where two servers might each believe they hold the name
 * cannot be closed. This is where the harm is closed instead: a session started
 * here would be reachable only through a name that now points elsewhere, and the
 * person who started it would watch a terminal that accepts keystrokes and never
 * runs them.
 */

beforeEach(() => {
  resetDrainingForTests()
})

describe('before anything has been lost', () => {
  it('is not draining', () => {
    expect(isDraining()).toBe(false)
  })

  it('is not draining for a server that never held an endpoint', () => {
    // win32, a directory anyone can write, a path too long for sun_path. That is
    // a downgrade to TCP-only, not a loss -- and refusing its sessions would
    // leave the machine with nothing that works.
    expect(isDraining()).toBe(false)
  })
})

describe('once the endpoint is gone', () => {
  it('notices by looking, rather than waiting to be told', () => {
    // Losing the name happens *to* this process: whoever took it has no way to
    // say so. An earlier version only ever noticed inside the idle watch's
    // snapshot -- up to a minute late, and never at all for `vorn-server serve`,
    // where that watch is switched off.
    let held = true
    watchEndpoint(() => held)
    expect(isDraining()).toBe(false)

    held = false
    expect(isDraining()).toBe(true)
  })

  it('does not un-notice when the name comes back', () => {
    // Nothing gives an endpoint back. A name that reappears is somebody else's
    // socket at the same path, and resuming on it would be the original bug.
    let held = true
    watchEndpoint(() => held)
    held = false
    expect(isDraining()).toBe(true)

    held = true
    expect(isDraining()).toBe(true)
  })

  it('stops asking once it knows', () => {
    // The check is an lstat per session creation. Cheap, but there is no reason
    // to keep paying it for an answer that cannot change.
    let asked = 0
    watchEndpoint(() => {
      asked++
      return false
    })

    isDraining()
    isDraining()
    isDraining()

    expect(asked).toBe(1)
  })

  it('can be told directly, without a check to consult', () => {
    beginDraining()
    expect(isDraining()).toBe(true)
  })
})

describe('what a person is told', () => {
  it('says the server is finishing, not that something failed', () => {
    // The sessions already running are fine, and the fix is to reopen Vorn. A
    // message that read as an error would send someone looking for a broken
    // thing.
    expect(DRAINING_MESSAGE).toMatch(/finishing/i)
    expect(DRAINING_MESSAGE).toMatch(/reopen/i)
  })
})
