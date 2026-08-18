import { useState, useEffect, useCallback } from 'react'
import QRCode from 'qrcode'
import { useAppStore } from '../../stores'
import { TailscaleStatus, ReachableUrls, DeviceToken } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'

// ─── OS Icons ────────────────────────────────────────────────────

function OsIcon({ os }: { os: string }) {
  const lower = os.toLowerCase()
  if (lower === 'macos' || lower === 'darwin') {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      >
        <rect x="3" y="4" width="18" height="12" rx="2" />
        <path d="M8 20h8M12 16v4" />
      </svg>
    )
  }
  if (lower === 'ios') {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      >
        <rect x="7" y="2" width="10" height="20" rx="2" />
        <circle cx="12" cy="18" r="1" fill="currentColor" />
      </svg>
    )
  }
  if (lower === 'android') {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      >
        <rect x="7" y="2" width="10" height="20" rx="2" />
        <line x1="9" y1="18" x2="15" y2="18" />
      </svg>
    )
  }
  if (lower === 'linux') {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      >
        <rect x="4" y="2" width="16" height="20" rx="2" />
        <circle cx="8" cy="6" r="1" fill="currentColor" />
      </svg>
    )
  }
  if (lower === 'windows') {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
      >
        <rect x="3" y="4" width="18" height="12" rx="2" />
        <path d="M7 20h10" />
      </svg>
    )
  }
  // Fallback: generic device
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  )
}

// ─── Status Badge ────────────────────────────────────────────────

function StatusDot({ online }: { online: boolean }) {
  return (
    <span
      className={`inline-block w-2 h-2 rounded-full ${online ? 'bg-emerald-400' : 'bg-gray-600'}`}
      title={online ? 'Online' : 'Offline'}
    />
  )
}

// ─── Copied Toast ────────────────────────────────────────────────

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)

  const copy = useCallback(() => {
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 2000)
      },
      () => {
        // Fallback for non-HTTPS or permission denied
        window.prompt('Copy this URL:', text)
      }
    )
  }, [text])

  return (
    <button
      onClick={copy}
      className="ml-2 px-2 py-1 text-xs rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-gray-400 hover:text-white transition-colors"
      title="Copy to clipboard"
    >
      {copied ? 'Copied!' : 'Copy'}
    </button>
  )
}

// ─── Prerequisites Card ──────────────────────────────────────────

/**
 * Tailscale used to be a prerequisite: without it the toggle did not render and the
 * server refused to bind wide. It is a recommendation now — every connection carries
 * a device token, so the credential is the boundary and the tailnet is what keeps
 * that credential off the wire in the clear. Hence the neutral treatment: this is
 * advice, not a blocked state.
 */
function TailscaleCard({
  status,
  onRefresh,
  loading
}: {
  status: TailscaleStatus
  onRefresh: () => void
  loading: boolean
}) {
  const notInstalled = !status.installed
  const notRunning = status.installed && !status.running

  return (
    <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-4 mb-6">
      <div className="flex items-start gap-3">
        {/* Warning icon */}
        <div className="mt-0.5 text-gray-500 shrink-0">
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
          </svg>
        </div>
        <div className="flex-1 min-w-0">
          {notInstalled && (
            <>
              <div className="text-sm font-medium text-gray-200 mb-1">
                Tailscale is not installed
              </div>
              <p className="text-xs text-gray-400 mb-3">
                Remote access works without it. Tailscale adds encryption in transit and reaches
                your devices from anywhere, not just this network — which is what you want on wifi
                you do not control.
              </p>
              <a
                href="https://tailscale.com/download"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md text-gray-300 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
              >
                Download Tailscale
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6" />
                  <polyline points="15 3 21 3 21 9" />
                  <line x1="10" y1="14" x2="21" y2="3" />
                </svg>
              </a>
            </>
          )}
          {notRunning && (
            <>
              <div className="text-sm font-medium text-gray-200 mb-1">
                Tailscale is not connected
              </div>
              <p className="text-xs text-gray-400">
                Open the Tailscale app and sign in to connect to your network.
                {status.backendState === 'NeedsLogin'
                  ? ' Your device needs to authenticate.'
                  : status.backendState === 'Stopped'
                    ? ' The connection is stopped. Click "Connect" in the Tailscale menu.'
                    : ''}
              </p>
            </>
          )}
        </div>
        <button
          onClick={onRefresh}
          disabled={loading}
          className="shrink-0 p-1.5 rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors disabled:opacity-40"
          title="Check again"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            className={loading ? 'animate-spin' : ''}
          >
            <polyline points="23 4 23 10 17 10" />
            <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
          </svg>
        </button>
      </div>
    </div>
  )
}

// ─── Device List ─────────────────────────────────────────────────

function TailnetPeerList({ status }: { status: TailscaleStatus }) {
  const allDevices = [
    {
      ip: status.selfIP,
      hostname: status.selfDNSName.split('.')[0] || 'This device',
      os: status.selfOS || 'unknown',
      online: true,
      isSelf: true
    },
    ...status.peers.map((p) => ({ ...p, isSelf: false }))
  ]

  const online = allDevices.filter((d) => d.online)
  const offline = allDevices.filter((d) => !d.online)

  return (
    <div className="mt-6">
      <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium mb-2">
        Devices on your network
      </div>
      <div
        className="rounded-lg border border-white/[0.06] overflow-hidden"
        style={{ background: 'var(--color-surface-sunken)' }}
      >
        {[...online, ...offline].map((device, i) => (
          <div
            key={device.ip || i}
            className={`flex items-center gap-3 px-4 py-3 ${
              i > 0 ? 'border-t border-white/[0.04]' : ''
            } ${device.isSelf ? 'bg-white/[0.02]' : ''}`}
          >
            <div className="text-gray-500">
              <OsIcon os={device.os} />
            </div>
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="text-sm text-gray-200 truncate">{device.hostname}</span>
                {device.isSelf && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-white/[0.06] text-gray-500">
                    this device
                  </span>
                )}
              </div>
              <div className="text-xs text-gray-600 font-mono">{device.ip}</div>
            </div>
            <StatusDot online={device.online} />
          </div>
        ))}
      </div>
    </div>
  )
}

// ─── QR Code ─────────────────────────────────────────────────────

function QRCodeDisplay({ url }: { url: string }) {
  const [dataUrl, setDataUrl] = useState<string | null>(null)

  useEffect(() => {
    let mounted = true
    QRCode.toDataURL(url, {
      width: 200,
      margin: 2,
      color: {
        dark: '#ffffffFF',
        light: '#00000000'
      }
    })
      .then((data) => {
        if (mounted) setDataUrl(data)
      })
      .catch((err) => console.warn('[QRCode] generation failed:', err))
    return () => {
      mounted = false
    }
  }, [url])

  if (!dataUrl) return null

  return (
    <div className="flex flex-col items-center">
      <div className="rounded-xl bg-white/[0.06] p-3 border border-white/[0.06]">
        <img
          src={dataUrl}
          alt="QR code to connect"
          className="w-[160px] h-[160px] sm:w-[200px] sm:h-[200px]"
        />
      </div>
      <p className="text-[10px] text-gray-600 mt-2">Scan with your phone to connect</p>
    </div>
  )
}

// ─── Connection Info ─────────────────────────────────────────────

/**
 * Every address this server answers on, not just the tailnet one.
 *
 * The old version read `status.appUrl` and returned null without it, so with
 * Tailscale absent the panel said remote access was on and never said where to point
 * a browser. A machine usually has several addresses and only the person looking at
 * the screen knows which network the other device is on, so list them all rather than
 * guess.
 */
function ConnectionInfo({ reachable }: { reachable: ReachableUrls | null }) {
  if (!reachable || reachable.urls.length === 0) return null
  const [primary, ...rest] = reachable.urls

  return (
    <div className="mt-4 space-y-4">
      <div>
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium mb-2">
          Access {reachable.urls.length > 1 ? 'URLs' : 'URL'}
        </div>
        <div className="space-y-1.5">
          {reachable.urls.map((url) => (
            <div
              key={url}
              className="flex items-center rounded-lg bg-white/[0.04] border border-white/[0.06] px-4 py-3"
            >
              <code className="text-sm text-emerald-400 font-mono flex-1 truncate">{url}</code>
              <CopyButton text={url} />
            </div>
          ))}
        </div>
        {rest.length > 0 && (
          <p className="text-[10px] text-gray-600 mt-2">
            Use whichever address the other device shares a network with.
          </p>
        )}
      </div>

      {/* The QR encodes the first, which is the tailnet address when there is one. */}
      <div className="flex justify-center py-2">
        <QRCodeDisplay url={primary} />
      </div>
    </div>
  )
}

/**
 * The devices allowed to connect, and the only place to make one.
 *
 * Until this existed, pairing a phone meant finding a terminal on the machine
 * running the server and typing `vorn-server token create` — which the panel could
 * only tell you about, not do. That was tolerable when the server was always the
 * machine in front of you; it is not once the server is somewhere else.
 */
function DeviceTokenList() {
  const [tokens, setTokens] = useState<DeviceToken[] | null>(null)
  const [creating, setCreating] = useState(false)
  const [name, setName] = useState('')
  /** Shown once, then unrecoverable — only its hash is stored. */
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
    <div className="mt-6">
      <div className="flex items-center justify-between mb-2">
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium">
          Devices {active.length > 0 && `(${active.length})`}
        </div>
        {!creating && (
          <button
            onClick={() => setCreating(true)}
            className="text-xs text-gray-400 hover:text-white px-2 py-1 rounded-md hover:bg-white/[0.06] transition-colors"
          >
            Add device
          </button>
        )}
      </div>

      {/* The one moment the token is readable. Said plainly rather than left to
          be discovered after the dialog closes. */}
      {minted && (
        <div className="mb-3 rounded-lg border border-white/[0.08] bg-white/[0.03] p-4">
          <div className="text-sm font-medium text-gray-200 mb-1">
            Token for &ldquo;{minted.name}&rdquo;
          </div>
          <p className="text-xs text-gray-500 mb-3">
            Copy it now. It is stored only as a hash, so this is the only time it can be shown.
          </p>
          <div className="flex items-center rounded-lg bg-white/[0.04] border border-white/[0.06] px-3 py-2">
            <code className="text-xs text-emerald-400 font-mono flex-1 truncate">
              {minted.plaintext}
            </code>
            <CopyButton text={minted.plaintext} />
          </div>
          <button
            onClick={() => setMinted(null)}
            className="mt-3 text-xs text-gray-500 hover:text-white px-2 py-1 rounded-md hover:bg-white/[0.06] transition-colors"
          >
            Done
          </button>
        </div>
      )}

      {creating && (
        <div className="mb-3 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
          <label className="block text-xs text-gray-400 mb-2" htmlFor="device-token-name">
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
          <div className="flex gap-2 mt-3">
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

      {error && <p className="text-xs text-amber-400/80 mb-2">{error}</p>}

      {tokens !== null && active.length === 0 && !creating && (
        <p className="text-xs text-gray-600">
          No devices yet. Add one, then open the address above on that device.
        </p>
      )}

      <div className="space-y-1.5">
        {active.map((token) => (
          <div
            key={token.id}
            className="flex items-center gap-3 rounded-lg bg-white/[0.02] border border-white/[0.06] px-4 py-3"
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
    </div>
  )
}

/**
 * Point this desktop at a Vorn running somewhere else.
 *
 * The other half of remote access: the panel above shares *this* machine's server
 * with other devices, and this connects to a server someone else is sharing. One
 * at a time — two servers means two databases, and the local one would shadow the
 * remote without saying so.
 *
 * Applying restarts the app, because the spawn-or-connect decision is made once at
 * startup and everything downstream assumes its answer.
 */
function HostModeCard() {
  const [expanded, setExpanded] = useState(false)
  const [current, setCurrent] = useState<{ mode: string; url: string } | null>(null)
  const [url, setUrl] = useState('')
  const [token, setToken] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    window.api
      .getConnectSettings?.()
      .then((s) => {
        if (cancelled || !s) return
        setCurrent({ mode: s.mode, url: s.url })
        setUrl(s.url)
      })
      .catch(() => {
        /* Electron-only; the web client is already talking to a host. */
      })
    return () => {
      cancelled = true
    }
  }, [])

  // The web client reaches a server by its address, so it has nothing to point.
  if (!current) return null

  const connected = current.mode === 'host'

  const apply = async (): Promise<void> => {
    setError(null)
    const result = await window.api.saveConnectSettings({ url, token })
    if (!result.ok) setError(result.error ?? 'Could not save those details.')
  }

  return (
    <div className="mt-8 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium text-gray-200 mb-1">
            {connected ? 'Connected to another Vorn' : 'Connect to another Vorn'}
          </div>
          <p className="text-xs text-gray-600">
            {connected
              ? `Using ${current.url}. Sessions and data live on that machine.`
              : 'Use a server running on another machine instead of this one. Its sessions, tasks and projects become yours.'}
          </p>
        </div>
        {!expanded && (
          <button
            onClick={() => setExpanded(true)}
            className="shrink-0 px-3 py-1.5 text-xs font-medium rounded-md text-gray-300 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
          >
            {connected ? 'Change' : 'Connect'}
          </button>
        )}
      </div>

      {expanded && (
        <div className="mt-4">
          <label className="block text-xs text-gray-400 mb-1.5" htmlFor="host-url">
            Server address
          </label>
          <input
            id="host-url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="192.168.0.4:61601"
            spellCheck={false}
            className="w-full rounded-md bg-white/[0.04] border border-white/[0.08] px-3 py-2 text-sm text-gray-200 font-mono placeholder:text-gray-600 outline-none focus:border-white/20"
          />
          <label className="block text-xs text-gray-400 mb-1.5 mt-3" htmlFor="host-token">
            Device token from that machine
          </label>
          <input
            id="host-token"
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="vorn_..."
            spellCheck={false}
            className="w-full rounded-md bg-white/[0.04] border border-white/[0.08] px-3 py-2 text-sm text-gray-200 font-mono placeholder:text-gray-600 outline-none focus:border-white/20"
          />

          {error && <p className="text-xs text-amber-400/80 mt-2">{error}</p>}

          <p className="text-xs text-gray-600 mt-3">
            Vorn restarts to apply this. Workflows and schedules run only while a desktop is
            attached to the host — they do not execute on the server by itself.
          </p>

          <div className="flex gap-2 mt-3">
            <button
              onClick={() => void apply()}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Connect and restart
            </button>
            {connected && (
              <button
                onClick={() => void window.api.useLocalServer()}
                className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-400 hover:text-white hover:bg-white/[0.06] transition-colors"
              >
                Use this machine
              </button>
            )}
            <button
              onClick={() => {
                setExpanded(false)
                setError(null)
                setToken('')
              }}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

// ─── Main Component ──────────────────────────────────────────────

export function NetworkSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const [status, setStatus] = useState<TailscaleStatus | null>(null)
  const [reachable, setReachable] = useState<ReachableUrls | null>(null)
  const [loading, setLoading] = useState(true)
  /** Set when the toggle is flipped on without a tailnet; see the panel below. */
  const [confirming, setConfirming] = useState(false)

  const fetchStatus = useCallback(async () => {
    setLoading(true)
    try {
      // Independent questions, so asked together: a missing Tailscale must not stop
      // the panel from showing an address, which is the whole point of this pass.
      const [result, urls] = await Promise.all([
        window.api.getTailscaleStatus(),
        window.api.getReachableUrls()
      ])
      setStatus(result)
      setReachable(urls)
    } catch (err) {
      console.error('[NetworkSettings] failed to get network status:', err)
      // Drop the addresses with the status. Keeping the previous ones would leave
      // the panel advertising a URL while it cannot reach the server to confirm it,
      // which reads as "still reachable" at the exact moment it is not.
      setReachable(null)
      setStatus({
        installed: false,
        running: false,
        backendState: 'Error',
        selfIP: '',
        selfDNSName: '',
        peers: []
      })
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: read the network status once on open
    fetchStatus()
  }, [fetchStatus])

  if (!config) return null

  const updateDefaults = (patch: Partial<typeof config.defaults>): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, ...patch }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  const enabled = config.defaults.networkAccessEnabled ?? false
  const onTailnet = status?.running === true
  const showTailscaleCard = status !== null && !onTailnet

  const setEnabled = (value: boolean): void => {
    setConfirming(false)
    updateDefaults({ networkAccessEnabled: value })
    // The server rebinds on the config change, which takes a moment; the addresses
    // it reports are what changes, so re-ask once it has settled.
    setTimeout(fetchStatus, 500)
  }

  return (
    <div>
      <SettingsPageHeader
        title="Remote Access"
        description="Reach this Vorn from your phone or another computer"
      />

      {/* Loading state */}
      {loading && !status && (
        <div className="flex items-center gap-3 py-8 justify-center">
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            className="animate-spin text-gray-500"
          >
            <polyline points="23 4 23 10 17 10" />
            <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
          </svg>
          <span className="text-sm text-gray-500">Checking network status...</span>
        </div>
      )}

      {/* Enable toggle — no longer gated on Tailscale */}
      <div className="space-y-1">
        <SettingRow
          label="Enable Remote Access"
          description="Let other devices on this network open Vorn in a browser"
        >
          <ToggleSwitch
            checked={enabled}
            onChange={(value) => {
              // Turning it off is never worth a confirmation, and neither is turning
              // it on over a tailnet, where the traffic is encrypted either way.
              if (!value || onTailnet) return setEnabled(value)
              setConfirming(true)
            }}
          />
        </SettingRow>
      </div>

      {/* The acknowledgement. Stating what the wide bind actually costs is the whole
          reason this pass touches the UI at all. */}
      {confirming && (
        <div className="mt-3 rounded-lg border border-amber-500/20 bg-amber-500/[0.04] p-4">
          <div className="text-sm font-medium text-amber-200 mb-1">Enable without Tailscale?</div>
          <div className="text-xs text-gray-400 space-y-1.5">
            <p>
              Vorn will accept connections from anything on this network, over plain HTTP. A device
              token is still required, but it travels unencrypted — anyone who can read this
              network&apos;s traffic can take it, and a token runs commands on this machine.
            </p>
            <p>Fine on a home or office network you trust. Not on shared or public wifi.</p>
          </div>
          <div className="flex gap-2 mt-3">
            <button
              onClick={() => setEnabled(true)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-200 border border-white/[0.08] hover:bg-white/[0.06] transition-colors"
            >
              Enable anyway
            </button>
            <button
              onClick={() => setConfirming(false)}
              className="px-3 py-1.5 text-xs font-medium rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Connection info + QR */}
      {enabled && <ConnectionInfo reachable={reachable} />}
      {enabled && <DeviceTokenList />}

      {/* Tailscale, as a recommendation rather than a prerequisite */}
      {showTailscaleCard && (
        <div className="mt-6">
          <TailscaleCard status={status} onRefresh={fetchStatus} loading={loading} />
        </div>
      )}

      {/* Device list */}
      {enabled && onTailnet && status && <TailnetPeerList status={status} />}

      <HostModeCard />

      {/* How it works — always visible */}
      {status && (
        <div className="mt-8 rounded-lg border border-white/[0.04] bg-white/[0.02] p-4">
          <div className="text-xs font-medium text-gray-400 mb-2">How it works</div>
          <div className="text-xs text-gray-600 space-y-1.5">
            <p>
              Every connection to Vorn carries a device token, on this machine and from anywhere
              else. Enabling remote access opens the port to the rest of the network; the token is
              what decides who gets in.
            </p>
            <p>
              Tailscale is the recommended way to reach it. It encrypts the connection and works
              from any network, so the token is never readable in transit and you are not limited to
              devices sitting on the same wifi.
            </p>
          </div>
        </div>
      )}
    </div>
  )
}
