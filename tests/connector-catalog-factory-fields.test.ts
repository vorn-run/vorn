import { describe, expect, it } from 'vitest'
import { parseCatalog } from '../packages/server/src/connectors/catalog'

/** The least an entry can carry and still be worth listing. */
const entry = {
  id: 'x',
  name: 'X',
  description: 'd',
  packageName: '@vornrun/connector-x',
  capabilities: ['triggers']
}

const first = (raw: Record<string, unknown>) =>
  parseCatalog({ version: 1, connectors: [{ ...entry, ...raw }] })?.[0]

describe('the rung a catalog entry declares', () => {
  it('is carried when this build knows how to describe it', () => {
    for (const authRung of ['none', 'cli', 'key', 'oauth']) {
      expect(first({ authRung })?.authRung).toBe(authRung)
    }
  })

  it('is dropped when it is not one, so a filter never has a phantom facet', () => {
    expect(first({ authRung: 'sso' })?.authRung).toBeUndefined()
    expect(first({ authRung: 7 })?.authRung).toBeUndefined()
    expect(first({})?.authRung).toBeUndefined()
  })
})

describe('the receipt behind a verified mark', () => {
  const verified = { version: '1.2.0', checkedAt: '2026-09-02T00:00:00Z', checks: ['manifest'] }

  it('is carried whole when it says what ran and when', () => {
    expect(first({ verified })?.verified).toEqual(verified)
  })

  it('survives a missing check list, because the version and date are the claim', () => {
    const checkless = { version: verified.version, checkedAt: verified.checkedAt }
    expect(first({ verified: checkless })?.verified).toEqual({ ...checkless, checks: [] })
    expect(first({ verified: { ...verified, checks: 'all' } })?.verified?.checks).toEqual([])
  })

  it('is refused when half-written, rather than vouching for nothing', () => {
    // A mark on a check nobody ran is worse than no mark at all.
    expect(first({ verified: { version: '1.2.0' } })?.verified).toBeUndefined()
    expect(first({ verified: { checkedAt: '2026-09-02T00:00:00Z' } })?.verified).toBeUndefined()
    expect(first({ verified: 'yes' })?.verified).toBeUndefined()
    expect(first({ verified: [] })?.verified).toBeUndefined()
  })
})

describe('the arguments an action declares', () => {
  it('are carried, so a step can be offered before the connector is installed', () => {
    const inputs = [{ key: 'message', label: 'Message', type: 'string', required: true }]
    const actions = [{ type: 'post', label: 'Post message', inputs }]
    expect(first({ actions })?.actions).toEqual(actions)
  })

  it('are filled in where the publisher was terse', () => {
    const actions = [{ type: 'post', label: 'Post', inputs: [{ key: 'text' }] }]
    expect(first({ actions })?.actions?.[0].inputs).toEqual([
      { key: 'text', label: 'text', type: 'string', required: false }
    ])
  })

  it('are repaired rather than handed to the library to map over', () => {
    // The library maps over these to build steps; a string where a list
    // belongs would take the step list down with a TypeError.
    const withInputs = (inputs: unknown) => first({ actions: [{ type: 'p', label: 'P', inputs }] })
    expect(withInputs('message')?.actions).toEqual([{ type: 'p', label: 'P', inputs: [] }])
    expect(withInputs(['message', { no: 'key' }])?.actions?.[0].inputs).toEqual([])
    expect(first({ actions: 'all of them' })?.actions).toEqual([])
  })
})
