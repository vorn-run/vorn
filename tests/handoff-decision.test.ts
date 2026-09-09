import { describe, it, expect } from 'vitest'
import { decideHandoff } from '../src/main/server/handoff-request'

/** Conservative by default: every no costs a spawn, and the yes keeps terminals across a release. */
const packaged = { buildChannel: 'packaged' as const }
const unix = 'ws+unix:///Users/x/.vorn/vorn.sock:/ws'

describe('deciding to ask for a handoff', () => {
  it('asks when the running server is an older release', () => {
    const verdict = decideHandoff({
      target: unix,
      platform: 'darwin',
      incumbent: { ...packaged, appVersion: '0.7.0' },
      self: { ...packaged, appVersion: '0.8.0' }
    })
    expect(verdict.ask).toBe(true)
    expect(verdict.why).toContain('0.7.0')
  })

  it('leaves a server that is already this build alone', () => {
    const verdict = decideHandoff({
      target: unix,
      platform: 'darwin',
      incumbent: { ...packaged, appVersion: '0.8.0' },
      self: { ...packaged, appVersion: '0.8.0' }
    })
    expect(verdict).toEqual({ ask: false, why: 'the running server is already this build' })
  })

  it('will not ask over a port', () => {
    // The server refuses it over TCP anyway; asked here so the app declines quietly.
    const verdict = decideHandoff({
      target: 'ws://127.0.0.1:5123/ws',
      platform: 'darwin',
      incumbent: { ...packaged, appVersion: '0.7.0' },
      self: { ...packaged, appVersion: '0.8.0' }
    })
    expect(verdict.ask).toBe(false)
  })

  it('will not ask on a platform that cannot pass a terminal', () => {
    const verdict = decideHandoff({
      target: unix,
      platform: 'win32',
      incumbent: { ...packaged, appVersion: '0.7.0' },
      self: { ...packaged, appVersion: '0.8.0' }
    })
    expect(verdict.ask).toBe(false)
  })

  it('leaves a server somebody started themselves alone, unless told otherwise', () => {
    const cli = {
      target: unix,
      platform: 'darwin' as const,
      incumbent: { ...packaged, appVersion: 'unknown' },
      self: { ...packaged, appVersion: '0.8.0' }
    }
    expect(decideHandoff(cli).ask).toBe(false)
    // A person pressing the button has answered the question the version was
    // standing in for.
    expect(decideHandoff({ ...cli, forced: true }).ask).toBe(true)
  })

  it('asks when forced, even with matching versions', () => {
    // Every dev build carries one version, so nothing else would ever move a server.
    const verdict = decideHandoff({
      target: unix,
      platform: 'darwin',
      incumbent: { buildChannel: 'dev', appVersion: '0.8.0' },
      self: { buildChannel: 'dev', appVersion: '0.8.0' },
      forced: true
    })
    expect(verdict.ask).toBe(true)
  })

  it('never crosses build channels', () => {
    const verdict = decideHandoff({
      target: unix,
      platform: 'darwin',
      incumbent: { buildChannel: 'dev', appVersion: '0.8.0' },
      self: { buildChannel: 'packaged', appVersion: '0.9.0' },
      forced: true
    })
    expect(verdict.ask).toBe(false)
  })
})
