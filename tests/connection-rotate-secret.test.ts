import { describe, it, expect, beforeEach } from 'vitest'
import {
  clearDecryptedCreds,
  getDecryptedCreds,
  setDecryptedCreds
} from '../packages/server/src/connectors/decrypted-creds'
import { secretEnvFor } from '../packages/server/src/script-runner'

/**
 * The rotate handler itself needs a database and a live server, which no unit
 * test here stands up. What it promises that a caller can observe is the state
 * it leaves behind: the new secret readable at once, merged with whatever else
 * the connection holds. That is what these pin, against the same store the
 * handler writes and every consumer reads.
 */
function rotateInto(connectionId: string, field: string, plaintext: string): void {
  setDecryptedCreds(connectionId, { ...getDecryptedCreds(connectionId), [field]: plaintext })
}

beforeEach(() => {
  clearDecryptedCreds('conn-1')
  clearDecryptedCreds('conn-2')
})

describe('what a rotation leaves behind', () => {
  it('serves the new secret immediately, with no window where none is readable', () => {
    setDecryptedCreds('conn-1', { secret: 'sk_live_old' })
    rotateInto('conn-1', 'secret', 'sk_live_new')

    // Read the way a poll or an action reads it, with no resync in between.
    expect(getDecryptedCreds('conn-1')).toEqual({ secret: 'sk_live_new' })
  })

  it('leaves the connection’s other secrets alone', () => {
    setDecryptedCreds('conn-2', { apiKey: 'keep-me', secretEnv: '{"TOKEN":"old"}' })
    rotateInto('conn-2', 'secretEnv', '{"TOKEN":"new"}')

    expect(getDecryptedCreds('conn-2')).toEqual({
      apiKey: 'keep-me',
      secretEnv: '{"TOKEN":"new"}'
    })
  })

  it('hands a step spawned right after the rotation the new value', () => {
    setDecryptedCreds('conn-2', { secretEnv: '{"TOKEN":"old"}' })
    rotateInto('conn-2', 'secretEnv', '{"TOKEN":"new"}')

    expect(secretEnvFor('conn-2')).toEqual({ TOKEN: 'new' })
  })

  it('is the first thing a connection knows, when it held nothing before', () => {
    rotateInto('conn-1', 'secret', 'sk_live_first')
    expect(getDecryptedCreds('conn-1')).toEqual({ secret: 'sk_live_first' })
  })
})
