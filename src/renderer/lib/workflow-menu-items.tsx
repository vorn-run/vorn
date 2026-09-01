import { type ReactNode } from 'react'
import { Workflow } from 'lucide-react'
import { ICON_MAP } from '../components/project-sidebar/icon-map'
import { executeWorkflow } from './workflow-execution'
import { isContextualWorkflow, needsRunPrompt } from './workflow-helpers'
import type { ManualRunContext } from './workflow-helpers'
import { useAppStore } from '../stores'
import { toast } from '../components/Toast'
import type { WorkflowDefinition } from '../../shared/types'

export interface WorkflowMenuItem {
  id: string
  iconElement: ReactNode
  label: string
  detail?: string
  onClick: () => void
  separator?: boolean
  isHeader?: boolean
}

/**
 * Caller-supplied context for a workflow menu. When `task` or `source` is
 * present, the menu lists only contextual workflows (those whose manual
 * trigger has `contextual: true`); when both are absent (e.g. empty grid
 * right-click) it lists only non-contextual workflows so users don't see
 * actions that would dead-end on a missing source.
 */
export type WorkflowMenuContext = ManualRunContext

/**
 * The single door for starting a workflow from the UI.
 *
 * Anything the user still has to supply — a source folder, or declared run
 * inputs — opens the run dialog first; everything else launches immediately.
 * Every manual surface (sidebar, palette, card/terminal menus, the editor's
 * Run button) must go through here rather than calling `executeWorkflow`
 * directly: skipping the prompt is silent, and produces a run whose
 * `{{inputs.*}}` templates reach the agent unresolved.
 *
 * `executeWorkflow` itself stays unguarded because the scheduler, connector
 * triggers and missed-schedule recovery legitimately run without a user.
 */
export function startManualRun(
  workflow: WorkflowDefinition,
  ctx?: ManualRunContext,
  options?: { targetNodeId?: string }
): void {
  // Every manual surface funnels here; a startless workflow cannot run.
  if (!workflow.nodes.some((n) => n.type === 'trigger')) {
    toast.error(`"${workflow.name}" has no trigger — add one in the editor first`)
    return
  }
  if (needsRunPrompt(workflow, ctx)) {
    useAppStore.getState().setPendingWorkflowRun(workflow.id, ctx, options?.targetNodeId)
    return
  }
  void executeWorkflow(workflow, ctx, { source: 'manual', targetNodeId: options?.targetNodeId })
}

export function buildWorkflowMenuItems(
  workflows: WorkflowDefinition[],
  onSelect: () => void,
  context?: WorkflowMenuContext
): WorkflowMenuItem[] {
  const hasContext = !!(context?.task || context?.source)
  const filtered = workflows.filter((wf) =>
    hasContext ? isContextualWorkflow(wf) : !isContextualWorkflow(wf)
  )
  return filtered.map((wf) => {
    const WfIcon = ICON_MAP[wf.icon] || Workflow
    return {
      id: wf.id,
      iconElement: <WfIcon size={12} color={wf.iconColor} />,
      label: wf.name,
      onClick: () => {
        onSelect()
        startManualRun(
          wf,
          hasContext ? { task: context?.task, source: context?.source } : undefined
        )
      }
    }
  })
}
