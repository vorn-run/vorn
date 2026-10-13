import { useAppStore } from '../../stores'
import { NotificationConfig } from '../../../shared/types'
import { playNotificationSound } from '../../lib/notifications'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'

const DEFAULT_NOTIFICATIONS: NotificationConfig = {
  enabled: false,
  onWaiting: true,
  onError: true,
  onBell: true
}

export function NotificationSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)

  if (!config) return null

  const notifications = { ...DEFAULT_NOTIFICATIONS, ...config.defaults.notifications }

  const updateNotifications = (patch: Partial<NotificationConfig>): void => {
    const updated = {
      ...config,
      defaults: {
        ...config.defaults,
        notifications: { ...notifications, ...patch }
      }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  return (
    <div>
      <SettingsPageHeader
        title="Notifications"
        description="Get notified when agents need your attention"
      />

      <div className="space-y-1">
        <SettingRow
          label="Enable Notifications"
          description="Show OS notifications when agents need attention"
        >
          <ToggleSwitch
            checked={notifications.enabled}
            onChange={(enabled) => {
              updateNotifications({ enabled })
              if (enabled && Notification.permission === 'default') {
                Notification.requestPermission()
              }
            }}
          />
        </SettingRow>

        <SettingRow
          label="Waiting for Input"
          description="Notify when an agent is waiting for your response"
          disabled={!notifications.enabled}
        >
          <ToggleSwitch
            checked={notifications.onWaiting}
            onChange={(onWaiting) => updateNotifications({ onWaiting })}
            disabled={!notifications.enabled}
          />
        </SettingRow>

        <SettingRow
          label="Errors"
          description="Notify when an agent encounters an error"
          disabled={!notifications.enabled}
        >
          <ToggleSwitch
            checked={notifications.onError}
            onChange={(onError) => updateNotifications({ onError })}
            disabled={!notifications.enabled}
          />
        </SettingRow>

        <SettingRow
          label="Terminal Bell"
          description="Notify when an agent explicitly sends a notification signal"
          disabled={!notifications.enabled}
        >
          <ToggleSwitch
            checked={notifications.onBell}
            onChange={(onBell) => updateNotifications({ onBell })}
            disabled={!notifications.enabled}
          />
        </SettingRow>
      </div>

      {/* Sound section */}
      <h3 className="text-sm font-medium text-gray-200 mt-8 mb-2">Sound</h3>
      <p className="text-sm text-gray-500 mb-6">Play audio tones when agents change status</p>

      <div className="space-y-3">
        <SettingRow
          label="Sound Effects"
          description="Play a tone on waiting, error, and bell events (works even when focused)"
          disabled={!notifications.enabled}
        >
          <ToggleSwitch
            checked={notifications.soundEnabled ?? false}
            onChange={(soundEnabled) => updateNotifications({ soundEnabled })}
            disabled={!notifications.enabled}
          />
        </SettingRow>

        {notifications.soundEnabled && notifications.enabled && (
          <div className="flex items-center gap-3 px-4 py-2">
            <span className="text-xs text-gray-400 shrink-0 w-12">Volume</span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round((notifications.soundVolume ?? 0.5) * 100)}
              onChange={(e) => updateNotifications({ soundVolume: Number(e.target.value) / 100 })}
              className="flex-1 h-1 bg-white/[0.08] rounded-full appearance-none cursor-pointer
                         [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3
                         [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:rounded-full
                         [&::-webkit-slider-thumb]:bg-white"
            />
            <span className="text-xs text-gray-500 w-8 text-right">
              {Math.round((notifications.soundVolume ?? 0.5) * 100)}%
            </span>
            <button
              onClick={() => playNotificationSound('bell', notifications.soundVolume ?? 0.5)}
              className="px-2 py-1 text-[11px] text-gray-400 hover:text-white bg-white/[0.06]
                         hover:bg-white/[0.1] rounded transition-colors"
            >
              Test
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
