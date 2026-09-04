import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { getShortcut } from '../../lib/keyboard-shortcuts'
import { describeUpdateStatus, hasPendingUpdate } from '../../lib/update-status'
import { facesRestart, updateCostLine } from '../../lib/update-cost'
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
  // Selected down to what the footer actually draws rather than the whole
  // status object: download-progress fires many times a second, and a new
  // object identity each time would re-render this subtree throughout a
  // download to produce exactly the same output.
  const pending = useAppStore((s) => hasPendingUpdate(s.appUpdateStatus))
  const shortLabel = useAppStore((s) =>
    hasPendingUpdate(s.appUpdateStatus) ? describeUpdateStatus(s.appUpdateStatus).shortLabel : null
  )
  // Selected as two primitives for the same reason as the status above: this
  // subtree must not re-render on every keystroke a session prints.
  const sessionCount = useAppStore((s) => [...s.terminals.values()].filter(facesRestart).length)
  const aTurnIsRunning = useAppStore((s) =>
    [...s.terminals.values()].some((t) => facesRestart(t) && t.status === 'running')
  )
  const dismissed = useAppStore((s) => s.updateBannerDismissed)
  const setDismissed = useAppStore((s) => s.setUpdateBannerDismissed)

  const cost = updateCostLine(sessionCount, aTurnIsRunning)

  const settingsShortcut = getShortcut('settings')?.display
  // Collapsed the rail is 52px, which the banner cannot live in; dismissed the
  // user asked for it gone. Both keep the dot, so the update stays findable
  // without the banner — the old top banner simply vanished until relaunch.
  const showBanner = pending && !dismissed && !isCollapsed
  const showDot = pending && (dismissed || isCollapsed)

  return (
    <div className="shrink-0">
      {showBanner && (
        <div className="mx-2 mb-1.5 px-2.5 py-2 border border-white/[0.08] bg-white/[0.03] rounded-md">
          <div className="flex items-center gap-1.5">
            <span className="w-[5px] h-[5px] rounded-full bg-bronzo shrink-0" />
            <span className="text-[11.5px] text-gray-200 truncate">{shortLabel}</span>
            <button
              onClick={() => setDismissed(true)}
              aria-label="Dismiss update notice"
              className="ml-auto text-gray-600 hover:text-gray-300 transition-colors text-[11px] leading-none shrink-0"
            >
              ✕
            </button>
          </div>
          {/* The same sentence the Updates panel gives, because this button does
              the same thing. Without it the shortcut is the quieter way to end
              every session, which is the wrong way round. */}
          {cost && <div className="mt-1.5 text-[10.5px] leading-snug text-bronzo">{cost}</div>}
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
              // The panel opens on whatever category was last viewed, so a
              // badged gear has to name the one it is advertising.
              if (showDot) setSettingsCategory('updates')
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
