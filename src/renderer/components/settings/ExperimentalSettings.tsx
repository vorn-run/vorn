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

/** Why vornd, which runs every terminal, is not in use, or null. */
function vorndNote(status: VorndStatus | null): string | null {
  if (status?.state !== 'failed') return null
  return `Terminals cannot run, because vornd, the native daemon, is not in use: ${status.detail}.`
}

/** The native server's switch. The server reads it when it starts vornd. */
const SERVER_SWITCH = {
  label: 'Native server',
  description:
    'Answer git, file explorer, editor, agent lookup and shell lookup calls in vornd instead of the server. Applies after restarting Vorn'
}

/** What the native server is doing, when it differs from what the switch says, or null. */
function serverNote(on: boolean, status: VorndStatus | null): string | null {
  if (!status) return null
  if (on && status.state === 'failed')
    return 'The server answers these calls itself while vornd is not in use.'
  const answering = status.state === 'on' && status.nativeServer
  if (on && status.state === 'on' && !answering)
    return 'vornd answers these calls the next time Vorn starts.'
  if (!on && answering) return 'The server answers these calls again the next time Vorn starts.'
  return null
}

/** The store's switch. The server reads it when it starts. */
const STORE_SWITCH = {
  label: 'Native store',
  description:
    'Keep tasks, workflows and settings through the native store, on the same database. Applies after restarting Vorn'
}

/** What the store is doing, when it differs from what the switch says, or null. */
function storeNote(on: boolean, status: CoreStatus['store'] | undefined): string | null {
  if (!status) return null
  if (on && !status.native && status.error)
    return `Vorn kept its built-in store, because the native store did not open: ${status.error}.`
  if (on && !status.native) return 'Vorn uses the native store the next time it starts.'
  if (!on && status.native) return 'Vorn goes back to its built-in store the next time it starts.'
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
    return `The native core did not load, so git runs more slowly and the native store is not available${
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
  // Null until it arrives, and for good where vornd's status cannot be asked (the browser).
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
  const daemonNote = vorndNote(daemon)
  const nativeStoreNote = storeNote(flags.nativeStore === true, status?.store)
  const nativeServerNote = serverNote(flags.nativeServer === true, daemon)

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
      <div className="mt-1 space-y-1">
        <SettingRow label={SERVER_SWITCH.label} description={SERVER_SWITCH.description}>
          <ToggleSwitch
            checked={flags.nativeServer === true}
            onChange={(value) => setFlag('nativeServer', value)}
            label={SERVER_SWITCH.label}
          />
        </SettingRow>
      </div>
      {nativeServerNote && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {nativeServerNote}
        </div>
      )}
      <div className="mt-1 space-y-1">
        <SettingRow label={STORE_SWITCH.label} description={STORE_SWITCH.description}>
          <ToggleSwitch
            checked={flags.nativeStore === true}
            onChange={(value) => setFlag('nativeStore', value)}
            label={STORE_SWITCH.label}
          />
        </SettingRow>
      </div>
      {nativeStoreNote && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {nativeStoreNote}
        </div>
      )}
      {status?.version && (
        <div className="mt-4 text-xs text-gray-500">Native core {status.version}</div>
      )}
    </div>
  )
}
