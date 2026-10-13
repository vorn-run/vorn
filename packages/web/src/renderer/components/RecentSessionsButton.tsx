import { useState } from 'react'
import { RotateCcw } from 'lucide-react'
import { Tooltip } from './Tooltip'
import { RecentSessionsPopover } from './RecentSessionsPopover'

export function RecentSessionsButton() {
  const [open, setOpen] = useState(false)
  return (
    <div className="relative flex items-center">
      <Tooltip label="Recent sessions" position="bottom">
        <button
          onClick={() => setOpen(!open)}
          className="p-1 text-gray-400 hover:text-white hover:bg-white/[0.06] rounded-md transition-colors"
          aria-label="Recent sessions"
        >
          <RotateCcw size={16} strokeWidth={1.5} />
        </button>
      </Tooltip>
      <RecentSessionsPopover isOpen={open} onClose={() => setOpen(false)} />
    </div>
  )
}
