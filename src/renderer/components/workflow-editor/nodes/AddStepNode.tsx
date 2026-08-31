import { useState, useRef, useEffect } from 'react'
import { Plus, Play, GitFork, Terminal, Split, Hand, Zap, Repeat } from 'lucide-react'
import { NODE_GLYPH, NODE_SELECTED } from '../node-visuals'

interface Props {
  onAddAction: () => void
  onAddParallelBranch?: () => void
  onAddScript?: () => void
  onAddCondition?: () => void
  onAddApproval?: () => void
  onAddLoop?: () => void
  onAddConnectorAction?: () => void
}

export function ConnectorButton({
  onAddAction,
  onAddParallelBranch,
  onAddScript,
  onAddCondition,
  onAddApproval,
  onAddLoop,
  onAddConnectorAction
}: Props) {
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    function handleClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [open])

  return (
    <div className="relative flex flex-col items-center" ref={menuRef}>
      <button
        aria-label="Add a step"
        onClick={(e) => {
          e.stopPropagation()
          setOpen(!open)
        }}
        className={`w-[22px] h-[22px] rounded-full flex items-center justify-center
                    border transition-all z-10
                    ${
                      open
                        ? `bg-white/[0.08] ${NODE_SELECTED} text-white`
                        : 'bg-surface-node border-white/[0.1] text-gray-500 hover:border-white/[0.2] hover:text-white'
                    }`}
      >
        <Plus size={13} strokeWidth={2.5} />
      </button>

      {open && (
        <div
          className="absolute top-full mt-2 z-50 bg-surface-overlay border border-white/[0.12]
                     rounded-lg shadow-xl shadow-black/40 py-1.5 min-w-[200px]
                     animate-in fade-in-0 zoom-in-95 duration-100"
          style={{ left: '50%', transform: 'translateX(-50%)' }}
        >
          <button
            onClick={(e) => {
              e.stopPropagation()
              setOpen(false)
              onAddAction()
            }}
            className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                       hover:bg-white/[0.06] hover:text-white transition-colors text-left"
          >
            <Play size={14} className={`${NODE_GLYPH} shrink-0`} />
            Add an agent
          </button>

          {onAddScript && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                setOpen(false)
                onAddScript()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                         hover:bg-white/[0.06] hover:text-white transition-colors text-left"
            >
              <Terminal size={14} className={`${NODE_GLYPH} shrink-0`} />
              Add a script
            </button>
          )}

          {onAddCondition && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                setOpen(false)
                onAddCondition()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                         hover:bg-white/[0.06] hover:text-white transition-colors text-left"
            >
              <GitFork size={14} className={`${NODE_GLYPH} shrink-0`} />
              Add a condition
            </button>
          )}

          {onAddApproval && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                setOpen(false)
                onAddApproval()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                         hover:bg-white/[0.06] hover:text-white transition-colors text-left"
            >
              <Hand size={14} className={`${NODE_GLYPH} shrink-0`} />
              Add an approval gate
            </button>
          )}

          {onAddLoop && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                setOpen(false)
                onAddLoop()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                         hover:bg-white/[0.06] hover:text-white transition-colors text-left"
            >
              <Repeat size={14} className={`${NODE_GLYPH} shrink-0`} />
              Repeat steps until…
            </button>
          )}

          {onAddConnectorAction && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                setOpen(false)
                onAddConnectorAction()
              }}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                         hover:bg-white/[0.06] hover:text-white transition-colors text-left"
            >
              <Zap size={14} className={`${NODE_GLYPH} shrink-0`} />
              Call a connector action
            </button>
          )}

          {onAddParallelBranch && (
            <>
              <div className="h-px bg-white/[0.08] my-1" />
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  setOpen(false)
                  onAddParallelBranch()
                }}
                className="w-full flex items-center gap-2.5 px-3 py-2 text-[13px] text-gray-300
                           hover:bg-white/[0.06] hover:text-white transition-colors text-left"
              >
                <Split size={14} className={`${NODE_GLYPH} shrink-0`} />
                Add a parallel branch
              </button>
            </>
          )}
        </div>
      )}
    </div>
  )
}
