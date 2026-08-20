import { useCallback, useEffect, useState } from 'react'
import type { PairingRequest, ReachableUrls } from '../../../../shared/types'
import { SettingRow } from '../SettingRow'
import { QRCode128 } from './shared'

/**
 * Pair a phone by letting it look at this screen.
 *
 * The address QR above this one carries only an address, because a device
 * token never expires and authorises running terminals: a QR carrying one
 * would leave a permanent key on screen for anyone who photographs the
 * monitor. This one carries a code that dies in five minutes and works once.
 *
 * Scanning it is not enough on its own. Scanning proves someone saw the
 * screen, not that they were meant to, so the exchange stops here and waits
 * for a person to say yes to a named device.
 *
 * Rendered only while the server is reachable from elsewhere. Bound to
 * loopback there is nothing for a phone to connect to, and a code that cannot
 * be redeemed is worse than no offer at all.
 */

function useCountdown(expiresAt: number | null): number {
  const [remaining, setRemaining] = useState(0)

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

export function PairAPhone({ reachable }: { reachable: ReachableUrls | null }) {
  const [code, setCode] = useState<string | null>(null)
  const [expiresAt, setExpiresAt] = useState<number | null>(null)
  const [asking, setAsking] = useState<PairingRequest | null>(null)
  const [error, setError] = useState<string | null>(null)
  const remaining = useCountdown(expiresAt)
  // A code past its countdown is one the server has already forgotten, so it
  // stops being shown rather than being cleared by an effect that would make
  // rendering depend on a state update.
  const showingCode = code !== null && remaining > 0

  const stop = useCallback((): void => {
    setCode(null)
    setExpiresAt(null)
    setAsking(null)
  }, [])

  // A phone that asks while this panel is closed would otherwise wait on a
  // prompt nobody is going to see, so the pending list is read on mount too.
  useEffect(() => {
    let mounted = true
    void window.api.pendingPairings().then((pending) => {
      if (mounted && pending.length > 0) setAsking(pending[0])
    })
    return () => {
      mounted = false
    }
  }, [])

  useEffect(() => window.api.onPairingRequested((request) => setAsking(request)), [])

  useEffect(() => {
    return () => {
      // Leaving the panel puts the code away: it is only safe while watched.
      void window.api.cancelPairing()
    }
  }, [])

  if (!reachable?.remote || reachable.urls.length === 0) return null

  const start = async (): Promise<void> => {
    setError(null)
    try {
      const started = await window.api.startPairing()
      setCode(started.code)
      setExpiresAt(started.expiresAt)
    } catch {
      setError('Could not start pairing.')
    }
  }

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
    stop()
  }

  const cancel = async (): Promise<void> => {
    stop()
    await window.api.cancelPairing()
  }

  return (
    <SettingRow
      label="Pair a phone"
      description="Show a code the Vorn app can scan. It expires in five minutes."
    >
      <div className="w-full">
        {asking ? (
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/[0.06] p-3">
            <div className="text-sm text-white font-medium">A phone is asking to pair</div>
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
              Approving gives it permission to run terminals on this machine. Only approve a device
              you are holding.
            </p>
            <div className="mt-3 flex gap-2">
              <button
                onClick={() => void decide(true)}
                className="px-3 py-1.5 text-xs rounded-md bg-white text-black hover:bg-white/90 transition-colors font-medium"
              >
                Approve
              </button>
              <button
                onClick={() => void decide(false)}
                className="px-3 py-1.5 text-xs rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-gray-300 transition-colors"
              >
                Deny
              </button>
            </div>
          </div>
        ) : showingCode ? (
          <div className="flex items-start gap-3">
            <QRCode128
              url={`vorn://pair?url=${encodeURIComponent(reachable.urls[0])}&code=${code}`}
            />
            <div className="min-w-0">
              <div className="font-mono text-sm text-white tracking-wider">{code}</div>
              <div className="mt-1 text-xs text-gray-500 tabular-nums">
                Expires in {formatRemaining(remaining)}
              </div>
              <p className="mt-2 text-[11px] leading-4 text-gray-500">
                Scan it in the Vorn app, then approve the phone here.
              </p>
              <button
                onClick={() => void cancel()}
                className="mt-2 px-2 py-1 text-xs rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-gray-400 hover:text-white transition-colors"
              >
                Stop showing
              </button>
            </div>
          </div>
        ) : (
          <button
            onClick={() => void start()}
            className="px-3 py-1.5 text-xs rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-gray-300 hover:text-white transition-colors"
          >
            Pair a phone
          </button>
        )}
        {error ? <p className="mt-2 text-xs text-red-400">{error}</p> : null}
      </div>
    </SettingRow>
  )
}
