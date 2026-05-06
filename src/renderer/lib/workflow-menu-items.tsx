import { type ReactNode } from 'react'
import { Zap } from 'lucide-react'
import { ICON_MAP } from '../components/project-sidebar/icon-map'
import { executeWorkflow } from './workflow-execution'
import { isContextualWorkflow } from './workflow-helpers'
import { useAppStore } from '../stores'
import type { TaskConfig, TerminalSession, WorkflowDefinition } from '../../shared/types'

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
export interface WorkflowMenuContext {
  task?: TaskConfig
  source?: TerminalSession
}

/**
 * Routes a workflow run from a non-contextual surface (sidebar, palette).
 * Contextual workflows open SourcePromptDialog so the user picks the source;
 * everything else launches immediately. Centralized here so each call site
 * doesn't need to repeat the gating logic.
 */
export function runWorkflowFromGlobalSurface(workflow: WorkflowDefinition): void {
  if (isContextualWorkflow(workflow)) {
    useAppStore.getState().setPendingContextualWorkflowId(workflow.id)
    return
  }
  void executeWorkflow(workflow, undefined, { source: 'manual' })
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
    const WfIcon = ICON_MAP[wf.icon] || Zap
    return {
      id: wf.id,
      iconElement: <WfIcon size={12} color={wf.iconColor} />,
      label: wf.name,
      onClick: () => {
        onSelect()
        executeWorkflow(
          wf,
          hasContext ? { task: context?.task, source: context?.source } : undefined,
          { source: 'manual' }
        )
      }
    }
  })
}
