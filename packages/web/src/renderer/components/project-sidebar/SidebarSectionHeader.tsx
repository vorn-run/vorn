import type { ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

export function SidebarSectionHeader({
  title,
  isCollapsed,
  sectionCollapsed,
  onToggle,
  actions
}: {
  title: string
  isCollapsed: boolean
  sectionCollapsed: boolean
  onToggle: () => void
  actions?: ReactNode
}) {
  if (isCollapsed) {
    return <div className="pt-4" />
  }

  return (
    <div className="group/section pt-3 pb-1.5 flex items-center justify-between">
      <button
        onClick={onToggle}
        className="flex items-center gap-1.5 hover:text-gray-300 transition-colors"
      >
        <ChevronRight
          size={10}
          strokeWidth={2}
          className={`text-gray-600 transition-transform ${sectionCollapsed ? '' : 'rotate-90'}`}
        />
        <span className="text-[11px] font-medium text-gray-500 uppercase tracking-wider">
          {title}
        </span>
      </button>
      {actions && <div className="flex items-center gap-0.5">{actions}</div>}
    </div>
  )
}
