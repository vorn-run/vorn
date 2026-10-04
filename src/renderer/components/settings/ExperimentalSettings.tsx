import { useEffect, useState } from 'react'
import { useAppStore } from '../../stores'
import type { CoreStatus, ExperimentalConfig, VorndStatus } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'

/**
 * One switch per piece of the terminal pipeline that has moved onto the Rust
 * core. A row is added here when its work package lands, and removed when the
 * native path becomes the only one.
 */
const SWITCHES: { key: keyof ExperimentalConfig; label: string; description: string }[] = [
  {
    key: 'nativeScreen',
    label: 'Native screen model',
    description:
      "Keep each terminal's screen in Ghostty's engine instead of a second xterm, for history and restore"
  },
  {
    key: 'nativeAnalysis',
    label: 'Native output analysis',
    description:
      'Work out agent status and the output agents read back in Rust, once per flush instead of on every read'
  },
  {
    key: 'nativeGit',
    label: 'Native git',
    description:
      'Run git on the native core, off the main thread, so terminals keep flowing while git works. Applies from the next git command'
  }
]

/**
 * The daemon's switch, apart from the rest: the desktop app reads it when it
 * starts, not the server, so nothing about the core's state locks it.
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

/** What the core's state means for the switches, or null when they work as labelled. */
function coreNote(status: CoreStatus | 'unavailable' | null): string | null {
  if (!status) return null
  if (status === 'unavailable')
    return "This server can't report on the native core, so these stay on JavaScript."
  if (status.forced === 'js')
    return status.error
      ? `${status.error} by the server, so every switch is off.`
      : 'VORN_CORE=js is set for the server, so every switch is off.'
  // Before the forced-native note: a binary that will not load leaves every
  // terminal on JavaScript whatever VORN_CORE asks for.
  if (status.loaded === false) {
    return `The native core is not available in this build, so these stay on JavaScript${
      status.error ? `: ${status.error}` : '.'
    }`
  }
  // Before the forced-native note too: VORN_CORE=native cannot turn on what the
  // binary was built without.
  if (status.missing?.length) {
    const names = SWITCHES.filter((s) => status.missing.includes(s.key)).map((s) => s.label)
    return `This build of the native core does not include ${names.join(', ')}, so ${
      names.length === 1 ? 'that stays' : 'those stay'
    } on JavaScript${status.forced === 'native' ? ', even with VORN_CORE=native set' : ''}.`
  }
  if (status.forced === 'native')
    return 'VORN_CORE=native is set for the server, so every switch is on.'
  return null
}

export function ExperimentalSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  // 'unavailable' for a server older than the method: it never reads the
  // switches either, so they are locked rather than left to look as if they work.
  const [status, setStatus] = useState<CoreStatus | 'unavailable' | null>(null)
  // Null until it arrives, and for good where vornd cannot run (the browser):
  // the daemon's row is shown only once there is a status to show it with.
  const [daemon, setDaemon] = useState<VorndStatus | null>(null)

  useEffect(() => {
    let cancelled = false
    const unavailable = (): void => {
      if (!cancelled) setStatus('unavailable')
    }
    if (!window.api.getCoreStatus) unavailable()
    else
      void window.api
        .getCoreStatus()
        .then((next) => {
          if (!cancelled) setStatus(next ?? 'unavailable')
        })
        .catch(unavailable)
    void window.api
      .getVorndStatus?.()
      .then((next) => {
        if (!cancelled) setDaemon(next ?? null)
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
  const known = status === 'unavailable' ? null : status
  // Locked until the status arrives too: until then the page cannot know
  // whether VORN_CORE overrides a switch or the binary lacks it.
  const locked =
    status === null || status === 'unavailable' || known?.forced != null || known?.loaded === false

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
        description="Work in progress you can try before it is the default. Each switch applies to terminals opened after you change it."
      />
      {note && (
        <div className="mb-4 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {note}
        </div>
      )}
      <div className="space-y-1">
        {SWITCHES.map((s) => {
          const off = locked || known?.missing?.includes(s.key) === true
          return (
            <SettingRow key={s.key} label={s.label} description={s.description} disabled={off}>
              <ToggleSwitch
                checked={flags[s.key] === true}
                onChange={(value) => setFlag(s.key, value)}
                disabled={off}
                label={s.label}
              />
            </SettingRow>
          )
        })}
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
      {known?.version && (
        <div className="mt-4 text-xs text-gray-500">Native core {known.version}</div>
      )}
    </div>
  )
}
