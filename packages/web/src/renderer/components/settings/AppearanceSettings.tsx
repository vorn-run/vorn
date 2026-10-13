import { useAppStore } from '../../stores'
import { setAllTerminalsFontSize } from '../../lib/terminal-registry'
import { TaskViewMode } from '../../../shared/types'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { SegmentedControl } from './SegmentedControl'

export function AppearanceSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const rowHeight = useAppStore((s) => s.rowHeight)
  const setRowHeight = useAppStore((s) => s.setRowHeight)

  if (!config) return null

  const updateDefaults = (patch: Partial<typeof config.defaults>): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, ...patch }
    }
    window.api.saveConfig(updated)
    setConfig(updated)

    if (patch.fontSize !== undefined) {
      setAllTerminalsFontSize(patch.fontSize)
    }
    if (patch.layoutMode === 'tabs') {
      useAppStore.getState().setFocusedTerminal(null)
    }
  }

  return (
    <div>
      <SettingsPageHeader title="Appearance" description="Customize how Vorn looks" />

      <div className="space-y-1">
        {/* Font Size */}
        <SettingRow label="Font Size" description="Terminal font size in pixels">
          <div className="flex items-center gap-2">
            <button
              onClick={() =>
                updateDefaults({ fontSize: Math.max(8, config.defaults.fontSize - 1) })
              }
              className="w-7 h-7 flex items-center justify-center bg-white/[0.06] hover:bg-white/[0.1]
                         rounded-md text-gray-300 transition-colors text-sm"
            >
              -
            </button>
            <span className="text-sm text-gray-200 w-8 text-center">
              {config.defaults.fontSize}
            </span>
            <button
              onClick={() =>
                updateDefaults({ fontSize: Math.min(24, config.defaults.fontSize + 1) })
              }
              className="w-7 h-7 flex items-center justify-center bg-white/[0.06] hover:bg-white/[0.1]
                         rounded-md text-gray-300 transition-colors text-sm"
            >
              +
            </button>
          </div>
        </SettingRow>

        {/* Row Height */}
        <SettingRow
          label="Card Row Height"
          description={`Terminal card height in pixels (${Math.round(rowHeight)}px)`}
        >
          <div className="flex items-center gap-3">
            <input
              type="range"
              min={100}
              max={600}
              value={rowHeight}
              onChange={(e) => setRowHeight(Math.round(Number(e.target.value)))}
              className="w-32 accent-blue-500"
            />
            <span className="text-sm text-gray-400 w-12 text-right">{Math.round(rowHeight)}px</span>
          </div>
        </SettingRow>

        {/* Layout Mode */}
        <SettingRow
          label="Layout Mode"
          description="Choose between grid or tab layout for sessions"
        >
          <SegmentedControl
            options={[
              {
                value: 'grid',
                label: 'Grid',
                icon: (
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <rect x="3" y="3" width="7" height="7" />
                    <rect x="14" y="3" width="7" height="7" />
                    <rect x="3" y="14" width="7" height="7" />
                    <rect x="14" y="14" width="7" height="7" />
                  </svg>
                )
              },
              {
                value: 'tabs',
                label: 'Tabs',
                icon: (
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <rect x="3" y="3" width="18" height="18" rx="2" />
                    <path d="M3 9h18" />
                    <path d="M9 3v6" />
                  </svg>
                )
              }
            ]}
            value={config.defaults.layoutMode || 'grid'}
            onChange={(value) => updateDefaults({ layoutMode: value as 'grid' | 'tabs' })}
          />
        </SettingRow>

        {/* Default Task View */}
        <SettingRow
          label="Default Task View"
          description="Choose between list or kanban board for tasks"
        >
          <SegmentedControl
            options={[
              {
                value: 'list',
                label: 'List',
                icon: (
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <line x1="8" y1="6" x2="21" y2="6" />
                    <line x1="8" y1="12" x2="21" y2="12" />
                    <line x1="8" y1="18" x2="21" y2="18" />
                    <line x1="3" y1="6" x2="3.01" y2="6" />
                    <line x1="3" y1="12" x2="3.01" y2="12" />
                    <line x1="3" y1="18" x2="3.01" y2="18" />
                  </svg>
                )
              },
              {
                value: 'kanban',
                label: 'Kanban',
                icon: (
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <rect x="3" y="3" width="18" height="18" rx="2" />
                    <path d="M9 3v18" />
                    <path d="M15 3v18" />
                  </svg>
                )
              }
            ]}
            value={config.defaults.taskViewMode || 'list'}
            onChange={(value) => updateDefaults({ taskViewMode: value as TaskViewMode })}
          />
        </SettingRow>
      </div>
    </div>
  )
}
