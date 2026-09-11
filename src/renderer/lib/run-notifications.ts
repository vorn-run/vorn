import type { WorkflowExecution } from '../../shared/types'
import { isSignInWait } from '@vornrun/shared/workflow-graph'
import { useAppStore } from '../stores'
import { sendWorkflowGateNotification, sendWorkflowSignInNotification } from './notifications'

/**
 * The window's half of a run that is happening somewhere else.
 *
 * Runs execute in the server, which has no notifications, no sound and no
 * window to focus -- so the announcing stays here, driven by the run updates
 * arriving over the wire. What is announced is a *transition*: a gate that has
 * just started waiting, a run that has just ended. Every update carries the
 * whole run, so without remembering what was last seen, one gate would be
 * announced on every step that followed it.
 */
const lastSeen = new Map<string, { status: string; waiting: Set<string> }>()

function waitingNodes(execution: WorkflowExecution): Set<string> {
  return new Set(
    execution.nodeStates.filter((ns) => ns.status === 'waiting').map((ns) => ns.nodeId)
  )
}

export function announceRun(execution: WorkflowExecution): void {
  const store = useAppStore.getState()
  const workflow = store.config?.workflows?.find((w) => w.id === execution.workflowId)
  const previous = lastSeen.get(execution.runId)
  const waiting = waitingNodes(execution)

  if (workflow) {
    for (const nodeId of waiting) {
      if (previous?.waiting.has(nodeId)) continue
      const node = workflow.nodes.find((n) => n.id === nodeId)
      const openWorkflow = () => {
        useAppStore.getState().setEditingWorkflowId(workflow.id)
        useAppStore.getState().setWorkflowEditorOpen(true)
      }
      const state = execution.nodeStates.find((ns) => ns.nodeId === nodeId)
      if (state && isSignInWait(state)) {
        sendWorkflowSignInNotification(
          workflow,
          nodeId,
          node?.label ?? 'A step',
          state.error,
          store.config ?? null,
          openWorkflow
        )
        continue
      }
      sendWorkflowGateNotification(
        workflow,
        nodeId,
        node?.label ?? 'Approval',
        (node?.config as { message?: string } | undefined)?.message,
        store.config ?? null,
        openWorkflow
      )
    }
  }

  const ended = execution.status === 'success' || execution.status === 'error'
  if (ended && previous && previous.status === 'running' && workflow) {
    const steps = execution.nodeStates.filter((ns) => ns.status !== 'pending').length
    if (Notification.permission === 'granted') {
      new Notification('Vorn', {
        body: `Workflow "${workflow.name}" ${execution.status === 'success' ? 'completed' : 'failed'} — ${steps} step${steps === 1 ? '' : 's'}`
      })
    }
  }

  // Cancelled counts as over here even though it raises nothing: no further
  // update arrives for a stopped run, so anything kept would be kept forever.
  if (execution.status !== 'running') lastSeen.delete(execution.runId)
  else lastSeen.set(execution.runId, { status: execution.status, waiting })
}

/** Test seam: a fresh window has seen nothing. */
export function resetAnnouncedRuns(): void {
  lastSeen.clear()
}
