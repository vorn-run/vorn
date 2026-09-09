import { describe, it, expect } from 'vitest'
import { describeServerRuntime } from '../src/renderer/lib/server-runtime'
import type { ServerRuntimeStatus } from '../src/shared/types'

/** The wording somebody reads while working out why a fix they know shipped did nothing. */
const base: ServerRuntimeStatus = {
  serverVersion: '0.8.0',
  appVersion: '0.8.0',
  serverPid: 100,
  adopted: true,
  canUpgrade: true,
  sessions: 3,
  lastUpgrade: null
}

describe('what the panel says about the running server', () => {
  it('reports a matching build without offering anything', () => {
    const view = describeServerRuntime(base)
    expect(view.offerMove).toBe(false)
    expect(view.trailing).toBe('Current')
  })

  it('names both versions when they differ, and what moving would carry', () => {
    const view = describeServerRuntime({ ...base, serverVersion: '0.7.0' })
    expect(view.description).toContain('0.7.0')
    expect(view.description).toContain('0.8.0')
    // "Moving" sounds destructive until it says what happens to the terminals.
    expect(view.description).toContain('3 terminals across without stopping them')
    expect(view.offerMove).toBe(true)
  })

  it('counts one terminal in the singular', () => {
    const view = describeServerRuntime({ ...base, serverVersion: '0.7.0', sessions: 1 })
    expect(view.description).toContain('1 terminal across without stopping it')
  })

  it('says why the button is missing rather than showing a dead one', () => {
    const view = describeServerRuntime({ ...base, serverVersion: '0.7.0', canUpgrade: false })
    expect(view.offerMove).toBe(false)
    expect(view.description).toContain('needs a restart of Vorn')
  })

  it('carries the reason a previous attempt failed', () => {
    const view = describeServerRuntime({
      ...base,
      serverVersion: '0.7.0',
      lastUpgrade: { kind: 'failed', why: 'a terminal could not be described' }
    })
    expect(view.description).toContain('a terminal could not be described')
    // Still offered: nothing was lost, so trying again is free.
    expect(view.offerMove).toBe(true)
  })

  it('says a move is under way while it is', () => {
    const view = describeServerRuntime({ ...base, lastUpgrade: { kind: 'working' } })
    expect(view.description).toContain('keep running')
    expect(view.offerMove).toBe(false)
  })

  it('names a server nobody in this app started', () => {
    const view = describeServerRuntime({ ...base, serverVersion: 'unknown' })
    expect(view.description).toContain('started outside Vorn')
  })
})
