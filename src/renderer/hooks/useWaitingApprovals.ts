import { useMemo } from 'react'
import { isSignInWait } from '@vornrun/shared/workflow-graph'
import { useAppStore } from '../stores'
import type { WorkflowExecution, NodeExecutionState, WorkflowDefinition } from '../../shared/types'

export interface WaitingApproval {
  execution: WorkflowExecution
  nodeState: NodeExecutionState
  workflow?: WorkflowDefinition
}

const EMPTY: WaitingApproval[] = []

/**
 * Selector returns a string signature of the waiting-gate set so the hook only
 * re-runs when a gate enters or exits `waiting` — not on every log-chunk
 * mutation of `workflowExecutions`.
 */
function waitingSignature(
  workflowExecutions: Map<
    string,
    { nodeStates: Array<{ nodeId: string; status: string; waitingFor?: string }> }
  >
): string {
  const parts: string[] = []
  for (const [id, exec] of workflowExecutions) {
    for (const ns of exec.nodeStates) {
      if (ns.status === 'waiting' && !isSignInWait(ns)) parts.push(`${id}:${ns.nodeId}`)
    }
  }
  parts.sort()
  return parts.join(',')
}

export function useWaitingApprovals(): WaitingApproval[] {
  const signature = useAppStore((s) => waitingSignature(s.workflowExecutions))
  const workflows = useAppStore((s) => s.config?.workflows)
  const activeWorkspace = useAppStore((s) => s.activeWorkspace)

  return useMemo(() => {
    if (signature === '') return EMPTY
    const out: WaitingApproval[] = []
    const executions = useAppStore.getState().workflowExecutions
    for (const execution of executions.values()) {
      const workflow = workflows?.find((w) => w.id === execution.workflowId)
      if (workflow && (workflow.workspaceId ?? 'personal') !== activeWorkspace) continue
      for (const ns of execution.nodeStates) {
        if (ns.status === 'waiting' && !isSignInWait(ns)) {
          out.push({ execution, nodeState: ns, workflow })
        }
      }
    }
    return out
  }, [signature, workflows, activeWorkspace])
}
