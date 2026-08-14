import { useState, useEffect, useCallback } from 'react'
import QRCode from 'qrcode'
import { useAppStore } from '../../stores'
import { TailscaleStatus } from '../../../shared/types'
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

function PrerequisitesCard({
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
    <div className="rounded-lg border border-amber-500/20 bg-amber-500/[0.04] p-4 mb-6">
      <div className="flex items-start gap-3">
        {/* Warning icon */}
        <div className="mt-0.5 text-amber-400 shrink-0">
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </div>
        <div className="flex-1 min-w-0">
          {notInstalled && (
            <>
              <div className="text-sm font-medium text-amber-200 mb-1">
                Tailscale is not installed
              </div>
              <p className="text-xs text-gray-400 mb-3">
                Tailscale creates a secure private network between your devices. Install it to
                access Vorn from your phone, tablet, or other computers.
              </p>
              <a
                href="https://tailscale.com/download"
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md bg-amber-500/10 text-amber-300 hover:bg-amber-500/20 transition-colors"
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
              <div className="text-sm font-medium text-amber-200 mb-1">
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

function DeviceList({ status }: { status: TailscaleStatus }) {
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

function ConnectionInfo({ status }: { status: TailscaleStatus }) {
  if (!status.appUrl) return null

  return (
    <div className="mt-4 space-y-4">
      {/* URL display */}
      <div>
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium mb-2">
          Access URL
        </div>
        <div className="flex items-center rounded-lg bg-white/[0.04] border border-white/[0.06] px-4 py-3">
          <code className="text-sm text-emerald-400 font-mono flex-1 truncate">
            {status.appUrl}
          </code>
          <CopyButton text={status.appUrl} />
        </div>
      </div>

      {/* QR code */}
      <div className="flex justify-center py-2">
        <QRCodeDisplay url={status.appUrl} />
      </div>
    </div>
  )
}

// ─── Main Component ──────────────────────────────────────────────

export function NetworkSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const [status, setStatus] = useState<TailscaleStatus | null>(null)
  const [loading, setLoading] = useState(true)

  const fetchStatus = useCallback(async () => {
    setLoading(true)
    try {
      const result = await window.api.getTailscaleStatus()
      setStatus(result)
    } catch (err) {
      console.error('[NetworkSettings] failed to get tailscale status:', err)
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
  const showPrereqs = status && (!status.installed || !status.running)
  const showConnectionInfo = status?.running && enabled

  return (
    <div>
      <SettingsPageHeader
        title="Remote Access"
        description="Access Vorn from other devices on your Tailscale network"
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
          <span className="text-sm text-gray-500">Checking Tailscale status...</span>
        </div>
      )}

      {/* Prerequisites warning */}
      {status && showPrereqs && (
        <PrerequisitesCard status={status} onRefresh={fetchStatus} loading={loading} />
      )}

      {/* Enable toggle */}
      {status && status.running && (
        <div className="space-y-1">
          <SettingRow
            label="Enable Remote Access"
            description="Allow other devices on your Tailscale network to access Vorn"
          >
            <ToggleSwitch
              checked={enabled}
              onChange={(value) => {
                updateDefaults({ networkAccessEnabled: value })
                // Re-fetch status to get updated appUrl
                setTimeout(fetchStatus, 500)
              }}
            />
          </SettingRow>
        </div>
      )}

      {/* Connection info + QR */}
      {showConnectionInfo && status && <ConnectionInfo status={status} />}

      {/* Device list */}
      {showConnectionInfo && status && <DeviceList status={status} />}

      {/* How it works — always visible */}
      {status && !showPrereqs && (
        <div className="mt-8 rounded-lg border border-white/[0.04] bg-white/[0.02] p-4">
          <div className="text-xs font-medium text-gray-400 mb-2">How it works</div>
          <div className="text-xs text-gray-600 space-y-1.5">
            <p>
              Tailscale creates an encrypted mesh network between your devices. When remote access
              is enabled, other devices on your tailnet can connect to Vorn.
            </p>
            <p>
              Only devices signed into your Tailscale account can reach this address. No passwords,
              no port forwarding, no firewall rules.
            </p>
          </div>
        </div>
      )}
    </div>
  )
}
