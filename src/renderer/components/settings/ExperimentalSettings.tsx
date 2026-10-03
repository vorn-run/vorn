import { useAppStore } from '../../stores'
import type { ExperimentalFlags } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'

/**
 * Native replacements for parts of the server, one switch each.
 *
 * Every switch is off until turned on here, and the code it replaces stays the
 * default. Once one has been on without trouble it becomes the default and the
 * switch goes, along with the path it switched away from.
 */
export function ExperimentalSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)

  if (!config) return null

  const flags = config.defaults.experimental ?? {}
  const update = (patch: ExperimentalFlags): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, experimental: { ...flags, ...patch } }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  return (
    <div>
      <SettingsPageHeader
        title="Experimental"
        description="Faster native versions of parts of Vorn, off until you turn them on"
      />

      <div className="space-y-1">
        <SettingRow
          label="Native Git"
          description="Run git in Vorn's Rust core instead of on the server's main thread, so a large diff or a slow repository no longer stalls every terminal while it runs. Takes effect on the next git command."
        >
          <ToggleSwitch
            checked={flags.nativeGit === true}
            onChange={(nativeGit) => update({ nativeGit })}
          />
        </SettingRow>
      </div>
    </div>
  )
}
