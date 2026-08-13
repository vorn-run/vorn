import { useState, useMemo, useCallback } from 'react'
import { useAppStore } from '../../stores'
import { Tooltip } from '../Tooltip'
import { WorkflowItem } from './WorkflowItem'
import { WorkflowFilterToolbar } from './WorkflowFilterToolbar'
import { WorkflowContextMenu } from './WorkflowContextMenu'
import { SidebarSectionHeader } from './SidebarSectionHeader'
import { isScheduledWorkflow } from '../../lib/workflow-helpers'
import { Workflow, Activity } from 'lucide-react'
import type { WorkflowDefinition } from '../../../shared/types'
import { useWaitingApprovals } from '../../hooks/useWaitingApprovals'

type ContextMenuState = { id: string; x: number; y: number } | null

export function WorkflowsSection({
  isCollapsed,
  workspaceWorkflows
}: {
  isCollapsed: boolean
  workspaceWorkflows: WorkflowDefinition[]
}) {
  const setWorkflowEditorOpen = useAppStore((s) => s.setWorkflowEditorOpen)
  const setEditingWorkflowId = useAppStore((s) => s.setEditingWorkflowId)
  const removeWorkflow = useAppStore((s) => s.removeWorkflow)
  const updateWorkflow = useAppStore((s) => s.updateWorkflow)
  const reorderWorkflows = useAppStore((s) => s.reorderWorkflows)
  const workflowFilter = useAppStore((s) => s.sidebarWorkflowFilter)
  const editingWorkflowId = useAppStore((s) => s.editingWorkflowId)
  const isWorkflowEditorOpen = useAppStore((s) => s.isWorkflowEditorOpen)
  const allRunsSelected = editingWorkflowId === null && !isWorkflowEditorOpen
  const waitingCount = useWaitingApprovals().length

  const [sectionCollapsed, setSectionCollapsed] = useState(false)
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null)
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null)
  const [dragSourceIndex, setDragSourceIndex] = useState<number | null>(null)

  const iconSize = isCollapsed ? 22 : 14

  const filteredWorkflows = useMemo(() => {
    if (workflowFilter === 'all') return workspaceWorkflows
    if (workflowFilter === 'manual')
      return workspaceWorkflows.filter((w) => !isScheduledWorkflow(w))
    return workspaceWorkflows.filter((w) => isScheduledWorkflow(w))
  }, [workspaceWorkflows, workflowFilter])

  const handleContextMenu = (e: React.MouseEvent, workflowId: string) => {
    setContextMenu({ id: workflowId, x: e.clientX, y: e.clientY })
  }

  const handleDragStart = useCallback((e: React.DragEvent, index: number) => {
    setDragSourceIndex(index)
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', String(index))
  }, [])

  const handleDragOver = useCallback((e: React.DragEvent, index: number) => {
    e.preventDefault()
    e.dataTransfer.dropEffect = 'move'
    setDragOverIndex(index)
  }, [])

  const handleDrop = useCallback(
    (e: React.DragEvent, toIndex: number) => {
      e.preventDefault()
      const fromIndex = parseInt(e.dataTransfer.getData('text/plain'), 10)
      if (!isNaN(fromIndex) && fromIndex !== toIndex) {
        reorderWorkflows(fromIndex, toIndex)
      }
      setDragSourceIndex(null)
      setDragOverIndex(null)
    },
    [reorderWorkflows]
  )

  const handleDragEnd = useCallback(() => {
    setDragSourceIndex(null)
    setDragOverIndex(null)
  }, [])

  const menuWorkflow = contextMenu ? workspaceWorkflows.find((w) => w.id === contextMenu.id) : null
  const menuIsScheduled = menuWorkflow ? isScheduledWorkflow(menuWorkflow) : false

  return (
    <>
      <SidebarSectionHeader
        title="Workflows"
        isCollapsed={isCollapsed}
        sectionCollapsed={sectionCollapsed}
        onToggle={() => setSectionCollapsed(!sectionCollapsed)}
        actions={
          <>
            <WorkflowFilterToolbar />
            <Tooltip label="New workflow" position="bottom">
              <button
                onClick={() => {
                  setEditingWorkflowId(null)
                  setWorkflowEditorOpen(true)
                }}
                aria-label="New workflow"
                className="p-0.5 rounded text-gray-600 hover:text-white hover:bg-white/[0.08] transition-colors"
              >
                <Workflow size={13} strokeWidth={1.5} />
              </button>
            </Tooltip>
          </>
        }
      />

      {!sectionCollapsed && (
        <button
          type="button"
          onClick={() => {
            setEditingWorkflowId(null)
            setWorkflowEditorOpen(false)
          }}
          title={isCollapsed ? 'All runs' : undefined}
          aria-label="All runs"
          aria-pressed={allRunsSelected}
          className={`group/all relative w-full text-left px-2 py-1.5 rounded-md text-[13px] flex items-center gap-2 min-w-0 transition-colors ${
            allRunsSelected ? 'text-white' : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
          } ${isCollapsed ? 'justify-center px-0' : ''}`}
        >
          {allRunsSelected && !isCollapsed && (
            <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />
          )}
          <Activity size={iconSize} strokeWidth={1.5} className="text-gray-400 shrink-0" />
          {!isCollapsed && (
            <>
              <span className="min-w-0 flex-1 truncate">All runs</span>
              {waitingCount > 0 && (
                <span className="font-mono text-[10px] text-bronzo tabular-nums">
                  {waitingCount}
                </span>
              )}
            </>
          )}
        </button>
      )}

      {!isCollapsed && !sectionCollapsed && filteredWorkflows.length === 0 && (
        <p className="text-[13px] text-gray-600 px-2.5 py-1">No workflows</p>
      )}

      {!isCollapsed &&
        !sectionCollapsed &&
        filteredWorkflows.map((wf) => {
          const fullIndex = workspaceWorkflows.findIndex((w) => w.id === wf.id)
          const canReorder = workflowFilter === 'all'
          return (
            <div
              key={wf.id}
              draggable={canReorder && !isCollapsed}
              onDragStart={canReorder ? (e) => handleDragStart(e, fullIndex) : undefined}
              onDragOver={canReorder ? (e) => handleDragOver(e, fullIndex) : undefined}
              onDrop={canReorder ? (e) => handleDrop(e, fullIndex) : undefined}
              onDragEnd={canReorder ? handleDragEnd : undefined}
              className={`${canReorder ? 'cursor-grab active:cursor-grabbing' : ''} ${
                dragOverIndex === fullIndex && dragSourceIndex !== fullIndex
                  ? 'border-t border-white/40'
                  : ''
              }`}
            >
              <WorkflowItem
                workflow={wf}
                isCollapsed={isCollapsed}
                iconSize={iconSize}
                onContextMenu={handleContextMenu}
              />
            </div>
          )
        })}

      {isCollapsed &&
        filteredWorkflows.map((wf) => (
          <WorkflowItem
            key={wf.id}
            workflow={wf}
            isCollapsed={isCollapsed}
            iconSize={iconSize}
            onContextMenu={handleContextMenu}
          />
        ))}

      {contextMenu && menuWorkflow && (
        <WorkflowContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          isScheduled={menuIsScheduled}
          isEnabled={menuWorkflow.enabled}
          onEdit={() => {
            setEditingWorkflowId(menuWorkflow.id)
            setWorkflowEditorOpen(true)
          }}
          onDelete={() => removeWorkflow(menuWorkflow.id)}
          onToggleEnabled={
            menuIsScheduled
              ? () =>
                  updateWorkflow(menuWorkflow.id, {
                    ...menuWorkflow,
                    enabled: !menuWorkflow.enabled
                  })
              : undefined
          }
          onClose={() => setContextMenu(null)}
        />
      )}
    </>
  )
}
