import { useState, useEffect, useCallback } from 'react'
import { useAppStore } from '../../stores'
import { TailscaleStatus, ReachableUrls } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SegmentedControl } from './SegmentedControl'
import { ShareThisMachine } from './network/ShareThisMachine'
import { UseAnotherMachine } from './network/UseAnotherMachine'

/**
 * Remote Access: which Vorn this window talks to, and who else may reach it.
 *
 * The panel used to stack eight blocks in one column, and the saturation was a
 * symptom of a structural problem rather than a styling one. Seven of those blocks
 * were about sharing *this* machine; one was about joining *another*. Opposite
 * directions, drawn identically, offered together.
 *
 * They cannot both apply. Host mode never starts a local server, so while connected
 * to one there is nothing here to share, and every sharing control was describing
 * something that was not running. Hence one question at the top: the two halves are
 * genuinely exclusive, so nothing contradictory is ever on screen.
 *
 * Two things were dropped rather than moved. A list of every device on the tailnet,
 * which answered "what is on my Tailscale account" — a question Tailscale's own app
 * answers, and never the one this panel is for. And a "How it works" card whose
 * twenty-eight words restated the two blocks directly above it.
 */
type Mode = 'local' | 'host'

export function NetworkSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const [status, setStatus] = useState<TailscaleStatus | null>(null)
  const [reachable, setReachable] = useState<ReachableUrls | null>(null)
  const [loading, setLoading] = useState(true)
  /**
   * Which half is on screen, and where the app actually points.
   *
   * `connect` is null in the web build, where `getConnectSettings` answers null: a
   * browser reached its server by address and has nothing to point elsewhere. That
   * null is what hides the switch entirely rather than showing a half that cannot
   * work there.
   */
  const [connect, setConnect] = useState<{ mode: Mode; url: string } | null>(null)
  const [viewing, setViewing] = useState<Mode>('local')

  const fetchStatus = useCallback(async () => {
    setLoading(true)
    try {
      // Independent questions, so asked together: a missing Tailscale must not stop
      // the panel from showing an address.
      const [result, urls] = await Promise.all([
        window.api.getTailscaleStatus(),
        window.api.getReachableUrls()
      ])
      setStatus(result)
      setReachable(urls)
    } catch (err) {
      console.error('[NetworkSettings] failed to get network status:', err)
      // Drop the addresses with the status. Keeping the previous ones would leave the
      // panel advertising a URL while it cannot reach the server to confirm it, which
      // reads as "still reachable" at exactly the moment it is not.
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

  useEffect(() => {
    let cancelled = false
    window.api
      .getConnectSettings?.()
      .then((s) => {
        if (cancelled || !s) return
        const mode = s.mode === 'host' ? 'host' : 'local'
        setConnect({ mode, url: s.url })
        setViewing(mode)
      })
      .catch(() => {
        /* Electron-only; the web client is already talking to a host. */
      })
    return () => {
      cancelled = true
    }
  }, [])

  if (!config) return null

  const updateDefaults = (patch: Partial<typeof config.defaults>): void => {
    const updated = { ...config, defaults: { ...config.defaults, ...patch } }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  const enabled = config.defaults.networkAccessEnabled ?? false

  const setEnabled = (value: boolean): void => {
    updateDefaults({ networkAccessEnabled: value })
    // The server rebinds on the config change, which takes a moment; the addresses it
    // reports are what changes, so re-ask once it has settled.
    setTimeout(fetchStatus, 500)
  }

  return (
    <div>
      <SettingsPageHeader
        title="Remote Access"
        description="Reach this Vorn from your phone or another computer"
      />

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

      {connect && (
        <div className="mb-5">
          <SegmentedControl
            options={[
              { value: 'local', label: 'This machine' },
              { value: 'host', label: 'Another machine' }
            ]}
            value={viewing}
            onChange={(value) => setViewing(value as Mode)}
          />
        </div>
      )}

      {viewing === 'host' && connect ? (
        <UseAnotherMachine mode={connect.mode} url={connect.url} />
      ) : (
        <ShareThisMachine
          enabled={enabled}
          status={status}
          reachable={reachable}
          onChange={setEnabled}
        />
      )}
    </div>
  )
}
