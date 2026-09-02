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

describe('what a connector says it fires on', () => {
  it('gets the same repair an action does, since the same row draws it', () => {
    const triggers = [
      { type: 'opened', label: 'Issue opened', description: 'Each new issue' },
      { type: 'closed' },
      { type: 'noisy', label: 7, description: { long: 'nope' } }
    ]
    expect(first({ triggers })?.triggers).toEqual([
      { type: 'opened', label: 'Issue opened', description: 'Each new issue' },
      { type: 'closed', label: 'closed' },
      { type: 'noisy', label: 'noisy' }
    ])
  })

  it('drops what cannot be named or is not an entry at all', () => {
    expect(first({ triggers: [{ label: 'Nameless' }, 'opened', [], null] })?.triggers).toEqual([])
    expect(first({ triggers: 'nope' })?.triggers).toEqual([])
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

  it('carry the choices a select needs to be drawn before install', () => {
    const inputs = [
      {
        key: 'level',
        label: 'Level',
        type: 'select',
        required: false,
        options: [{ value: 'high' }, { value: 'low', label: 'Low priority' }]
      },
      { key: 'channel', label: 'Channel', type: 'select', required: true, loadOptions: 'channels' }
    ]
    expect(first({ actions: [{ type: 'p', label: 'P', inputs }] })?.actions?.[0].inputs).toEqual(
      inputs
    )
  })

  it('drop a choice that would select nothing, and a nameless options set', () => {
    const withInput = (input: Record<string, unknown>) =>
      first({ actions: [{ type: 'p', label: 'P', inputs: [{ key: 'v', ...input }] }] })?.actions?.[0]
        .inputs?.[0]
    expect(withInput({ options: [{ label: 'Empty' }, 'high', { value: 'ok' }] })?.options).toEqual([
      { value: 'ok' }
    ])
    expect(withInput({ options: 'high,low' })?.options).toBeUndefined()
    expect(withInput({ loadOptions: '' })?.loadOptions).toBeUndefined()
    expect(withInput({ loadOptions: 12 })?.loadOptions).toBeUndefined()
  })

  it('are repaired rather than handed to the library to map over', () => {
    // The library maps over these to build steps; a string where a list
    // belongs would take the step list down with a TypeError.
    const withInputs = (inputs: unknown) => first({ actions: [{ type: 'p', label: 'P', inputs }] })
    expect(withInputs('message')?.actions).toEqual([{ type: 'p', label: 'P', inputs: [] }])
    expect(withInputs(['message', { no: 'key' }])?.actions?.[0].inputs).toEqual([])
    expect(first({ actions: 'all of them' })?.actions).toEqual([])
  })

  it('keeps prose only when it is prose', () => {
    // The row renders this straight into the DOM; an object would read as
    // "[object Object]" under the action's name.
    const actions = [{ type: 'post', label: 'Post', description: { long: 'nope' } }]
    expect(first({ actions })?.actions).toEqual([{ type: 'post', label: 'Post' }])
    expect(first({ actions: [{ type: 'post', label: 42 }] })?.actions).toEqual([
      { type: 'post', label: 'post' }
    ])
  })

  it('drops an action with nothing to call it by, rather than listing a blank step', () => {
    expect(first({ actions: [{ label: 'Nameless' }, 'post', []] })?.actions).toEqual([])
    // A label is what a person reads; falling back to the type beats an empty row.
    expect(first({ actions: [{ type: 'post' }] })?.actions).toEqual([
      { type: 'post', label: 'post' }
    ])
  })
})
