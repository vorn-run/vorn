import { Plus } from 'lucide-react'
import { NODE_SELECTED } from '../node-visuals'

/** The + on the canvas: it opens the step library and stays lit while it is the anchor. */
export function ConnectorButton({ onOpen, active }: { onOpen: () => void; active?: boolean }) {
  return (
    <button
      aria-label="Add a step"
      onClick={(e) => {
        e.stopPropagation()
        onOpen()
      }}
      className={`w-[22px] h-[22px] rounded-full flex items-center justify-center
                  border transition-all z-10
                  ${
                    active
                      ? `bg-white/[0.08] ${NODE_SELECTED} text-white`
                      : 'bg-surface-node border-white/[0.1] text-gray-500 hover:border-white/[0.2] hover:text-white'
                  }`}
    >
      <Plus size={13} strokeWidth={2.5} />
    </button>
  )
}
