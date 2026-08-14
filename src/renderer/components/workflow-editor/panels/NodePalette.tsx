import { Zap, Clock, Play, ListPlus, ArrowRightLeft, Terminal } from 'lucide-react'
import type { TriggerConfig } from '../../../../shared/types'
import { NODE_GLYPH } from '../node-visuals'

interface Props {
  onAddTrigger: (type: TriggerConfig['triggerType']) => void
  onAddLaunchAgent: () => void
  onAddScript: () => void
  hasTrigger: boolean
}

export function NodePalette({ onAddTrigger, onAddLaunchAgent, onAddScript, hasTrigger }: Props) {
  return (
    <div className="w-[180px] border-r border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag">
      <div className="px-3 py-3 border-b border-white/[0.08]">
        <span className="text-[11px] font-medium text-gray-500 uppercase tracking-wider">
          Add Nodes
        </span>
      </div>

      <div className="flex-1 overflow-y-auto p-2 space-y-4">
        {/* Triggers */}
        {!hasTrigger && (
          <div>
            <span className="text-[10px] text-gray-600 uppercase tracking-wider font-medium px-1.5 block mb-1.5">
              Triggers
            </span>
            <div className="space-y-1">
              <PaletteItem
                icon={<Zap size={14} className={NODE_GLYPH} />}
                label="Manual"
                onClick={() => onAddTrigger('manual')}
              />
              <PaletteItem
                icon={<Clock size={14} className={NODE_GLYPH} />}
                label="Schedule"
                onClick={() => onAddTrigger('recurring')}
              />
              <PaletteItem
                icon={<ListPlus size={14} className={NODE_GLYPH} />}
                label="Task Created"
                onClick={() => onAddTrigger('taskCreated')}
              />
              <PaletteItem
                icon={<ArrowRightLeft size={14} className={NODE_GLYPH} />}
                label="Task Status Change"
                onClick={() => onAddTrigger('taskStatusChanged')}
              />
            </div>
          </div>
        )}

        {/* Actions */}
        <div>
          <span className="text-[10px] text-gray-600 uppercase tracking-wider font-medium px-1.5 block mb-1.5">
            Actions
          </span>
          <div className="space-y-1">
            <PaletteItem
              icon={<Play size={14} className={NODE_GLYPH} />}
              label="Launch Agent"
              onClick={onAddLaunchAgent}
            />
            <PaletteItem
              icon={<Terminal size={14} className={NODE_GLYPH} />}
              label="Run Script"
              onClick={onAddScript}
            />
          </div>
        </div>
      </div>
    </div>
  )
}

function PaletteItem({
  icon,
  label,
  onClick
}: {
  icon: React.ReactNode
  label: string
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-2 px-2.5 py-2 text-[12px] text-gray-300
                 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08]
                 rounded-md transition-colors text-left"
    >
      {icon}
      {label}
    </button>
  )
}
