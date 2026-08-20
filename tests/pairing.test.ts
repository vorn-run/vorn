import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const mintOwnerTokenMock = vi.hoisted(() =>
  vi.fn((name: string) => ({
    token: { id: 'tok-1', name, createdAt: '2026-01-01T00:00:00.000Z' },
    plaintext: `vorn_tok-1_secret-for-${name}`
  }))
)

vi.mock('../packages/server/src/token-manager', () => ({ mintOwnerToken: mintOwnerTokenMock }))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

import {
  APPROVAL_TTL_MS,
  CODE_TTL_MS,
  MAX_ATTEMPTS,
  approveRequest,
  cancelPairing,
  denyRequest,
  pendingRequests,
  pollRequest,
  redeemCode,
  resetPairing,
  startPairing
} from '../packages/server/src/pairing'

const FROM = '192.168.0.31'

/** Redeem the code that is actually showing, which most cases need first. */
function scan(code: string, name = 'iPhone'): string {
  const result = redeemCode(code, name, FROM)
  if (!result.ok) throw new Error(`expected the code to be accepted, got ${result.reason}`)
  return result.requestId
}

beforeEach(() => {
  vi.useFakeTimers()
  mintOwnerTokenMock.mockClear()
  resetPairing()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('the code itself', () => {
  it('carries the entropy it claims', () => {
    // Eight characters of a 32 letter alphabet is forty bits. NIST asks twenty.
    const { code } = startPairing()

    expect(code).toMatch(/^[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/)
  })

  it('differs every time it is issued', () => {
    const seen = new Set<string>()
    for (let i = 0; i < 50; i++) seen.add(startPairing().code)

    expect(seen.size).toBe(50)
  })

  it('forgives the spacing and case a person types', () => {
    const { code } = startPairing()

    expect(redeemCode(code.replace('-', '').toLowerCase(), 'iPhone', FROM).ok).toBe(true)
  })
})

describe('a code that has run out', () => {
  it('is refused once it expires', () => {
    const { code } = startPairing()

    vi.advanceTimersByTime(CODE_TTL_MS)

    expect(redeemCode(code, 'iPhone', FROM)).toEqual({ ok: false, reason: 'expired' })
  })

  it('still works a moment before', () => {
    const { code } = startPairing()

    vi.advanceTimersByTime(CODE_TTL_MS - 1)

    expect(redeemCode(code, 'iPhone', FROM).ok).toBe(true)
  })

  it('is spent by one exchange, so a photograph cannot be used twice', () => {
    const { code } = startPairing()
    scan(code)

    expect(redeemCode(code, 'Another phone', FROM)).toEqual({ ok: false, reason: 'spent' })
  })

  it('stops being offered once pairing is cancelled', () => {
    const { code } = startPairing()

    cancelPairing()

    expect(redeemCode(code, 'iPhone', FROM).ok).toBe(false)
  })
})

describe('guessing', () => {
  it('refuses a wrong code', () => {
    startPairing()

    expect(redeemCode('AAAA-AAAA', 'iPhone', FROM).ok).toBe(false)
  })

  it('answers a wrong code and no code alike, so neither reveals the other', () => {
    const withNoPairingOpen = redeemCode('AAAA-AAAA', 'iPhone', FROM)
    startPairing()
    const withPairingOpen = redeemCode('BBBB-BBBB', 'iPhone', FROM)

    expect(withNoPairingOpen).toEqual({ ok: false, reason: 'unknown' })
    expect(withPairingOpen).toEqual({ ok: false, reason: 'unknown' })
  })

  it('gives up on the code after too many wrong guesses', () => {
    const { code } = startPairing()
    for (let i = 0; i < MAX_ATTEMPTS; i++) redeemCode('AAAA-AAAA', 'iPhone', FROM)

    // Even the right code, because the code being ground at is now suspect.
    expect(redeemCode(code, 'iPhone', FROM)).toEqual({ ok: false, reason: 'throttled' })
  })

  it('does not spend an attempt on the right code', () => {
    const { code } = startPairing()
    for (let i = 0; i < MAX_ATTEMPTS - 1; i++) redeemCode('AAAA-AAAA', 'iPhone', FROM)

    expect(redeemCode(code, 'iPhone', FROM).ok).toBe(true)
  })
})

describe('waiting on a person', () => {
  it('hands over nothing until someone approves', () => {
    const { code } = startPairing()
    const requestId = scan(code)

    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'pending' })
    expect(mintOwnerTokenMock).not.toHaveBeenCalled()
  })

  it('shows what is asking, so the person can recognise it', () => {
    const { code } = startPairing()
    scan(code, 'Javier iPhone')

    expect(pendingRequests()).toEqual([
      expect.objectContaining({ deviceName: 'Javier iPhone', address: FROM, status: 'pending' })
    ])
  })

  it('mints the token only when it is collected', () => {
    const { code } = startPairing()
    const requestId = scan(code)
    approveRequest(requestId)
    expect(mintOwnerTokenMock).not.toHaveBeenCalled()

    const result = pollRequest(requestId, 'javiers-mac')

    expect(result).toEqual({
      status: 'approved',
      token: 'vorn_tok-1_secret-for-iPhone',
      name: 'javiers-mac'
    })
    expect(mintOwnerTokenMock).toHaveBeenCalledTimes(1)
  })

  it('hands the token over once and never again', () => {
    const { code } = startPairing()
    const requestId = scan(code)
    approveRequest(requestId)
    pollRequest(requestId, 'mac')

    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'expired' })
    expect(mintOwnerTokenMock).toHaveBeenCalledTimes(1)
  })

  it('leaves no credential behind when an approval is never collected', () => {
    const { code } = startPairing()
    const requestId = scan(code)
    approveRequest(requestId)

    vi.advanceTimersByTime(APPROVAL_TTL_MS)

    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'expired' })
    expect(mintOwnerTokenMock).not.toHaveBeenCalled()
  })

  it('is final once denied, and yields nothing', () => {
    const { code } = startPairing()
    const requestId = scan(code)

    expect(denyRequest(requestId)).toBe(true)
    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'denied' })
    expect(approveRequest(requestId)).toBe(false)
    expect(mintOwnerTokenMock).not.toHaveBeenCalled()
  })

  it('cannot be approved twice', () => {
    const { code } = startPairing()
    const requestId = scan(code)

    expect(approveRequest(requestId)).toBe(true)
    expect(approveRequest(requestId)).toBe(false)
  })

  it('expires a request nobody answered', () => {
    const { code } = startPairing()
    const requestId = scan(code)

    vi.advanceTimersByTime(CODE_TTL_MS)

    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'expired' })
    expect(pendingRequests()).toEqual([])
  })

  it('knows nothing of an id it never issued', () => {
    expect(pollRequest('made-up', 'mac')).toEqual({ status: 'expired' })
    expect(approveRequest('made-up')).toBe(false)
    expect(denyRequest('made-up')).toBe(false)
  })

  it('names the token after the device, so the list says which phone', () => {
    const { code } = startPairing()
    const requestId = scan(code, 'Javier iPhone')
    approveRequest(requestId)
    pollRequest(requestId, 'mac')

    expect(mintOwnerTokenMock).toHaveBeenCalledWith('Javier iPhone')
  })
})

describe('one pairing not disturbing the next', () => {
  it('a late collect leaves the code now on screen alive', () => {
    // The case the approval window exists for: a phone that lost its network
    // between scanning and collecting. Meanwhile the owner gave up and showed
    // a new code, which must still work.
    const first = startPairing()
    const stranded = scan(first.code)
    approveRequest(stranded)

    const second = startPairing()
    pollRequest(stranded, 'mac')

    expect(redeemCode(second.code, 'Another phone', FROM).ok).toBe(true)
  })

  it('still retires the code it belongs to', () => {
    const { code } = startPairing()
    const requestId = scan(code)
    approveRequest(requestId)

    pollRequest(requestId, 'mac')

    // Nothing is showing now, so nothing can be offered.
    expect(redeemCode(code, 'Another phone', FROM).ok).toBe(false)
  })
})

describe('not keeping what can no longer be used', () => {
  it('forgets a request nobody ever collected', () => {
    // Otherwise every abandoned pairing stays in memory for the life of the
    // process, which on a machine left running is forever.
    const { code } = startPairing()
    scan(code)
    expect(pendingRequests()).toHaveLength(1)

    vi.advanceTimersByTime(CODE_TTL_MS)

    expect(pendingRequests()).toHaveLength(0)
  })

  it('refuses to approve a request whose code has died', () => {
    // The prompt on the desktop holds an id, so one left on screen was still
    // answerable hours later — and answering it still handed over a token.
    const { code } = startPairing()
    const requestId = scan(code)

    vi.advanceTimersByTime(CODE_TTL_MS)

    expect(approveRequest(requestId)).toBe(false)
    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'expired' })
  })

  it('refuses to deny one too, rather than reviving it', () => {
    const { code } = startPairing()
    const requestId = scan(code)

    vi.advanceTimersByTime(CODE_TTL_MS)

    expect(denyRequest(requestId)).toBe(false)
  })

  it('forgets a collected request once its window has passed', () => {
    const { code } = startPairing()
    const requestId = scan(code)
    approveRequest(requestId)
    pollRequest(requestId, 'mac')

    vi.advanceTimersByTime(APPROVAL_TTL_MS)
    // Nothing to find, rather than a spent entry kept forever.
    expect(pollRequest(requestId, 'mac')).toEqual({ status: 'expired' })
  })
})
