import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { getShortcut } from '../../lib/keyboard-shortcuts'
import { describeUpdateStatus, hasPendingUpdate } from '../../lib/update-status'
import { CircleHelp, Settings } from 'lucide-react'

export function SidebarFooter({
  isCollapsed,
  closeSidebarOnMobile
}: {
  isCollapsed: boolean
  closeSidebarOnMobile: () => void
}) {
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen)
  const setSettingsCategory = useAppStore((s) => s.setSettingsCategory)
  const setOnboardingOpen = useAppStore((s) => s.setOnboardingOpen)
  const status = useAppStore((s) => s.appUpdateStatus)
  const dismissed = useAppStore((s) => s.updateBannerDismissed)
  const setDismissed = useAppStore((s) => s.setUpdateBannerDismissed)

  const settingsShortcut = getShortcut('settings')?.display
  const pending = hasPendingUpdate(status)
  // Collapsed the rail is 52px, which the banner cannot live in; dismissed the
  // user asked for it gone. Both keep the dot, so the update stays findable
  // without the banner — the old top banner simply vanished until relaunch.
  const showBanner = pending && !dismissed && !isCollapsed
  const showDot = pending && !showBanner

  const openUpdates = (): void => {
    // Order matters: the panel opens on whatever category was last viewed, so
    // set it before opening or this lands on Appearance.
    setSettingsCategory('updates')
    setSettingsOpen(true)
    closeSidebarOnMobile()
  }

  return (
    <div className="shrink-0">
      {showBanner && (
        <div className="mx-2 mb-1.5 px-2.5 py-2 border border-white/[0.08] bg-white/[0.03] rounded-md">
          <div className="flex items-center gap-1.5">
            <span className="w-[5px] h-[5px] rounded-full bg-bronzo shrink-0" />
            <span className="text-[11.5px] text-gray-200 truncate">
              {describeUpdateStatus(status).shortLabel}
            </span>
            <button
              onClick={() => setDismissed(true)}
              aria-label="Dismiss update notice"
              className="ml-auto text-gray-600 hover:text-gray-300 transition-colors text-[11px] leading-none shrink-0"
            >
              ✕
            </button>
          </div>
          <button
            onClick={() => window.api.installUpdate()}
            className="mt-2 w-full px-2 py-1 text-[11px] text-gray-300 bg-white/[0.04]
                       hover:bg-white/[0.08] border border-white/[0.08] rounded
                       transition-colors"
          >
            Restart to update
          </button>
        </div>
      )}

      <div
        className={`flex items-center gap-0.5 ${isCollapsed ? 'flex-col p-1.5' : 'px-2 py-1.5'}`}
      >
        <Tooltip label="Welcome Guide" position="right">
          <button
            onClick={() => setOnboardingOpen(true)}
            aria-label="Welcome Guide"
            className="p-1.5 text-gray-500 hover:text-gray-200 hover:bg-white/[0.04]
                       rounded-md transition-colors"
          >
            <CircleHelp size={16} strokeWidth={1.5} />
          </button>
        </Tooltip>
        <Tooltip
          label={showDot ? 'Settings — update ready' : 'Settings'}
          shortcut={settingsShortcut}
          position="right"
        >
          <button
            onClick={() => {
              if (showDot) {
                openUpdates()
                return
              }
              setSettingsOpen(true)
              closeSidebarOnMobile()
            }}
            aria-label={showDot ? 'Settings, update ready' : 'Settings'}
            className="relative p-1.5 text-gray-500 hover:text-gray-200 hover:bg-white/[0.04]
                       rounded-md transition-colors"
          >
            <Settings size={16} strokeWidth={1.5} />
            {showDot && (
              <span
                aria-hidden="true"
                className="absolute top-0.5 right-0.5 w-[5px] h-[5px] rounded-full bg-bronzo
                           ring-2 ring-surface-panel"
              />
            )}
          </button>
        </Tooltip>
      </div>
    </div>
  )
}
