import { ICON_MAP } from './icon-map'

export function ProjectIcon({
  icon,
  color,
  size = 14
}: {
  icon?: string
  color?: string
  size?: number
}) {
  if (icon && ICON_MAP[icon]) {
    const Icon = ICON_MAP[icon]
    return <Icon size={size} color={color || '#6b7280'} strokeWidth={1.5} />
  }
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={color || 'currentColor'}
      strokeWidth="1.5"
    >
      <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
    </svg>
  )
}
