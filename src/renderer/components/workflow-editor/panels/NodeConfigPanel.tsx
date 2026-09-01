import { useState, useEffect, useRef } from 'react'
import { X, MoreHorizontal, Trash2, Replace, StepForward } from 'lucide-react'
import {
  WorkflowNode,
  LoopConfig,
  TriggerConfig,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ApprovalConfig,
  CreateTaskFromItemConfig,
  CallConnectorActionConfig,
  HttpRequestConfig,
  WorkflowNodeErrorPolicy
} from '../../../../shared/types'
import { ConnectorIcon } from '../../ConnectorIcon'
import { REPLACEABLE_NODE_TYPES } from '../../../lib/workflow-helpers'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { TriggerConfigForm } from './TriggerConfigForm'
import { LaunchAgentConfigForm } from './LaunchAgentConfigForm'
import { ScriptConfigForm } from './ScriptConfigForm'
import { ConditionConfigForm } from './ConditionConfigForm'
import { ApprovalConfigForm } from './ApprovalConfigForm'
import { LoopConfigForm } from './LoopConfigForm'
import { CreateTaskFromItemNodeForm } from './CreateTaskFromItemNodeForm'
import { CallConnectorActionNodeForm } from './CallConnectorActionNodeForm'
import { HttpRequestConfigForm } from './HttpRequestConfigForm'
import { NODE_TYPE_ICON, NODE_GLYPH } from '../node-visuals'
import type { StepVariableGroup, TemplateVariable } from '../../../lib/template-vars'

interface Props {
  /** Open the step library in trigger scope; shown on trigger nodes. */
  onOpenTriggerLibrary?: () => void
  /** Open the step library to swap this step in place; shown on replaceable steps. */
  onOpenReplaceLibrary?: (nodeId: string) => void
  node: WorkflowNode
  allNodes?: WorkflowNode[]
  onChange: (nodeId: string, config: WorkflowNode['config']) => void
  onLabelChange: (nodeId: string, label: string) => void
  onErrorChange?: (nodeId: string, policy: WorkflowNodeErrorPolicy) => void
  onDelete: (nodeId: string) => void
  onClose: () => void
  triggerType?: TriggerConfig['triggerType']
  isContextualTrigger?: boolean
  /** Autocomplete entries for the workflow's declared manual-run inputs. */
  inputVars?: TemplateVariable[]
  stepGroups?: StepVariableGroup[]
  /** Run only this step and its upstream slice; absent when the step is not a valid target. */
  onRunToStep?: (nodeId: string) => void
}

export function NodeConfigPanel({
  onOpenTriggerLibrary,
  onOpenReplaceLibrary,
  node,
  onRunToStep,
  allNodes,
  onChange,
  onLabelChange,
  onErrorChange,
  onDelete,
  onClose,
  triggerType,
  isContextualTrigger,
  inputVars,
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

  const tc = NODE_TYPE_ICON[node.type]
  const Icon = tc

  // For connector-action nodes the generic Zap is uninformative — show the
  // connector's own mark (GitHub / Linear / MCP / …) by looking the selected
  // connection up. Falls back to Zap when no connection is chosen yet.
  const connectorConfig =
    node.type === 'callConnectorAction' ? (node.config as CallConnectorActionConfig) : null
  const headerConnectorId = useConnectorIdFor(connectorConfig?.connectionId)
  const headerConnectorIcon = useConnectionIconFor(connectorConfig?.connectionId)

  return (
    <div className="w-[420px] border-l border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag">
      <div className="flex items-center gap-2 px-5 py-3 border-b border-white/[0.08]">
        {headerConnectorId ? (
          <ConnectorIcon
            connectorId={headerConnectorId}
            icon={headerConnectorIcon}
            size={14}
            className={`${NODE_GLYPH} shrink-0`}
          />
        ) : (
          <Icon size={14} className={`${NODE_GLYPH} shrink-0`} />
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
                style={{ background: 'var(--color-surface-overlay)' }}
              >
                {onOpenReplaceLibrary && REPLACEABLE_NODE_TYPES.has(node.type) && (
                  <button
                    onClick={() => {
                      setShowMenu(false)
                      onOpenReplaceLibrary(node.id)
                    }}
                    className="w-full px-3 py-2 text-left text-[12px] text-gray-300 hover:text-white
                                 hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
                  >
                    <Replace size={12} strokeWidth={1.5} />
                    Replace step
                  </button>
                )}
                <button
                  onClick={() => {
                    setShowMenu(false)
                    onDelete(node.id)
                  }}
                  className="w-full px-3 py-2 text-left text-[12px] text-danger/80 hover:text-danger
                               hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
                >
                  <Trash2 size={12} strokeWidth={1.5} />
                  {node.type === 'trigger' ? 'Remove trigger' : 'Remove action'}
                </button>
              </div>
            )}
          </div>
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
        {node.type === 'trigger' && (
          <TriggerConfigForm
            config={node.config as TriggerConfig}
            onChange={(config) => onChange(node.id, config)}
            onOpenLibrary={onOpenTriggerLibrary}
          />
        )}

        {node.type === 'launchAgent' && (
          <LaunchAgentConfigForm
            inputVars={inputVars}
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
            inputVars={inputVars}
            config={node.config as ScriptConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            isContextualTrigger={isContextualTrigger}
            stepGroups={stepGroups}
          />
        )}

        {node.type === 'condition' && (
          <ConditionConfigForm
            inputVars={inputVars}
            config={node.config as ConditionConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            stepGroups={stepGroups || []}
          />
        )}

        {node.type === 'loop' && (
          <LoopConfigForm
            config={node.config as LoopConfig}
            onChange={(config) => onChange(node.id, config)}
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

        {node.type === 'httpRequest' && (
          <HttpRequestConfigForm
            inputVars={inputVars}
            config={node.config as HttpRequestConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            isContextualTrigger={isContextualTrigger}
            stepGroups={stepGroups}
          />
        )}

        {node.type === 'callConnectorAction' && (
          <CallConnectorActionNodeForm
            inputVars={inputVars}
            config={node.config as CallConnectorActionConfig}
            onChange={(config) => onChange(node.id, config)}
            triggerType={triggerType}
            stepGroups={stepGroups}
          />
        )}

        {/* A trigger has nothing downstream of its own failure to govern. */}
        {node.type !== 'trigger' && onErrorChange && (
          <div className="pt-4 border-t border-white/[0.08]">
            <label
              htmlFor={`onError-${node.id}`}
              className="block text-[11px] text-gray-500 mb-1.5"
            >
              If this step fails
            </label>
            {/* Tailwind's gray-900 and gray-700 are blue-tinted (#111827,
                #374151), so this control read as the one blue thing in the
                editor. Every other select in these panels uses this wash. */}
            <select
              id={`onError-${node.id}`}
              value={node.onError ?? 'stop'}
              onChange={(e) => onErrorChange(node.id, e.target.value as WorkflowNodeErrorPolicy)}
              className="w-full px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                         text-[13px] text-gray-200 focus:outline-none focus:border-white/[0.2]"
            >
              <option value="stop">Stop the run</option>
              <option value="continue">Carry on anyway</option>
            </select>
            <p className="text-[10px] text-gray-600 mt-1.5">
              {(node.onError ?? 'stop') === 'stop'
                ? 'Everything downstream is skipped and the run ends as failed.'
                : 'Later steps run as if this one had succeeded — for a step whose failure does not invalidate the rest.'}
            </p>
          </div>
        )}
      </div>

      {onRunToStep && (
        <div className="shrink-0 border-t border-white/[0.08] p-3">
          <button
            onClick={() => onRunToStep(node.id)}
            className="w-full flex items-center justify-center gap-2 border border-white/[0.12] rounded-md
                       px-3 py-1.5 text-[12px] text-gray-300 hover:text-white hover:border-white/[0.2]
                       transition-colors"
          >
            <StepForward size={12} strokeWidth={2} />
            Run to this step
          </button>
        </div>
      )}
    </div>
  )
}
