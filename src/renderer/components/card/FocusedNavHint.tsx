import { useShallow } from 'zustand/react/shallow'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { MOD } from '../../lib/platform'
import { ICON_BUTTON } from '../../lib/icon-button'

interface Props {
  terminalId: string
}

export function FocusedNavHint({ terminalId }: Props) {
  const { focusableTerminalIds, setFocusedTerminal } = useAppStore(
    useShallow((s) => ({
      focusableTerminalIds: s.focusableTerminalIds,
      setFocusedTerminal: s.setFocusedTerminal
    }))
  )

  if (focusableTerminalIds.length < 2) return null

  const index = focusableTerminalIds.indexOf(terminalId)
  if (index === -1) return null

  const total = focusableTerminalIds.length
  const prevId = focusableTerminalIds[(index - 1 + total) % total]
  const nextId = focusableTerminalIds[(index + 1) % total]

  return (
    <div className="flex items-center gap-1 shrink-0 pr-1">
      <span className="text-[11px] font-mono text-gray-500 tabular-nums select-none">
        {index + 1}
        <span className="text-gray-700"> / </span>
        {total}
      </span>
      <Tooltip label="Previous session" shortcut={`${MOD}[`} position="bottom">
        <button
          type="button"
          onClick={() => setFocusedTerminal(prevId)}
          onPointerDown={(e) => e.stopPropagation()}
          className={ICON_BUTTON}
          aria-label="Previous session"
        >
          <ChevronLeft size={14} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Next session" shortcut={`${MOD}]`} position="bottom">
        <button
          type="button"
          onClick={() => setFocusedTerminal(nextId)}
          onPointerDown={(e) => e.stopPropagation()}
          className={ICON_BUTTON}
          aria-label="Next session"
        >
          <ChevronRight size={14} strokeWidth={2} />
        </button>
      </Tooltip>
    </div>
  )
}
