import { describe, expect, it } from 'vitest'
import { defineConnector } from '../packages/connector-sdk/src/index'
import type { ConnectorDefinition } from '../packages/connector-sdk/src/types'
import { greeted } from './helpers/connector-server'

// For a connector whose credentials come from an external login, so trouble shows before a poll fails.
const preflight = async (definition: ConnectorDefinition) =>
  (await greeted(defineConnector(definition))).call('connector/preflight')

const base: ConnectorDefinition = {
  id: 'acme',
  name: 'Acme',
  version: '1.0.0',
  triggers: [{ type: 't', label: 'T', poll: () => ({ items: [] }) }]
}

describe('the preflight a connector answers', () => {
  // Null rather than ok: a connector with nothing to check has not been verified.
  it('is null when the connector declares none', async () => {
    expect(await preflight(base)).toEqual({ ok: null })
  })

  it('reports a passing check', async () => {
    expect(await preflight({ ...base, preflight: () => ({ ok: true }) })).toEqual({ ok: true })
  })

  it('carries the message back, because it says what to do about it', async () => {
    expect(
      await preflight({
        ...base,
        preflight: () => ({ ok: false, message: 'Sign in by running `gh auth login`.' })
      })
    ).toEqual({ ok: false, message: 'Sign in by running `gh auth login`.' })
  })

  it('awaits an async preflight rather than reporting the promise', async () => {
    expect(
      await preflight({
        ...base,
        preflight: async () => {
          await Promise.resolve()
          return { ok: false, message: 'still no' }
        }
      })
    ).toEqual({ ok: false, message: 'still no' })
  })

  // A throw means "broken"; it answers as a failed check, not as a failed call.
  it('turns a throw into a failed check carrying its message', async () => {
    expect(
      await preflight({
        ...base,
        preflight: () => {
          throw new Error('gh not found on PATH')
        }
      })
    ).toEqual({ ok: false, message: 'gh not found on PATH' })
  })

  it('survives a rejected promise the same way', async () => {
    expect(
      await preflight({ ...base, preflight: () => Promise.reject(new Error('signed out')) })
    ).toEqual({ ok: false, message: 'signed out' })
  })
})
