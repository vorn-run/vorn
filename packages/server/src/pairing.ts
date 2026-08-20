import crypto from 'node:crypto'
import { mintOwnerToken } from './token-manager'
import log from './logger'
import type { PairingRequest } from '@vornrun/shared/types'

/**
 * Pairing a phone by showing it a code.
 *
 * A device token never expires and authorises running terminals, so a QR
 * carrying one is a permanent key on a screen: anyone who photographs the
 * monitor keeps it, and revoking by hand is the only undo. What the screen
 * shows instead is a code that dies in five minutes and works once, which the
 * phone trades for a token only after a person approves the exchange here.
 *
 * Held in memory rather than the database, deliberately. A pairing code that
 * survived a restart would be a code nobody is watching for, and the whole
 * point is that it is only good while someone is looking at it.
 */

/**
 * How long a code is worth showing.
 *
 * NIST SP 800-63B calls an out-of-band authentication invalid after ten
 * minutes. Five is inside that and still longer than it takes to find a phone
 * and unlock it.
 */
export const CODE_TTL_MS = 5 * 60_000

/**
 * How long an approval stays collectable.
 *
 * The phone polls within seconds, so this only covers a phone that lost its
 * network between scanning and collecting. No token exists until it is
 * collected, so an approval nobody claims leaves nothing behind.
 */
export const APPROVAL_TTL_MS = 5 * 60_000

/**
 * How many wrong codes may be offered before the code is abandoned.
 *
 * NIST requires guessing to be capped; nothing else in this server rate limits
 * anything. Forty bits of entropy makes ten guesses hopeless on its own, and
 * the cap means a client cannot grind at it either.
 */
export const MAX_ATTEMPTS = 10

/**
 * Crockford's alphabet: no I, L, O or U, so nothing reads as a digit and no
 * accidental words form. Eight characters is forty bits, twice what NIST asks.
 */
const ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'
const CODE_LENGTH = 8

interface ActiveCode {
  code: string
  expiresAt: number
  attempts: number
  /** Set by the first successful redeem: a code is good for one exchange. */
  spent: boolean
}

interface StoredRequest extends PairingRequest {
  decidedAt: number | null
  /**
   * The code this request came from.
   *
   * A request outlives the code that made it: it can be approved, sit
   * uncollected while a phone is off the network, and be collected after the
   * owner has given up and started showing a different code. Without knowing
   * which code it belongs to, collecting it retires whichever code happens to
   * be current, and the screen goes on displaying a live countdown for one the
   * server has already forgotten.
   */
  code: string
}

let active: ActiveCode | null = null
const requests = new Map<string, StoredRequest>()

/**
 * Five random bytes are exactly forty bits, so the code is eight five-bit
 * groups with nothing left over. Taken this way rather than by `% 32` per byte,
 * which would make the first eight letters of the alphabet fractionally more
 * likely than the rest.
 */
function generateCode(): string {
  const bytes = crypto.randomBytes(5)
  let bits = 0n
  for (const byte of bytes) bits = (bits << 8n) | BigInt(byte)
  let out = ''
  for (let i = 0; i < CODE_LENGTH; i++) {
    const shift = BigInt((CODE_LENGTH - 1 - i) * 5)
    out += ALPHABET[Number((bits >> shift) & 31n)]
  }
  return out
}

/** `ABCD-EFGH`, because a code that has to be read aloud or typed usually is. */
export function formatCode(code: string): string {
  return `${code.slice(0, 4)}-${code.slice(4)}`
}

/** Accepts what a person can type: spacing, dashes and case are all forgiven. */
function normaliseCode(raw: unknown): string {
  return typeof raw === 'string' ? raw.toUpperCase().replace(/[^0-9A-Z]/g, '') : ''
}

/**
 * Drop what can no longer be acted on.
 *
 * Two reasons, and the second is the one that bites. A request nobody collects
 * used to sit in the map for the life of the process, which grows without
 * bound on a long-running server. And an expired request that is merely
 * filtered out of a list is still *there*: the approval prompt on the desktop
 * holds an id, so a prompt left on screen stayed answerable hours after its
 * code died, and answering it would still hand over a token.
 */
function prune(now: number): void {
  for (const [id, request] of requests) {
    const waitedTooLong = request.status === 'pending' && now - request.askedAt >= CODE_TTL_MS
    const decidedTooLongAgo =
      request.decidedAt !== null && now - request.decidedAt >= APPROVAL_TTL_MS
    if (waitedTooLong || decidedTooLongAgo) requests.delete(id)
  }
}

function expired(now: number): boolean {
  return active !== null && now >= active.expiresAt
}

/**
 * Begin pairing. Any code already showing is abandoned, so the screen and the
 * server never disagree about which code is live.
 */
export function startPairing(): { code: string; expiresAt: number } {
  const now = Date.now()
  active = { code: generateCode(), expiresAt: now + CODE_TTL_MS, attempts: 0, spent: false }
  log.info('[pairing] code issued')
  return { code: formatCode(active.code), expiresAt: active.expiresAt }
}

/** Stop showing a code. Pending requests are dropped with it. */
export function cancelPairing(): void {
  active = null
  for (const [id, request] of requests) {
    if (request.status === 'pending') requests.delete(id)
  }
}

export type RedeemResult =
  | { ok: true; requestId: string }
  | { ok: false; reason: 'unknown' | 'expired' | 'spent' | 'throttled' }

/**
 * Offer a code. A match records a request for someone to approve; it does not
 * hand anything over.
 *
 * A wrong code and no code at all answer the same way, so the reply cannot be
 * used to learn whether pairing is open.
 */
export function redeemCode(rawCode: unknown, deviceName: unknown, address: string): RedeemResult {
  const now = Date.now()
  prune(now)
  if (!active || expired(now)) return { ok: false, reason: active ? 'expired' : 'unknown' }
  if (active.spent) return { ok: false, reason: 'spent' }
  if (active.attempts >= MAX_ATTEMPTS) return { ok: false, reason: 'throttled' }

  const offered = normaliseCode(rawCode)
  const expectedBytes = Buffer.from(active.code, 'utf8')
  const offeredBytes = Buffer.from(offered, 'utf8')
  const matches =
    offeredBytes.length === expectedBytes.length &&
    crypto.timingSafeEqual(offeredBytes, expectedBytes)

  if (!matches) {
    active.attempts += 1
    log.warn({ attempts: active.attempts }, '[pairing] wrong code offered')
    return { ok: false, reason: active.attempts >= MAX_ATTEMPTS ? 'throttled' : 'unknown' }
  }

  active.spent = true
  const name = typeof deviceName === 'string' && deviceName.trim() ? deviceName.trim() : 'Phone'
  const requestId = crypto.randomUUID()
  requests.set(requestId, {
    requestId,
    deviceName: name.slice(0, 64),
    address,
    askedAt: now,
    status: 'pending',
    decidedAt: null,
    code: active.code
  })
  log.info({ requestId, deviceName: name }, '[pairing] a device asked to pair')
  return { ok: true, requestId }
}

/** Everything waiting on a person, so the UI can rehydrate rather than rely on having heard the push. */
export function pendingRequests(): PairingRequest[] {
  const now = Date.now()
  prune(now)
  return [...requests.values()]
    .filter((r) => r.status === 'pending' && now - r.askedAt < CODE_TTL_MS)
    .map(({ decidedAt: _decidedAt, code: _code, ...request }) => request)
}

export function approveRequest(requestId: unknown): boolean {
  prune(Date.now())
  const request = typeof requestId === 'string' ? requests.get(requestId) : undefined
  if (!request || request.status !== 'pending') return false
  request.status = 'approved'
  request.decidedAt = Date.now()
  log.info({ requestId: request.requestId }, '[pairing] approved')
  return true
}

export function denyRequest(requestId: unknown): boolean {
  prune(Date.now())
  const request = typeof requestId === 'string' ? requests.get(requestId) : undefined
  if (!request || request.status !== 'pending') return false
  request.status = 'denied'
  request.decidedAt = Date.now()
  log.info({ requestId: request.requestId }, '[pairing] denied')
  return true
}

export type PollResult =
  | { status: 'pending' }
  | { status: 'denied' }
  | { status: 'expired' }
  | { status: 'approved'; token: string; name: string }

/**
 * Ask what came of a request, and collect the token if one was granted.
 *
 * The token is minted here rather than on approval, so a request nobody
 * collects leaves no credential behind at all. It is handed out once: the
 * request is marked collected in the same breath, and a replayed id gets
 * nothing.
 */
export function pollRequest(requestId: unknown, machineName: string): PollResult {
  const now = Date.now()
  prune(now)
  const request = typeof requestId === 'string' ? requests.get(requestId) : undefined
  if (!request) return { status: 'expired' }

  if (request.status === 'denied') return { status: 'denied' }
  if (request.status === 'collected') return { status: 'expired' }

  if (request.status === 'pending') {
    if (now - request.askedAt >= CODE_TTL_MS) {
      requests.delete(request.requestId)
      return { status: 'expired' }
    }
    return { status: 'pending' }
  }

  if (request.decidedAt !== null && now - request.decidedAt >= APPROVAL_TTL_MS) {
    requests.delete(request.requestId)
    return { status: 'expired' }
  }

  const minted = mintOwnerToken(request.deviceName)
  request.status = 'collected'
  // Only if this is still the code that produced the request. A late collect
  // must not retire a code the owner is currently showing for someone else.
  if (active?.code === request.code) active = null
  log.info({ requestId: request.requestId }, '[pairing] token collected')
  return { status: 'approved', token: minted.plaintext, name: machineName }
}

/** Only for tests, which would otherwise leak a live code between cases. */
export function resetPairing(): void {
  active = null
  requests.clear()
}
