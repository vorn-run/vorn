import { useState, useEffect, useRef } from 'react'
import {
  X,
  Zap,
  Play,
  Terminal,
  GitFork,
  Hand,
  ListPlus,
  Zap as ZapIcon,
  MoreHorizontal,
  Trash2
} from 'lucide-react'
import {
  WorkflowNode,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  CreateTaskFromItemConfig,
  CallConnectorActionConfig
} from '../../../../shared/types'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnectorIdFor } from '../../../lib/use-connections'
import { TriggerConfigForm } from './TriggerConfigForm'
import { LaunchAgentConfigForm } from './LaunchAgentConfigForm'
import { ScriptConfigForm } from './ScriptConfigForm'
import { ConditionConfigForm } from './ConditionConfigForm'
import { ApprovalConfigForm } from './ApprovalConfigForm'
import { CreateTaskFromItemNodeForm } from './CreateTaskFromItemNodeForm'
import { CallConnectorActionNodeForm } from './CallConnectorActionNodeForm'
import type { StepVariableGroup } from '../../../lib/template-vars'

const NODE_TYPE_CONFIG: Record<
  WorkflowNode['type'],
  { icon: typeof Zap; label: string; color: string; bg: string }
> = {
  trigger: { icon: Zap, label: 'Trigger', color: 'text-blue-400', bg: 'bg-blue-500/10' },
  launchAgent: { icon: Play, label: 'Agent', color: 'text-green-400', bg: 'bg-green-500/10' },
  script: { icon: Terminal, label: 'Script', color: 'text-amber-400', bg: 'bg-amber-500/10' },
  condition: {
    icon: GitFork,
    label: 'Condition',
    color: 'text-purple-400',
    bg: 'bg-purple-500/10'
  },
  approval: { icon: Hand, label: 'Approval', color: 'text-amber-400', bg: 'bg-amber-500/10' },
  createTaskFromItem: {
    icon: ListPlus,
    label: 'Create Task',
    color: 'text-gray-300',
    bg: 'bg-white/[0.06]'
  },
  callConnectorAction: {
    icon: ZapIcon,
    label: 'Connector Action',
    color: 'text-gray-300',
    bg: 'bg-white/[0.06]'
  }
}

interface Props {
  node: WorkflowNode
  allNodes?: WorkflowNode[]
  onChange: (nodeId: string, config: WorkflowNode['config']) => void
  onLabelChange: (nodeId: string, label: string) => void
  onDelete: (nodeId: string) => void
  onClose: () => void
  triggerType?: TriggerConfig['triggerType']
  isContextualTrigger?: boolean
  stepGroups?: StepVariableGroup[]
}

export function NodeConfigPanel({
  node,
  allNodes,
  onChange,
  onLabelChange,
  onDelete,
  onClose,
  triggerType,
  isContextualTrigger,
  stepGroups
}: Props) {
  const [showMenu, setShowMenu] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!showMenu) return
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false)
      }
    }
    document.addEventListener('pointerdown', handler)
    return () => document.removeEventListener('pointerdown', handler)
  }, [showMenu])

  const tc = NODE_TYPE_CONFIG[node.type]
  const Icon = tc.icon
  const canDelete = node.type !== 'trigger'

  // For connector-action nodes the generic Zap is uninformative — show the
  // connector's own mark (GitHub / Linear / MCP / …) by looking the selected
  // connection up. Falls back to Zap when no connection is chosen yet.
  const connectorConfig =
    node.type === 'callConnectorAction' ? (node.config as CallConnectorActionConfig) : null
  const headerConnectorId = useConnectorIdFor(connectorConfig?.connectionId)

  return (
    <div className="w-[420px] border-l border-white/[0.08] bg-[#1e1e22] flex flex-col h-full overflow-hidden titlebar-no-drag">
      <div className="flex items-center gap-2 px-5 py-3 border-b border-white/[0.08]">
        {headerConnectorId ? (
          <ConnectorIcon
            connectorId={headerConnectorId}
            size={14}
            className="text-gray-300 shrink-0"
          />
        ) : (
          <Icon size={14} className={`${tc.color} shrink-0`} />
        )}
        <input
          type="text"
          value={node.label}
          onChange={(e) => onLabelChange(node.id, e.target.value)}
          className="flex-1 min-w-0 text-[13px] font-medium text-white bg-transparent border-none outline-none
                     hover:bg-white/[0.04] focus:bg-white/[0.06] px-1.5 py-0.5 rounded transition-colors -ml-1.5"
          placeholder="Label"
        />
        <div className="flex items-center gap-0.5 shrink-0">
          {canDelete && (
            <div className="relative">
              <button
                onClick={() => setShowMenu(!showMenu)}
                aria-label="More node actions"
                aria-haspopup="menu"
                aria-expanded={showMenu}
                className="text-gray-500 hover:text-white p-1 rounded-md transition-colors"
              >
                <MoreHorizontal size={14} />
              </button>
              {showMenu && (
                <div
                  ref={menuRef}
                  className="absolute right-0 top-full mt-1 z-50 min-w-[160px] py-1 border border-white/[0.08] rounded-lg shadow-xl"
                  style={{ background: '#141416' }}
                >
                  <button
                    onClick={() => {
                      setShowMenu(false)
                      onDelete(node.id)
                    }}
                    className="w-full px-3 py-2 text-left text-[12px] text-red-400 hover:text-red-300
                               hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
                  >
                    <Trash2 size={12} strokeWidth={1.5} />
                    Remove action
                  </button>
                </div>
              )}
            </div>
          )}
          <button
            onClick={onClose}
            aria-label="Close node config"
            className="text-gray-500 hover:text-white p-1 rounded-md transition-colors"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-5 space-y-5">
        {node.slug && node.type !== 'trigger' && (
          <p className="text-[10px] text-gray-600 font-mono -mt-2 mb-3">
            Ref: {`{{steps.${node.slug}.output}}`}
          </p>
        )}

        {node.type === 'trigger' && (
          <TriggerConfigForm
            config={node.config as TriggerConfig}
            onChange={(config) => onChange(node.id, config)}
          />
        )}

        {node.type === 'launchAgent' && (
          <LaunchAgentConfigForm
            config={node.config as LaunchAgentConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            isContextualTrigger={isContextualTrigger}
            stepGroups={stepGroups}
            currentNodeId={node.id}
            allNodes={allNodes}
          />
        )}

        {node.type === 'script' && (
          <ScriptConfigForm
            config={node.config as ScriptConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            isContextualTrigger={isContextualTrigger}
            stepGroups={stepGroups}
          />
        )}

        {node.type === 'condition' && (
          <ConditionConfigForm
            config={node.config as ConditionConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            stepGroups={stepGroups || []}
          />
        )}

        {node.type === 'approval' && (
          <ApprovalConfigForm
            config={node.config as ApprovalConfig}
            onChange={(config) => onChange(node.id, config)}
          />
        )}

        {node.type === 'createTaskFromItem' && (
          <CreateTaskFromItemNodeForm
            config={node.config as CreateTaskFromItemConfig}
            onChange={(config) => onChange(node.id, config)}
          />
        )}

        {node.type === 'callConnectorAction' && (
          <CallConnectorActionNodeForm
            config={node.config as CallConnectorActionConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            stepGroups={stepGroups}
          />
        )}
      </div>
    </div>
  )
}
