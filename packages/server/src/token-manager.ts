import crypto, { randomUUID } from 'node:crypto'
import type { DeviceToken } from '@vornrun/shared/types'
import {
  dbGetOwnerUser,
  dbInsertDeviceToken,
  dbGetDeviceTokenSecret,
  dbListDeviceTokens,
  dbHasDeviceTokens,
  dbRevokeDeviceToken,
  dbTouchDeviceToken
} from './database'
import log from './logger'

/**
 * Device tokens: the credential a non-local client presents to reach this
 * server.
 *
 * Opaque and hashed at rest rather than signed. Revocation then costs one row
 * update instead of a blocklist, there is no signing key to rotate or leak, and
 * no library to add. A signed token would buy stateless verification across
 * many servers, which is not the shape of the problem — one server owns its
 * own sessions.
 *
 * Nothing calls `verifyToken` yet. The socket boundary that will is a separate
 * change, kept apart deliberately: it is the one edit in this area that can
 * stop the app from starting.
 */

const PREFIX = 'vorn'
const SECRET_BYTES = 32

/** `vorn_<id>_<secret>` — see `parseToken` for why the id travels in the clear. */
export interface MintedToken {
  token: DeviceToken
  /**
   * The only time the plaintext exists. Not stored, not recoverable, and the
   * caller is responsible for showing it once and then dropping it.
   */
  plaintext: string
}

/**
 * Compare two secrets without leaking their contents through timing.
 *
 * `timingSafeEqual` throws on a length mismatch, which any wrong-length guess
 * produces — so the length is checked first and a bad credential reads as a
 * failed comparison rather than an exception on the connection path.
 */
export function constantTimeEqual(a: Buffer, b: Buffer): boolean {
  if (a.length !== b.length) return false
  return crypto.timingSafeEqual(a, b)
}

/** Raw digest. Stored as hex; compared as bytes. */
function sha256(value: string): Buffer {
  return crypto.createHash('sha256').update(value, 'utf8').digest()
}

/**
 * The id half is not a secret and is sent in the clear so verification is a
 * primary-key lookup. Hashing every row to find a match would make each
 * connection cost a full table scan.
 *
 * Split on the *first* separator after the id rather than on every `_`. The
 * secret is base64url, whose alphabet includes `_`, so splitting the whole
 * string would reject any token whose random bytes happened to encode one —
 * intermittently, for a large share of minted tokens. The id is a UUID and
 * contains no `_`, so the first separator after the prefix is unambiguous.
 */
function parseToken(raw: string): { id: string; secret: string } | null {
  const head = `${PREFIX}_`
  if (!raw.startsWith(head)) return null

  const rest = raw.slice(head.length)
  const sep = rest.indexOf('_')
  if (sep <= 0) return null

  // `id` is non-empty because `sep <= 0` already returned.
  const secret = rest.slice(sep + 1)
  if (!secret) return null

  return { id: rest.slice(0, sep), secret }
}

/**
 * Mint a token for a user. Returns the plaintext exactly once; only its hash
 * reaches the database.
 */
export function mintToken(userId: string, name: string): MintedToken {
  const id = randomUUID()
  const secret = crypto.randomBytes(SECRET_BYTES).toString('base64url')
  const createdAt = new Date().toISOString()

  const token: DeviceToken = {
    id,
    userId,
    name,
    createdAt,
    lastSeenAt: null,
    revokedAt: null
  }

  dbInsertDeviceToken({ id, userId, name, createdAt, tokenHash: sha256(secret).toString('hex') })
  log.info({ tokenId: id, name }, '[token] minted device token')

  return { token, plaintext: `${PREFIX}_${id}_${secret}` }
}

/**
 * Mint a token for the seeded owner. Convenience for first-run, where there is
 * exactly one user and asking which one would be noise.
 */
export function mintOwnerToken(name: string): MintedToken {
  const owner = dbGetOwnerUser()
  if (!owner) {
    throw new Error('No owner user found. The database may not have been migrated.')
  }
  return mintToken(owner.id, name)
}

/**
 * Resolve a presented token to its owner, or null if it is malformed, unknown,
 * tampered with, or revoked.
 *
 * Returning a single null for every failure is deliberate: a caller that could
 * tell "no such token" from "wrong secret" would leak which ids exist.
 */
export function verifyToken(raw: string): { userId: string; tokenId: string } | null {
  const parsed = parseToken(raw)
  if (!parsed) return null

  const record = dbGetDeviceTokenSecret(parsed.id)
  if (!record) return null
  if (record.revokedAt) return null

  // Compare the 32 raw digest bytes rather than 64 characters of hex — one less
  // encoding pass on what will become the per-connection auth path.
  const presented = sha256(parsed.secret)
  const stored = Buffer.from(record.tokenHash, 'hex')
  // timingSafeEqual throws on length mismatch, which a corrupt or hand-edited
  // row can produce. Compare lengths first so a bad row reads as a failed
  // verification rather than an exception on the connection path.
  if (presented.length !== stored.length) return null
  if (!crypto.timingSafeEqual(presented, stored)) return null

  return { userId: record.userId, tokenId: record.id }
}

export function listTokens(): DeviceToken[] {
  return dbListDeviceTokens()
}

/** Whether this data directory has any token at all. */
export function hasTokens(): boolean {
  return dbHasDeviceTokens()
}

/** False when the id is unknown or the token was already revoked. */
export function revokeToken(id: string): boolean {
  const revoked = dbRevokeDeviceToken(id, new Date().toISOString())
  if (revoked) log.info({ tokenId: id }, '[token] revoked device token')
  return revoked
}

export function touchLastSeen(id: string): void {
  dbTouchDeviceToken(id, new Date().toISOString())
}
