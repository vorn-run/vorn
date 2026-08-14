import { useAppStore } from '../../stores'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'
import { SegmentedControl } from './SegmentedControl'
import { describeUpdateStatus } from '../../lib/update-status'

const TONE_CLASS = {
  idle: 'bg-white/[0.18]',
  busy: 'bg-white/[0.4]',
  accent: 'bg-bronzo',
  danger: 'bg-danger'
} as const

export function UpdatesSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const status = useAppStore((s) => s.appUpdateStatus)

  if (!config) return null

  const channel = config.defaults.updateChannel ?? 'stable'
  const autoDownload = config.defaults.updateAutoDownload !== false
  const view = describeUpdateStatus(status, channel)

  const updateDefaults = (patch: Partial<typeof config.defaults>): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, ...patch }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  return (
    <div>
      <SettingsPageHeader title="Updates" description="How Vorn keeps itself current" />

      {/* Reports rather than configures, so it is not a SettingRow: the panel
          should answer "what is happening?" before it offers any control. */}
      <div className="mb-5 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg flex items-center gap-3">
        <span className={`w-[5px] h-[5px] rounded-full shrink-0 ${TONE_CLASS[view.tone]}`} />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] text-gray-200">{view.label}</div>
          {view.detail && <div className="text-xs text-gray-500 mt-0.5">{view.detail}</div>}
          {view.percent != null && (
            <div className="h-[3px] bg-white/[0.08] rounded-full mt-2 overflow-hidden">
              <div
                className="h-full bg-white/30 rounded-full transition-[width] duration-300"
                style={{ width: `${view.percent}%` }}
              />
            </div>
          )}
        </div>
        {view.action === 'restart' && (
          <button
            onClick={() => window.api.installUpdate()}
            className="shrink-0 px-3 py-1 text-xs font-medium text-white bg-white/[0.1]
                       hover:bg-white/[0.14] border border-white/[0.08] rounded-md transition-colors"
          >
            Restart Now
          </button>
        )}
        {view.action === 'download' && (
          <button
            onClick={() => window.api.downloadUpdate()}
            className="shrink-0 px-3 py-1 text-xs font-medium text-gray-300 bg-white/[0.06]
                       hover:bg-white/[0.1] border border-white/[0.08] rounded-md transition-colors"
          >
            Download
          </button>
        )}
        {view.action === 'retry' && (
          <button
            onClick={() => window.api.checkForUpdates()}
            className="shrink-0 px-3 py-1 text-xs font-medium text-gray-300 bg-white/[0.06]
                       hover:bg-white/[0.1] border border-white/[0.08] rounded-md transition-colors"
          >
            Retry
          </button>
        )}
      </div>

      <div className="space-y-1">
        <SettingRow label="Current version" description={`Vorn ${window.api.getAppVersion()}`}>
          <button
            onClick={() => window.api.checkForUpdates()}
            disabled={status.kind === 'checking' || status.kind === 'unsupported'}
            className="px-3 py-1.5 text-xs text-gray-300 bg-white/[0.04] hover:bg-white/[0.08]
                       border border-white/[0.08] rounded-md transition-colors
                       disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {status.kind === 'checking' ? 'Checking…' : 'Check Now'}
          </button>
        </SettingRow>

        <SettingRow
          label="Update channel"
          description="Beta receives early releases; stable receives tested releases only"
        >
          <SegmentedControl
            value={channel}
            onChange={(next) => {
              const ch = next as 'stable' | 'beta'
              updateDefaults({ updateChannel: ch })
              window.api.setUpdateChannel(ch)
            }}
            options={[
              { value: 'stable', label: 'Stable' },
              { value: 'beta', label: 'Beta' }
            ]}
          />
        </SettingRow>

        <SettingRow
          label="Download automatically"
          description="Installing still waits for you to restart"
        >
          <ToggleSwitch
            checked={autoDownload}
            onChange={(enabled) => {
              updateDefaults({ updateAutoDownload: enabled })
              window.api.setUpdateAutoDownload(enabled)
            }}
          />
        </SettingRow>
      </div>
    </div>
  )
}
