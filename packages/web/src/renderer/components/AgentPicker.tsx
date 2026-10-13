import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Check, ChevronDown, Bot, ClipboardList } from 'lucide-react'
import { AiAgentType, LaunchAgentType } from '../../shared/types'
import { AgentIcon } from './AgentIcon'
import { useAnchoredMenu } from '../hooks/useAnchoredMenu'

const MENU_ITEM_PX = 28
const MENU_PADDING_PX = 8

const AGENT_LABELS: Record<AiAgentType, string> = {
  claude: 'Claude',
  copilot: 'Copilot',
  codex: 'Codex',
  opencode: 'OpenCode',
  gemini: 'Gemini'
}

export function AgentPicker({
  currentAgent,
  onChange,
  installStatus,
  variant = 'compact',
  allowNone = false,
  allowFromTask = false
}: {
  currentAgent: LaunchAgentType | null
  onChange: (agent: LaunchAgentType | null) => void
  installStatus: Record<AiAgentType, boolean>
  variant?: 'compact' | 'form'
  allowNone?: boolean
  allowFromTask?: boolean
}) {
  const agents = Object.keys(AGENT_LABELS) as AiAgentType[]
  const itemCount = agents.length + (allowNone ? 1 : 0) + (allowFromTask ? 1 : 0)
  const { open, setOpen, toggle, triggerRef, menuRef, position } = useAnchoredMenu({
    estimateHeight: () => itemCount * MENU_ITEM_PX + MENU_PADDING_PX
  })

  const handleSelect = (agent: LaunchAgentType | null) => {
    if (agent && agent !== 'fromTask' && !installStatus[agent]) return
    setOpen(false)
    if (agent !== currentAgent) {
      onChange(agent)
    }
  }

  return (
    <>
      <button
        ref={triggerRef}
        onClick={toggle}
        className={
          variant === 'form'
            ? 'w-full flex items-center gap-2 px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md text-white hover:border-white/[0.2] transition-colors'
            : 'flex items-center gap-1.5 rounded-[4px] border border-white/[0.12] px-1.5 py-0.5 text-[12px] text-gray-300 transition-colors hover:border-white/25 hover:bg-white/[0.04]'
        }
      >
        {currentAgent === 'fromTask' ? (
          <ClipboardList size={14} className="text-blue-400" />
        ) : currentAgent ? (
          <AgentIcon agentType={currentAgent} size={14} />
        ) : (
          <Bot size={14} className="text-gray-500" />
        )}
        <span className={`flex-1 text-left ${currentAgent ? '' : 'text-gray-600'}`}>
          {currentAgent === 'fromTask'
            ? 'From Task'
            : currentAgent
              ? AGENT_LABELS[currentAgent]
              : 'Unassigned'}
        </span>
        <ChevronDown size={11} className="text-gray-500" />
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              initial={{ opacity: 0, y: -4, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -4, scale: 0.96 }}
              transition={{ type: 'spring', stiffness: 500, damping: 30 }}
              className="fixed z-[150] rounded-lg border border-white/[0.1] shadow-2xl py-1"
              style={{
                top: position.top,
                bottom: position.bottom,
                left: position.left,
                minWidth: Math.max(180, position.width),
                background: 'var(--color-surface-overlay)'
              }}
            >
              {allowNone && (
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    handleSelect(null)
                  }}
                  className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-500 hover:bg-white/[0.06] transition-colors"
                >
                  <Bot size={14} className="text-gray-600" />
                  <span className="flex-1 text-left italic">None</span>
                  {!currentAgent && <Check size={13} className="text-gray-400" />}
                </button>
              )}
              {allowFromTask && (
                <>
                  <button
                    onClick={(e) => {
                      e.stopPropagation()
                      handleSelect('fromTask')
                    }}
                    className="w-full flex items-center gap-2.5 px-3 py-1.5 text-xs text-gray-300 hover:bg-white/[0.06] transition-colors"
                    title="Resolved from the task's assigned agent at run time"
                  >
                    <ClipboardList size={14} className="text-blue-400" />
                    <span className="flex-1 text-left">From Task</span>
                    {currentAgent === 'fromTask' && <Check size={13} className="text-gray-400" />}
                  </button>
                  <div className="border-t border-white/[0.06] my-1" />
                </>
              )}
              {agents.map((agent) => {
                const installed = installStatus[agent]
                const isCurrent = agent === currentAgent
                return (
                  <button
                    key={agent}
                    onClick={(e) => {
                      e.stopPropagation()
                      handleSelect(agent)
                    }}
                    disabled={!installed}
                    className={`w-full flex items-center gap-2.5 px-3 py-1.5 text-xs transition-colors ${
                      !installed
                        ? 'text-gray-600 cursor-not-allowed'
                        : 'text-gray-300 hover:bg-white/[0.06] cursor-pointer'
                    }`}
                    title={!installed ? `${agent} is not installed` : undefined}
                  >
                    <AgentIcon agentType={agent} size={14} />
                    <span className="flex-1 text-left">{AGENT_LABELS[agent]}</span>
                    {!installed && <span className="text-[10px] text-gray-600">Not installed</span>}
                    {isCurrent && installed && <Check size={13} className="text-gray-400" />}
                  </button>
                )
              })}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  )
}
