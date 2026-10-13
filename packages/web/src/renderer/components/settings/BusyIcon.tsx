import { Loader2, type LucideIcon } from 'lucide-react'

// A button says it is working in the place it already draws its icon.
export function BusyIcon({
  busy,
  icon: Icon,
  size
}: {
  busy: boolean
  icon?: LucideIcon
  size: number
}): React.ReactElement | null {
  if (busy) return <Loader2 size={size} className="animate-spin shrink-0" />
  return Icon ? <Icon size={size} /> : null
}
