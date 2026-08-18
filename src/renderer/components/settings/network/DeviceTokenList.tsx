import { useState, useEffect, useCallback } from 'react'
import type { DeviceToken } from '../../../../shared/types'
import { CopyButton } from './shared'

/**
 * The devices allowed to connect, and the only place to make one.
 *
 * Before this, pairing a phone meant finding a terminal on the machine running the
 * server and typing `vorn-server token create`, which the panel could only tell you
 * about rather than do. That was tolerable while the server was always the machine
 * in front of you. It stopped being tolerable once the server could be elsewhere.
 */
export function DeviceTokenList() {
  const [tokens, setTokens] = useState<DeviceToken[] | null>(null)
  const [creating, setCreating] = useState(false)
  const [name, setName] = useState('')
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

  const create = async (): Promise<void> => {
    setError(null)
    try {
      const result = await window.api.createDeviceToken(name.trim() || 'Device')
      setMinted({ name: result.token.name, plaintext: result.plaintext })
      setName('')
      setCreating(false)
      await load()
    } catch (err) {
      console.error('[NetworkSettings] failed to create a device token:', err)
      setError('Could not create a token.')
    }
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

  const active = (tokens ?? []).filter((t) => !t.revokedAt)

  return (
    <div className="mt-3 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="flex items-center justify-between">
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium">
          Devices{active.length > 0 && ` · ${active.length}`}
        </div>
        {!creating && !minted && (
          <button
            onClick={() => setCreating(true)}
            className="text-xs text-gray-400 hover:text-white px-2 py-1 -mr-2 rounded-md hover:bg-white/[0.06] transition-colors"
          >
            Add device
          </button>
        )}
      </div>

      {/* The one moment the token is readable. Said plainly rather than left to be
          discovered after the panel closes. */}
      {minted && (
        <div className="mt-3 rounded-lg border border-amber-500/20 bg-amber-500/[0.04] p-3">
          <div className="text-sm font-medium text-gray-200">
            Token for &ldquo;{minted.name}&rdquo;
          </div>
          <p className="text-xs text-gray-500 mt-1 mb-2.5">
            Copy it now. Only its hash is stored, so this is the only time it can be shown.
          </p>
          <div className="flex items-center rounded-md bg-black/30 border border-white/[0.06] px-3 py-2">
            <code className="text-xs text-emerald-400 font-mono flex-1 truncate">
              {minted.plaintext}
            </code>
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

      {creating && (
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
              onClick={() => {
                setCreating(false)
                setName('')
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {error && <p className="text-xs text-amber-400/80 mt-2">{error}</p>}

      {tokens !== null && active.length === 0 && !creating && !minted && (
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
                    className="px-2 py-1 text-xs font-medium rounded-md text-red-300 border border-red-500/20 hover:bg-red-500/10 transition-colors"
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
