import { useState, useEffect, useCallback } from 'react'
import type { DeviceToken, PairingRequest, ReachableUrls } from '../../../../shared/types'
import { CopyButton, ScannableQRCode } from './shared'
import { addressKind, addressHost } from './address-kind'

/** Ticks once a second so a shown code says how long it has left. */
function useCountdown(expiresAt: number | null): number {
  // Seeded from the deadline rather than from zero. Effects run after paint, so
  // a zero start means the first frame of a freshly issued code reports no time
  // left, and anything reading that renders the expired state for a frame
  // before the first tick corrects it.
  const [remaining, setRemaining] = useState(() =>
    expiresAt === null ? 0 : Math.max(0, expiresAt - Date.now())
  )
  useEffect(() => {
    if (expiresAt === null) return
    const tick = (): void => setRemaining(Math.max(0, expiresAt - Date.now()))
    tick()
    const timer = setInterval(tick, 1000)
    return () => clearInterval(timer)
  }, [expiresAt])
  return remaining
}

function formatRemaining(ms: number): string {
  const total = Math.ceil(ms / 1000)
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/**
 * The devices allowed to connect, and the only place to make one.
 *
 * Before this, pairing a phone meant finding a terminal on the machine running the
 * server and typing `vorn-server token create`, which the panel could only tell you
 * about rather than do. That was tolerable while the server was always the machine
 * in front of you. It stopped being tolerable once the server could be elsewhere.
 *
 * Adding a device asks how, rather than offering two separate controls that both
 * add one. Showing a code is the good way and leads; a token to copy stays for
 * the machines a camera cannot reach, and for a device with no camera at all.
 */
export function DeviceTokenList({ reachable }: { reachable: ReachableUrls | null }) {
  const [tokens, setTokens] = useState<DeviceToken[] | null>(null)
  /**
   * How a device is being added, if one is. Adding goes straight to a code:
   * that is what nearly every device wants, and asking first is a question
   * with a predictable answer. Typing is reachable from there.
   */
  const [adding, setAdding] = useState<'scan' | 'token' | null>(null)
  const [name, setName] = useState('')
  const [code, setCode] = useState<string | null>(null)
  const [expiresAt, setExpiresAt] = useState<number | null>(null)
  const [asking, setAsking] = useState<PairingRequest | null>(null)
  /**
   * Which address the code points at.
   *
   * Not a detail: a phone can only reach the machine over a network it is on,
   * so a code carrying the tailnet address is unusable from a phone on the
   * wifi, and the desktop has no way to know which one that is. Tailscale
   * sorts first because it is the only encrypted option, which made it the
   * silent default and left LAN pairing with no way in at all.
   */
  const [chosenUrl, setChosenUrl] = useState<string | null>(null)
  /** Shown once, then unrecoverable: only its hash is stored. */
  const [minted, setMinted] = useState<{ name: string; plaintext: string } | null>(null)
  const [confirmRevokeId, setConfirmRevokeId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    try {
      setTokens(await window.api.listDeviceTokens())
    } catch (err) {
      console.error('[NetworkSettings] failed to list device tokens:', err)
      setError('Could not read the device list.')
    }
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: read the device list once on open
    load()
  }, [load])

  useEffect(() => window.api.onPairingRequested((request) => setAsking(request)), [])

  // Approving does not create the token; the phone collecting it does, a moment
  // later. Reloading on approval therefore looked at a list that had nothing
  // new in it yet, and the device only turned up if the panel was left and
  // reopened.
  useEffect(() => window.api.onPairingCollected(() => void load()), [load])

  useEffect(() => {
    return () => {
      // Closing the panel puts any code away: it is only safe while watched.
      void window.api.cancelPairing()
    }
  }, [])

  const create = async (): Promise<void> => {
    setError(null)
    try {
      const result = await window.api.createDeviceToken(name.trim() || 'Device')
      setMinted({ name: result.token.name, plaintext: result.plaintext })
      setName('')
      setAdding(null)
      await load()
    } catch (err) {
      console.error('[NetworkSettings] failed to create a device token:', err)
      setError('Could not create a token.')
    }
  }

  const startScan = async (): Promise<void> => {
    setError(null)
    try {
      const started = await window.api.startPairing()
      setCode(started.code)
      setExpiresAt(started.expiresAt)
      setAdding('scan')
    } catch {
      setError('Could not start pairing.')
    }
  }

  const closeAdding = useCallback((): void => {
    setAdding(null)
    setCode(null)
    setExpiresAt(null)
    setAsking(null)
    setName('')
    setChosenUrl(null)
    // Retired, not merely hidden. Taking a code off screen while the server
    // still honours it is the opposite of what stopping means: whoever
    // photographed it a moment ago could still use it for the rest of the five
    // minutes, after it was explicitly put away.
    void window.api.cancelPairing()
  }, [])

  const decide = async (approve: boolean): Promise<void> => {
    if (!asking) return
    const request = asking
    setAsking(null)
    try {
      if (approve) await window.api.approvePairing(request.requestId)
      else await window.api.denyPairing(request.requestId)
    } catch {
      setError('Could not answer that request.')
    }
    closeAdding()
    await load()
  }

  const revoke = async (id: string): Promise<void> => {
    setConfirmRevokeId(null)
    try {
      await window.api.revokeDeviceToken(id)
      await load()
    } catch (err) {
      console.error('[NetworkSettings] failed to revoke a device token:', err)
      setError('Could not revoke that token.')
    }
  }

  const remaining = useCountdown(expiresAt)
  // Derived rather than cleared by an effect: a code past its countdown is one
  // the server has already forgotten.
  const showingCode = code !== null && remaining > 0
  // A phone cannot reach a server bound to loopback, so a code it could never
  // redeem is not worth offering.
  const canScan = reachable?.remote === true && (reachable?.urls.length ?? 0) > 0

  const urls = reachable?.urls ?? []
  const pairUrl = chosenUrl && urls.includes(chosenUrl) ? chosenUrl : (urls[0] ?? null)

  const active = (tokens ?? []).filter((t) => !t.revokedAt)

  return (
    <div className="mt-3 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="flex items-center justify-between">
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium">
          Devices{active.length > 0 && ` · ${active.length}`}
        </div>
        {!adding && !minted && (
          <button
            onClick={() => void (canScan ? startScan() : setAdding('token'))}
            className="text-xs text-gray-400 hover:text-white px-2 py-1 -mr-2 rounded-md hover:bg-white/[0.06] transition-colors"
          >
            Add device
          </button>
        )}
      </div>

      {/* The one moment the token is readable. Said plainly rather than left to be
          discovered after the panel closes. */}
      {minted && (
        <div className="mt-3 rounded-lg border border-bronzo/30 bg-surface-raised p-3">
          <div className="text-sm font-medium text-gray-200">
            Token for &ldquo;{minted.name}&rdquo;
          </div>
          <p className="text-xs text-gray-500 mt-1 mb-2.5">
            Copy it now. Only its hash is stored, so this is the only time it can be shown.
          </p>
          <div className="flex items-center rounded-md bg-black/30 border border-white/[0.06] px-3 py-2">
            <code className="text-xs text-ink font-mono flex-1 truncate">{minted.plaintext}</code>
            <CopyButton text={minted.plaintext} />
          </div>
          <button
            onClick={() => setMinted(null)}
            className="mt-2.5 text-xs text-gray-500 hover:text-white px-2 py-1 -ml-2 rounded-md hover:bg-white/[0.06] transition-colors"
          >
            Done
          </button>
        </div>
      )}

      {asking && (
        <div className="mt-3 rounded-lg border border-bronzo/40 bg-surface-raised p-3">
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 rounded-full bg-bronzo flex-shrink-0" />
            <div className="text-sm text-gray-200 font-medium">A phone is asking to pair</div>
          </div>
          <dl className="mt-2 space-y-1 text-xs">
            <div className="flex justify-between gap-3">
              <dt className="text-gray-500">Device</dt>
              <dd className="text-gray-300 font-mono truncate">{asking.deviceName}</dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-gray-500">Address</dt>
              <dd className="text-gray-300 font-mono">{asking.address}</dd>
            </div>
          </dl>
          <p className="mt-2 text-[11px] leading-4 text-gray-500">
            This lets it run terminals on this machine. Only approve a phone you are holding.
          </p>
          <div className="flex gap-2 mt-2.5">
            <button
              onClick={() => void decide(true)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Approve
            </button>
            <button
              onClick={() => void decide(false)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Deny
            </button>
          </div>
        </div>
      )}

      {adding === 'scan' && !asking && showingCode && pairUrl && (
        <div className="mt-3 flex items-start gap-4">
          <ScannableQRCode url={`vorn://pair?url=${encodeURIComponent(pairUrl)}&code=${code}`} />
          <div className="min-w-0 flex-1">
            <p className="text-sm text-gray-200">Scan this in the Vorn app.</p>
            <p className="mt-1 text-xs text-gray-500">
              Then approve the phone here. It will ask by name.
            </p>

            {urls.length > 1 && (
              <div className="mt-3">
                <div className="text-[10px] text-gray-600 uppercase tracking-wider mb-1">
                  Address in the code
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {urls.map((url) => {
                    const chosen = url === pairUrl
                    return (
                      <button
                        key={url}
                        onClick={() => setChosenUrl(url)}
                        title={url}
                        className={`px-2 py-1 font-mono text-[11px] rounded-md border transition-colors ${
                          chosen
                            ? 'border-white/20 bg-white/[0.08] text-gray-200'
                            : 'border-white/[0.06] text-gray-500 hover:text-white hover:bg-white/[0.06]'
                        }`}
                      >
                        {addressHost(url)}
                      </button>
                    )
                  })}
                </div>
                <p className="mt-1.5 text-[11px] text-gray-600">
                  {addressKind(pairUrl) === 'tailnet'
                    ? 'Encrypted. The phone must be on your tailnet.'
                    : 'Unencrypted. The phone must be able to reach this address.'}
                </p>
              </div>
            )}
            <div className="mt-3 text-[10px] text-gray-600 uppercase tracking-wider">
              Or type this code
            </div>
            <div className="font-mono text-base text-gray-200 tracking-widest mt-0.5">{code}</div>
            <div className="mt-1 text-[11px] text-gray-500 tabular-nums">
              Expires in {formatRemaining(remaining)}
            </div>
            <div className="mt-4 flex flex-col items-start gap-1">
              <button
                onClick={() => {
                  void window.api.cancelPairing()
                  setCode(null)
                  setExpiresAt(null)
                  setAdding('token')
                }}
                className="px-2 py-1 -ml-2 text-xs rounded-md text-gray-400 hover:text-white hover:bg-white/[0.06] transition-colors"
              >
                Pair manually instead
              </button>
              <button
                onClick={closeAdding}
                className="px-2 py-1 -ml-2 text-xs rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {adding === 'scan' && !asking && !showingCode && (
        <div className="mt-3">
          <p className="text-xs text-gray-500">That code expired.</p>
          <div className="flex gap-2 mt-2">
            <button
              onClick={() => void startScan()}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Show another
            </button>
            <button
              onClick={closeAdding}
              className="px-3 py-1.5 text-xs rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {adding === 'token' && !asking && (
        <div className="mt-3">
          <label className="block text-xs text-gray-400 mb-1.5" htmlFor="device-token-name">
            What is this device?
          </label>
          <input
            id="device-token-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void create()
            }}
            placeholder="My phone"
            autoFocus
            className="w-full rounded-md bg-white/[0.04] border border-white/[0.08] px-3 py-2 text-sm text-gray-200 placeholder:text-gray-600 outline-none focus:border-white/20"
          />
          <div className="flex gap-2 mt-2.5">
            <button
              onClick={() => void create()}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Create token
            </button>
            <button
              onClick={closeAdding}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {error && <p className="text-xs text-danger mt-2">{error}</p>}

      {tokens !== null && active.length === 0 && !adding && !minted && (
        <p className="text-xs text-gray-600 mt-2">
          No devices yet. Add one, then open the address above on it.
        </p>
      )}

      {active.length > 0 && (
        <div className="mt-1">
          {active.map((token) => (
            <div
              key={token.id}
              className="flex items-center gap-3 py-2 border-t border-white/[0.04] first:border-t-0"
            >
              <div className="flex-1 min-w-0">
                <div className="text-sm text-gray-200 truncate">{token.name}</div>
                <div className="text-[10px] text-gray-600">
                  {token.lastSeenAt
                    ? `Last seen ${new Date(token.lastSeenAt).toLocaleString()}`
                    : 'Never connected'}
                </div>
              </div>
              {confirmRevokeId === token.id ? (
                <div className="flex gap-1.5 shrink-0">
                  <button
                    onClick={() => void revoke(token.id)}
                    className="px-2 py-1 text-xs font-medium rounded-md text-danger border border-danger/25 hover:bg-danger/10 transition-colors"
                  >
                    Revoke
                  </button>
                  <button
                    onClick={() => setConfirmRevokeId(null)}
                    className="px-2 py-1 text-xs rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => setConfirmRevokeId(token.id)}
                  className="shrink-0 text-xs text-gray-500 hover:text-white px-2 py-1 rounded-md hover:bg-white/[0.06] transition-colors"
                >
                  Remove
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
