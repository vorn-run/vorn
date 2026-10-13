import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'
import { isMac } from '../lib/platform'

export function SidebarToggleButton() {
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  return (
    <Tooltip label="Toggle sidebar" shortcut={`${isMac ? '⌘' : 'Ctrl+'}B`} position="bottom">
      <button
        onClick={toggleSidebar}
        className="text-gray-400 hover:text-white active:text-white p-1 transition-colors rounded-md"
        title="Show sidebar"
        aria-label="Toggle sidebar"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="M9 3v18" />
        </svg>
      </button>
    </Tooltip>
  )
}
