import { useCallback, useMemo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { Check, X, Workflow } from 'lucide-react'
import {
  WorkflowExecution,
  NodeExecutionState,
  WorkflowDefinition,
  ApprovalConfig
} from '../../shared/types'
import { useAppStore } from '../stores'
import { ICON_MAP } from './project-sidebar/icon-map'
import { approveWorkflowGate, rejectWorkflowGate } from '../lib/workflow-execution'
import { Tooltip } from './Tooltip'
import { ICON_BUTTON, ICON_BUTTON_DANGER } from '../lib/icon-button'

interface Props {
  execution: WorkflowExecution
  nodeState: NodeExecutionState
  workflow?: WorkflowDefinition
}

export function WaitingApprovalPill({ execution, nodeState, workflow }: Props) {
  const { setEditingWorkflowId, setWorkflowEditorOpen } = useAppStore(
    useShallow((s) => ({
      setEditingWorkflowId: s.setEditingWorkflowId,
      setWorkflowEditorOpen: s.setWorkflowEditorOpen
    }))
  )

  const node = useMemo(
    () => workflow?.nodes.find((n) => n.id === nodeState.nodeId),
    [workflow, nodeState.nodeId]
  )
  const message = node?.type === 'approval' ? (node.config as ApprovalConfig).message : undefined

  const WfIcon = workflow ? ICON_MAP[workflow.icon] || Workflow : Workflow
  const wfIconColor = workflow?.iconColor

  const handleOpen = useCallback(() => {
    setEditingWorkflowId(execution.workflowId)
    setWorkflowEditorOpen(true)
  }, [execution.workflowId, setEditingWorkflowId, setWorkflowEditorOpen])

  const handleApprove = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      void approveWorkflowGate(execution, nodeState.nodeId)
    },
    [execution, nodeState.nodeId]
  )

  const handleReject = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      void rejectWorkflowGate(execution, nodeState.nodeId)
    },
    [execution, nodeState.nodeId]
  )

  return (
    <div
      onClick={handleOpen}
      className="inline-flex items-center gap-1.5 rounded-md border bg-surface-raised px-2.5 py-1
                 cursor-pointer transition-colors select-none
                 border-bronzo/40 hover:border-bronzo/70"
    >
      <div className="relative flex-shrink-0">
        <div className="w-1.5 h-1.5 rounded-full bg-bronzo" />
        <div
          className="absolute inset-[-1px] rounded-full bg-bronzo opacity-40 animate-ping"
          style={{ animationDuration: '2s' }}
        />
      </div>

      <WfIcon size={12} strokeWidth={1.5} color={wfIconColor || undefined} />

      <span className="text-[11px] font-medium text-gray-200 truncate max-w-[140px]">
        {workflow?.name || 'Workflow'}
      </span>

      {message && (
        <>
          <span className="text-[10px] text-gray-600 flex-shrink-0">&middot;</span>
          <span className="text-[10px] text-gray-500 truncate max-w-[180px]">{message}</span>
        </>
      )}

      <div className="flex items-center gap-0.5 ml-1 flex-shrink-0">
        <Tooltip label="Approve">
          <button onClick={handleApprove} aria-label="Approve" className={ICON_BUTTON}>
            <Check size={12} strokeWidth={2.5} />
          </button>
        </Tooltip>
        <Tooltip label="Reject">
          <button onClick={handleReject} aria-label="Reject" className={ICON_BUTTON_DANGER}>
            <X size={12} strokeWidth={2.5} />
          </button>
        </Tooltip>
      </div>
    </div>
  )
}
