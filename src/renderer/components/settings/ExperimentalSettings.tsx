import { useEffect, useState } from 'react'
import { useAppStore } from '../../stores'
import type {
  CoreStatus,
  ExperimentalConfig,
  SessionHolder,
  SessionHolders,
  VorndStatus
} from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'

/**
 * The daemon's switch. The desktop app reads it when it starts, not the
 * server, so it is shown only where vornd can run.
 */
const VORND_SWITCH = {
  label: 'Native daemon',
  description:
    'Connect to the server through vornd, the native daemon, instead of directly. Applies after restarting Vorn'
}

/** What vornd is doing, when it differs from what the switch says, or null. */
function vorndNote(on: boolean, status: VorndStatus | null): string | null {
  if (!status) return null
  if (status.state === 'failed')
    return `Vorn is connected to the server directly: ${status.detail}.`
  if (on && status.state === 'off') return 'Vorn connects through vornd the next time it starts.'
  if (!on && status.state === 'on') return 'Vorn stops using vornd the next time it starts.'
  return null
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`

/** What an older session holder means for the sessions on it. */
function olderNote(h: SessionHolder): string {
  const held =
    h.sessions === null
      ? 'Sessions started before Vorn was updated'
      : `${plural(h.sessions, 'session', 'sessions')} started before Vorn was updated`
  const verb = h.sessions === 1 ? 'is' : 'are'
  return h.compatible
    ? `${held} ${verb} still on the older session holder (${h.build}). It exits after the last one ends.`
    : `${held} ${verb} on a session holder (${h.build}) this version cannot talk to. They keep running until you end them.`
}

/** What is missing when the server runs without all of the native core, or null. */
function coreNote(status: CoreStatus | null): string | null {
  if (!status) return null
  if (!status.loaded) {
    return `The native core did not load, so terminals have no screen model or agent status${
      status.error ? `: ${status.error}` : '.'
    }`
  }
  if (status.missing.length) {
    return `This build of the native core was made without ${status.missing.join(', ')}.`
  }
  return null
}

export function ExperimentalSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  // Null until it arrives, and for a server older than the method.
  const [status, setStatus] = useState<CoreStatus | null>(null)
  // Null until it arrives, and for good where vornd cannot run (the browser):
  // the daemon's row is shown only once there is a status to show it with.
  const [daemon, setDaemon] = useState<VorndStatus | null>(null)
  const [holders, setHolders] = useState<SessionHolders | null>(null)
  const [endFailure, setEndFailure] = useState<string | null>(null)

  const loadHolders = (isCancelled: () => boolean = () => false): void => {
    void window.api
      .getSessionHolders?.()
      .then((next) => {
        if (!isCancelled()) setHolders(next ?? null)
      })
      .catch(() => {})
  }

  useEffect(() => {
    let cancelled = false
    void window.api
      .getCoreStatus?.()
      .then((next) => {
        if (!cancelled) setStatus(next ?? null)
      })
      .catch(() => {})
    void window.api
      .getVorndStatus?.()
      .then((next) => {
        if (cancelled) return
        setDaemon(next ?? null)
        if (next?.state === 'on') loadHolders(() => cancelled)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  if (!config) return null

  const flags = config.defaults.experimental ?? {}
  const note = coreNote(status)
  const daemonNote = vorndNote(flags.vornd === true, daemon)

  const endHolder = (h: SessionHolder): void => {
    const count = h.sessions === null ? 'the sessions' : plural(h.sessions, 'session', 'sessions')
    if (!window.confirm(`End ${count} on the older session holder? Their processes stop.`)) return
    void window.api
      .endSessionHolder?.(h.instance)
      .then((outcome) => {
        setEndFailure(outcome.ok ? null : `Could not end them: ${outcome.detail}.`)
        loadHolders()
      })
      .catch((err: Error) => setEndFailure(`Could not end them: ${err.message}.`))
  }

  const setFlag = (key: keyof ExperimentalConfig, value: boolean): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, experimental: { ...flags, [key]: value } }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  return (
    <div>
      <SettingsPageHeader
        title="Experimental"
        description="Work in progress you can try before it is the default."
      />
      {note && (
        <div className="mb-4 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {note}
        </div>
      )}
      <div className="space-y-1">
        {daemon && (
          <SettingRow label={VORND_SWITCH.label} description={VORND_SWITCH.description}>
            <ToggleSwitch
              checked={flags.vornd === true}
              onChange={(value) => setFlag('vornd', value)}
              label={VORND_SWITCH.label}
            />
          </SettingRow>
        )}
      </div>
      {daemonNote && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {daemonNote}
        </div>
      )}
      {holders?.error && !holders.current && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          The session holder is not running: {holders.error}.
        </div>
      )}
      {holders?.older
        .filter((h) => h.sessions !== 0)
        .map((h) => (
          <div
            key={h.instance}
            className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400 flex items-center gap-3"
          >
            <span className="flex-1">{olderNote(h)}</span>
            <button
              type="button"
              className="px-2 py-1 rounded border border-white/[0.12] text-gray-300 hover:bg-white/[0.06]"
              onClick={() => endHolder(h)}
            >
              End them
            </button>
          </div>
        ))}
      {endFailure && <div className="mt-2 text-xs text-red-400">{endFailure}</div>}
      {status?.version && (
        <div className="mt-4 text-xs text-gray-500">Native core {status.version}</div>
      )}
    </div>
  )
}
