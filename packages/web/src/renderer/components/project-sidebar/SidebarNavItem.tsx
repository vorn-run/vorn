import type { ReactNode } from 'react'

export function SidebarNavItem({
  isActive,
  isCollapsed,
  icon,
  label,
  badge,
  onClick
}: {
  isActive: boolean
  isCollapsed: boolean
  icon: ReactNode
  label: string
  badge?: ReactNode
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      className={`w-full text-left px-2 py-1.5 rounded-md text-[13px] transition-colors flex items-center gap-2 ${
        isActive
          ? 'bg-white/[0.08] text-white'
          : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
      } ${isCollapsed ? 'justify-center px-0' : ''}`}
      title={isCollapsed ? label : undefined}
    >
      <span className="shrink-0 flex items-center">{icon}</span>
      {!isCollapsed && (
        <>
          <span className="truncate">{label}</span>
          {badge != null && <span className="text-gray-500 text-xs ml-auto">{badge}</span>}
        </>
      )}
    </button>
  )
}
