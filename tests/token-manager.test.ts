import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

// Mock logger and filesystem to prevent side effects
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import { randomUUID, createHash } from 'node:crypto'
import {
  initTestDatabase,
  dbGetOwnerUser,
  dbGetDeviceTokenSecret,
  dbInsertDeviceToken
} from '../packages/server/src/database'
import {
  mintToken,
  mintOwnerToken,
  verifyToken,
  listTokens,
  hasTokens,
  revokeToken,
  touchLastSeen
} from '../packages/server/src/token-manager'

let teardown: () => void

/**
 * The secret half of a token. Not `split('_')[2]` — the secret is base64url and
 * may itself contain `_`, which is the bug this helper exists to avoid repeating
 * in the tests.
 */
function secretPartOf(plaintext: string, tokenId: string): string {
  return plaintext.slice(`vorn_${tokenId}_`.length)
}

/** The owner seeded by migration 14 — every test needs somebody to mint for. */
function ownerId(): string {
  const owner = dbGetOwnerUser()
  if (!owner) throw new Error('expected a seeded owner')
  return owner.id
}

beforeEach(() => {
  teardown = initTestDatabase()
})

afterEach(() => {
  teardown()
})

describe('seeded owner', () => {
  it('exists on a freshly created database', () => {
    const owner = dbGetOwnerUser()
    expect(owner).not.toBeNull()
    expect(owner?.role).toBe('owner')
    expect(owner?.name).toBeTruthy()
  })
})

describe('mintToken', () => {
  it('returns a token that parses and verifies', () => {
    const { token, plaintext } = mintToken(ownerId(), 'iPhone')

    // The id travels in the clear so verification is a primary-key lookup.
    expect(plaintext.startsWith(`vorn_${token.id}_`)).toBe(true)
    expect(secretPartOf(plaintext, token.id)).not.toBe('')
    expect(verifyToken(plaintext)).toEqual({ userId: token.userId, tokenId: token.id })
  })

  it('verifies a secret that contains the separator character', () => {
    // The secret is base64url, whose alphabet includes `_`. Splitting the whole
    // token on `_` rejected those — intermittently, for only some of the tokens
    // it had itself minted, which is the worst possible way to fail. Built by
    // hand rather than by minting, so the case is covered on every run instead
    // of whenever the random bytes happen to encode an underscore.
    const id = randomUUID()
    const secret = 'aa_bb__cc'
    const owner = ownerId()

    dbInsertDeviceToken({
      id,
      userId: owner,
      name: 'underscored secret',
      tokenHash: createHash('sha256').update(secret, 'utf8').digest('hex'),
      createdAt: new Date().toISOString()
    })

    expect(verifyToken(`vorn_${id}_${secret}`)).toEqual({ userId: owner, tokenId: id })
  })

  it('never stores the secret — only its hash', () => {
    const { token, plaintext } = mintToken(ownerId(), 'iPhone')
    const secret = secretPartOf(plaintext, token.id)

    const record = dbGetDeviceTokenSecret(token.id)
    expect(record).not.toBeNull()
    expect(record?.tokenHash).toHaveLength(64) // sha256, hex

    // Nothing anywhere in the persisted row reproduces the secret.
    expect(JSON.stringify(record)).not.toContain(secret)
  })

  it('gives two tokens different secrets', () => {
    const a = mintToken(ownerId(), 'one')
    const b = mintToken(ownerId(), 'two')
    expect(a.plaintext).not.toBe(b.plaintext)
    expect(a.token.id).not.toBe(b.token.id)
  })

  it('returns a token carrying no secret, so a listing cannot leak one', () => {
    const { token, plaintext } = mintToken(ownerId(), 'iPhone')
    const secret = secretPartOf(plaintext, token.id)
    expect(JSON.stringify(token)).not.toContain(secret)
  })
})

describe('mintOwnerToken', () => {
  it('mints for the seeded owner without being told who that is', () => {
    const { token } = mintOwnerToken('first-run')
    expect(token.userId).toBe(ownerId())
  })
})

describe('hasTokens', () => {
  it('is false on a fresh database and true once one is minted', () => {
    expect(hasTokens()).toBe(false)
    mintToken(ownerId(), 'iPhone')
    expect(hasTokens()).toBe(true)
  })

  it('stays true for a revoked token, which still exists', () => {
    const { token } = mintToken(ownerId(), 'iPhone')
    revokeToken(token.id)
    expect(hasTokens()).toBe(true)
  })
})

describe('verifyToken', () => {
  it('rejects a tampered secret', () => {
    const { token, plaintext } = mintToken(ownerId(), 'iPhone')
    const secret = secretPartOf(plaintext, token.id)
    const flipped = `${secret.slice(0, -1)}${secret.endsWith('A') ? 'B' : 'A'}`
    const tampered = `vorn_${token.id}_${flipped}`

    expect(verifyToken(tampered)).toBeNull()
  })

  it('rejects a revoked token', () => {
    const { token, plaintext } = mintToken(ownerId(), 'iPhone')
    expect(verifyToken(plaintext)).not.toBeNull()

    revokeToken(token.id)
    expect(verifyToken(plaintext)).toBeNull()
  })

  it('rejects an unknown id', () => {
    expect(verifyToken('vorn_00000000-0000-4000-8000-000000000000_deadbeef')).toBeNull()
  })

  it.each([
    ['empty', ''],
    ['wrong prefix', 'nope_id_secret'],
    ['no separator after the id', 'vorn_id'],
    ['empty id', 'vorn__secret'],
    ['empty secret', 'vorn_id_']
  ])('rejects a malformed token (%s)', (_label, raw) => {
    expect(verifyToken(raw)).toBeNull()
  })

  it('does not throw when a stored hash has the wrong length', () => {
    // timingSafeEqual throws on length mismatch. A truncated or hand-edited row
    // must read as a failed verification rather than as an exception on the
    // connection path, where it would surface to every client at once.
    const id = randomUUID()
    dbInsertDeviceToken({
      id,
      userId: ownerId(),
      name: 'corrupt row',
      tokenHash: 'not-a-sha256',
      createdAt: new Date().toISOString()
    })

    expect(() => verifyToken(`vorn_${id}_anything`)).not.toThrow()
    expect(verifyToken(`vorn_${id}_anything`)).toBeNull()
  })
})

describe('listTokens', () => {
  it('is empty on a fresh database', () => {
    expect(listTokens()).toEqual([])
  })

  it('returns tokens in creation order and reflects revocation', () => {
    const first = mintToken(ownerId(), 'first')
    const second = mintToken(ownerId(), 'second')

    expect(listTokens().map((t) => t.name)).toEqual(['first', 'second'])

    revokeToken(first.token.id)
    const listed = listTokens()
    expect(listed.find((t) => t.id === first.token.id)?.revokedAt).toBeTruthy()
    expect(listed.find((t) => t.id === second.token.id)?.revokedAt).toBeNull()
  })
})

describe('revokeToken', () => {
  it('reports false for an unknown id', () => {
    expect(revokeToken('not-a-token')).toBe(false)
  })

  it('reports false the second time, so a caller can tell it was already revoked', () => {
    const { token } = mintToken(ownerId(), 'iPhone')
    expect(revokeToken(token.id)).toBe(true)
    expect(revokeToken(token.id)).toBe(false)
  })
})

describe('touchLastSeen', () => {
  it('records a first sighting', () => {
    const { token } = mintToken(ownerId(), 'iPhone')
    expect(listTokens()[0].lastSeenAt).toBeNull()

    touchLastSeen(token.id)
    expect(listTokens()[0].lastSeenAt).toBeTruthy()
  })

  it('ignores an unknown id rather than throwing', () => {
    expect(() => touchLastSeen('not-a-token')).not.toThrow()
  })
})
