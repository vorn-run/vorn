import { useState, useCallback, useEffect, useMemo, useRef } from 'react'
import { motion } from 'framer-motion'
import { ArrowLeft, Save, Play, Trash2, History, Zap, MoreHorizontal, Settings } from 'lucide-react'
import { ICON_MAP } from '../project-sidebar/icon-map'
import { PROJECT_ICON_OPTIONS, ICON_COLOR_PALETTE } from '../../lib/project-icons'
import { Tooltip } from '../Tooltip'
import { useAppStore } from '../../stores'
import { isMac, isWeb, TRAFFIC_LIGHT_PAD_PX } from '../../lib/platform'
import { SidebarToggleButton } from '../SidebarToggleButton'
import { MainViewPills } from '../MainViewPills'
import { WindowControls } from '../WindowControls'
import {
  WorkflowDefinition,
  WorkflowNode,
  WorkflowEdge,
  TriggerConfig,
  AiAgentType,
  CallConnectorActionConfig,
  ConnectorActionDef,
  supportsExactSessionResume,
  getProjectRemoteHostId
} from '../../../shared/types'
import { WorkflowCanvas, AddableNodeType } from './WorkflowCanvas'
import { NodeConfigPanel } from './panels/NodeConfigPanel'
import { RunHistoryPanel } from './panels/RunHistoryPanel'
import { WorkflowPropertiesPanel } from './panels/WorkflowPropertiesPanel'
import {
  createTriggerNode,
  createLaunchAgentNode,
  createScriptNode,
  createConditionNode,
  createApprovalNode,
  createCallConnectorActionNode,
  appendNodeAfter,
  insertNodeBetween,
  insertBeforeFork,
  insertConditionBetween,
  addParallelBranch,
  removeNode,
  getWorktreeMode
} from '../../lib/workflow-helpers'
import { executeWorkflow } from '../../lib/workflow-execution'
import { toast } from '../Toast'
import {
  slugify,
  ensureUniqueSlug,
  getAncestorNodes,
  buildStepGroups
} from '../../lib/template-vars'

const EMPTY_TASKS: import('../../../shared/types').TaskConfig[] = []

export function WorkflowEditor({ inline = false }: { inline?: boolean } = {}) {
  const isOpen = useAppStore((s) => s.isWorkflowEditorOpen)
  const isActive = inline || isOpen
  const editingId = useAppStore((s) => s.editingWorkflowId)
  const isSidebarOpen = useAppStore((s) => s.isSidebarOpen)
  const setOpen = useAppStore((s) => s.setWorkflowEditorOpen)
  const setEditingId = useAppStore((s) => s.setEditingWorkflowId)
  const addWorkflow = useAppStore((s) => s.addWorkflow)
  const updateWorkflow = useAppStore((s) => s.updateWorkflow)
  const removeWorkflowFromStore = useAppStore((s) => s.removeWorkflow)
  const existingWorkflow = useAppStore((s) =>
    editingId ? (s.config?.workflows || []).find((w) => w.id === editingId) : null
  )
  const tasks = useAppStore((s) => s.config?.tasks ?? EMPTY_TASKS)
  const addTerminal = useAppStore((s) => s.addTerminal)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const setSelectedTaskId = useAppStore((s) => s.setSelectedTaskId)

  const [name, setName] = useState('New Workflow')
  const [icon, setIcon] = useState('Zap')
  const [iconColor, setIconColor] = useState('#3b82f6')
  const [nodes, setNodes] = useState<WorkflowNode[]>([])
  const [edges, setEdges] = useState<WorkflowEdge[]>([])
  const [enabled, setEnabled] = useState(true)
  const [staggerDelayMs, setStaggerDelayMs] = useState<number | undefined>(undefined)
  const [autoCleanupWorktrees, setAutoCleanupWorktrees] = useState(false)
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [showRunHistory, setShowRunHistory] = useState(false)
  const [showProperties, setShowProperties] = useState(true)
  const [showIconPicker, setShowIconPicker] = useState(false)
  const [showOverflowMenu, setShowOverflowMenu] = useState(false)
  const iconPickerRef = useRef<HTMLDivElement>(null)
  const overflowMenuRef = useRef<HTMLDivElement>(null)
  const [executionHistory, setExecutionHistory] = useState<
    import('../../../shared/types').WorkflowExecution[]
  >([])
  const loadedRunsForId = useRef<string | null>(null)

  const selectedNode = useMemo(
    () => nodes.find((n) => n.id === selectedNodeId) || null,
    [nodes, selectedNodeId]
  )

  const triggerType = useMemo(() => {
    const triggerNode = nodes.find((n) => n.type === 'trigger')
    if (!triggerNode) return undefined
    return (triggerNode.config as TriggerConfig).triggerType
  }, [nodes])

  const isContextualTrigger = useMemo(() => {
    const triggerNode = nodes.find((n) => n.type === 'trigger')
    if (!triggerNode) return false
    const cfg = triggerNode.config as TriggerConfig
    return cfg.triggerType === 'manual' && cfg.contextual === true
  }, [nodes])

  // Disabled state for the cleanup toggle: when every LaunchAgent inherits
  // its worktree from context, there's nothing for autoCleanupWorktrees to
  // act on, so toggling has no effect.
  const allWorktreesInherited = useMemo(() => {
    let anyCreates = false
    let anyInherits = false
    for (const node of nodes) {
      if (node.type !== 'launchAgent') continue
      const mode = getWorktreeMode(node.config as import('../../../shared/types').LaunchAgentConfig)
      if (mode === 'fromContext') anyInherits = true
      else if (mode === 'new') anyCreates = true
    }
    return anyInherits && !anyCreates
  }, [nodes])

  // Map of connectionId → action defs. Populated for every connection the
  // workflow's callConnectorAction nodes reference, so the autocomplete can
  // surface schema-typed outputs (e.g. `{{steps.createIssue.html_url}}`)
  // without making buildStepGroups async.
  const [connectionActions, setConnectionActions] = useState<Map<string, ConnectorActionDef[]>>(
    () => new Map()
  )

  // Join the referenced connection ids into a stable key so the effect below
  // only refires when the set of referenced connections changes — not on
  // every label edit, drag, or unrelated field update.
  const connectionIdsKey = useMemo(() => {
    const ids = new Set<string>()
    for (const n of nodes) {
      if (n.type === 'callConnectorAction') {
        const cfg = n.config as CallConnectorActionConfig
        if (cfg.connectionId) ids.add(cfg.connectionId)
      }
    }
    return [...ids].sort().join(',')
  }, [nodes])

  useEffect(() => {
    if (!connectionIdsKey) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setConnectionActions((prev) => (prev.size === 0 ? prev : new Map()))
      return
    }
    let cancelled = false
    // Per-connection so a single dead connection doesn't blank out the
    // autocomplete for healthy ones — each id resolves independently.
    Promise.all(
      connectionIdsKey.split(',').map(async (id): Promise<[string, ConnectorActionDef[]]> => {
        try {
          const actions = await window.api.listConnectionActions(id)
          return [id, actions]
        } catch {
          return [id, []]
        }
      })
    ).then((entries) => {
      if (!cancelled) setConnectionActions(new Map(entries))
    })
    return () => {
      cancelled = true
    }
  }, [connectionIdsKey])

  const lookupAction = useCallback(
    (connectionId: string, actionType: string) =>
      connectionActions.get(connectionId)?.find((a) => a.type === actionType),
    [connectionActions]
  )

  const stepGroups = useMemo(() => {
    if (!selectedNodeId) return []
    const ancestors = getAncestorNodes(nodes, edges, selectedNodeId)
    return buildStepGroups(ancestors, lookupAction)
  }, [nodes, edges, selectedNodeId, lookupAction])

  // Load execution history from database
  useEffect(() => {
    if (editingId && isActive && loadedRunsForId.current !== editingId) {
      loadedRunsForId.current = editingId
      window.api.listWorkflowRuns(editingId, 20).then(setExecutionHistory)
    }
    if (!isActive) {
      loadedRunsForId.current = null
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setExecutionHistory([])
    }
  }, [editingId, isActive])

  // Re-query on status transitions for this workflow only, plus whenever a
  // node flips into or out of 'waiting' (gate approve/reject). Selecting
  // scalars avoids refetching on every streaming log chunk.
  const liveExecSignature = useAppStore((s) => {
    if (!editingId) return ''
    const exec = s.workflowExecutions.get(editingId)
    if (!exec) return ''
    const waiting = (exec.nodeStates ?? [])
      .filter((n) => n.status === 'waiting')
      .map((n) => n.nodeId)
      .join(',')
    return `${exec.status ?? ''}|${exec.completedAt ?? ''}|${waiting}`
  })
  useEffect(() => {
    if (!editingId || !isActive || !liveExecSignature) return
    window.api.listWorkflowRuns(editingId, 20).then(setExecutionHistory).catch(console.error)
  }, [editingId, isActive, liveExecSignature])

  // Load existing workflow when editing (with slug migration)
  useEffect(() => {
    if (existingWorkflow) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setName(existingWorkflow.name)
      setIcon(existingWorkflow.icon)
      setIconColor(existingWorkflow.iconColor)
      const usedSlugs = new Set<string>()
      const migratedNodes = existingWorkflow.nodes.map((n) => {
        if (n.slug) {
          if (usedSlugs.has(n.slug)) {
            const uniqueSlug = ensureUniqueSlug(n.slug, usedSlugs)
            usedSlugs.add(uniqueSlug)
            return { ...n, slug: uniqueSlug }
          }
          usedSlugs.add(n.slug)
          return n
        }
        if (n.type === 'trigger') return n
        const slug = ensureUniqueSlug(slugify(n.label), usedSlugs)
        usedSlugs.add(slug)
        return { ...n, slug }
      })
      setNodes(migratedNodes)
      setEdges(existingWorkflow.edges)
      setEnabled(existingWorkflow.enabled)
      setStaggerDelayMs(existingWorkflow.staggerDelayMs)
      setAutoCleanupWorktrees(existingWorkflow.autoCleanupWorktrees ?? false)
    } else if (!editingId) {
      // New workflow — start with a manual trigger
      const trigger = createTriggerNode({ triggerType: 'manual' })
      setName('New Workflow')
      setIcon('Zap')
      setIconColor('#3b82f6')
      setNodes([trigger])
      setEdges([])
      setEnabled(true)
      setStaggerDelayMs(undefined)
    }
    setSelectedNodeId(null)
    setShowRunHistory(false)
  }, [existingWorkflow, editingId, isActive])

  useEffect(() => {
    if (!showIconPicker) return
    const handler = (e: MouseEvent) => {
      if (iconPickerRef.current && !iconPickerRef.current.contains(e.target as Node)) {
        setShowIconPicker(false)
      }
    }
    document.addEventListener('pointerdown', handler)
    return () => document.removeEventListener('pointerdown', handler)
  }, [showIconPicker])

  useEffect(() => {
    if (!showOverflowMenu) return
    const handler = (e: MouseEvent) => {
      if (overflowMenuRef.current && !overflowMenuRef.current.contains(e.target as Node)) {
        setShowOverflowMenu(false)
      }
    }
    document.addEventListener('pointerdown', handler)
    return () => document.removeEventListener('pointerdown', handler)
  }, [showOverflowMenu])

  const handleClose = useCallback(() => {
    setEditingId(null)
    setSelectedNodeId(null)
    setShowRunHistory(false)
    setOpen(false)
  }, [setOpen, setEditingId])

  const activeWorkspace = useAppStore((s) => s.activeWorkspace)

  const persistWorkflow = useCallback((): WorkflowDefinition => {
    const workflow: WorkflowDefinition = {
      id: editingId || crypto.randomUUID(),
      name,
      icon,
      iconColor,
      nodes,
      edges,
      enabled,
      ...(staggerDelayMs && { staggerDelayMs }),
      ...(autoCleanupWorktrees && { autoCleanupWorktrees }),
      ...(existingWorkflow?.lastRunAt && { lastRunAt: existingWorkflow.lastRunAt }),
      ...(existingWorkflow?.lastRunStatus && { lastRunStatus: existingWorkflow.lastRunStatus }),
      workspaceId: existingWorkflow?.workspaceId ?? activeWorkspace
    }
    if (editingId) {
      updateWorkflow(editingId, workflow)
    } else {
      addWorkflow(workflow)
      if (inline) setEditingId(workflow.id)
    }
    return workflow
  }, [
    editingId,
    name,
    icon,
    iconColor,
    nodes,
    edges,
    enabled,
    staggerDelayMs,
    autoCleanupWorktrees,
    existingWorkflow,
    updateWorkflow,
    addWorkflow,
    activeWorkspace,
    inline,
    setEditingId
  ])

  const handleSave = useCallback(() => {
    persistWorkflow()
    toast.success(editingId ? 'Workflow saved' : 'Workflow created')
    if (!inline) handleClose()
  }, [persistWorkflow, editingId, inline, handleClose])

  const handleRun = useCallback(async () => {
    const workflow = persistWorkflow()
    if (!inline) handleClose()
    await executeWorkflow(workflow)
  }, [persistWorkflow, inline, handleClose])

  const handleDelete = useCallback(() => {
    if (editingId) {
      removeWorkflowFromStore(editingId)
    }
    handleClose()
  }, [editingId, removeWorkflowFromStore, handleClose])

  const createNodeWithUniqueSlug = useCallback(
    (type: AddableNodeType) => {
      const projects = useAppStore.getState().config?.projects || []
      const firstProject = projects[0]
      const factories: Record<AddableNodeType, () => WorkflowNode> = {
        condition: () => createConditionNode(),
        approval: () => createApprovalNode(),
        script: () => createScriptNode(),
        connectorAction: () => createCallConnectorActionNode(),
        agent: () =>
          createLaunchAgentNode(
            firstProject ? { projectName: firstProject.name, projectPath: firstProject.path } : {}
          )
      }
      const newNode = factories[type]()
      if (newNode.slug) {
        const existingSlugs = new Set(nodes.filter((n) => n.slug).map((n) => n.slug!))
        newNode.slug = ensureUniqueSlug(newNode.slug, existingSlugs)
      }
      return newNode
    },
    [nodes]
  )

  const handleInsertNode = useCallback(
    (afterNodeId: string, beforeNodeId: string | null, type: AddableNodeType) => {
      // Condition nodes use a special insertion that creates true/false branches
      if (type === 'condition') {
        const result = insertConditionBetween(nodes, edges, afterNodeId, beforeNodeId)
        setNodes(result.nodes)
        setEdges(result.edges)
        // Select the condition node (last added)
        const condNode = result.nodes.find(
          (n) => n.type === 'condition' && !nodes.find((o) => o.id === n.id)
        )
        if (condNode) setSelectedNodeId(condNode.id)
        return
      }

      const newNode = createNodeWithUniqueSlug(type)

      let result: { nodes: WorkflowNode[]; edges: WorkflowEdge[] }
      if (beforeNodeId === '__FORK__') {
        result = insertBeforeFork(nodes, edges, afterNodeId, newNode)
      } else if (beforeNodeId) {
        const edge = edges.find((e) => e.source === afterNodeId && e.target === beforeNodeId)
        if (edge) {
          result = insertNodeBetween(nodes, edges, edge.id, newNode)
        } else {
          result = appendNodeAfter(nodes, edges, afterNodeId, newNode)
        }
      } else {
        result = appendNodeAfter(nodes, edges, afterNodeId, newNode)
      }

      setNodes(result.nodes)
      setEdges(result.edges)
      setSelectedNodeId(newNode.id)
    },
    [nodes, edges, createNodeWithUniqueSlug]
  )

  const handleAddParallelBranch = useCallback(
    (forkFromId: string, type: 'agent' | 'script') => {
      const newNode = createNodeWithUniqueSlug(type)
      const result = addParallelBranch(nodes, edges, forkFromId, newNode)
      setNodes(result.nodes)
      setEdges(result.edges)
      setSelectedNodeId(newNode.id)
    },
    [nodes, edges, createNodeWithUniqueSlug]
  )

  const handleNodeClick = useCallback((nodeId: string) => {
    setSelectedNodeId(nodeId || null)
    setShowRunHistory(false)
    if (!nodeId) setShowProperties(true)
  }, [])

  const handleNodeConfigChange = useCallback((nodeId: string, config: WorkflowNode['config']) => {
    setNodes((nds) => nds.map((n) => (n.id === nodeId ? { ...n, config } : n)))
  }, [])

  const handleNodeLabelChange = useCallback((nodeId: string, label: string) => {
    setNodes((nds) => {
      const node = nds.find((n) => n.id === nodeId)
      const oldSlug = node?.slug
      const existingSlugs = new Set(
        nds.filter((n) => n.id !== nodeId && n.slug).map((n) => n.slug!)
      )
      const newSlug = ensureUniqueSlug(slugify(label), existingSlugs)

      return nds.map((n) => {
        if (n.id === nodeId) return { ...n, label, slug: newSlug }
        // Update template references in other nodes when slug changes
        if (oldSlug && oldSlug !== newSlug && n.config) {
          const configStr = JSON.stringify(n.config)
          if (configStr.includes(`steps.${oldSlug}.`)) {
            return {
              ...n,
              config: JSON.parse(configStr.split(`steps.${oldSlug}.`).join(`steps.${newSlug}.`))
            }
          }
        }
        return n
      })
    })
  }, [])

  const handleDeleteNode = useCallback(
    (nodeId: string) => {
      const result = removeNode(nodes, edges, nodeId)
      setNodes(result.nodes)
      setEdges(result.edges)
      setSelectedNodeId(null)
    },
    [nodes, edges]
  )

  const handleResumeSession = useCallback(
    async (
      agentSessionId: string,
      agentType: AiAgentType,
      projectName: string,
      projectPath: string,
      branch?: string,
      useWorktree?: boolean
    ) => {
      if (!supportsExactSessionResume(agentType)) return

      const cfg = useAppStore.getState().config
      const proj = cfg?.projects.find((p) => p.name === projectName)
      const remoteHostId = proj ? getProjectRemoteHostId(proj) : undefined
      const effectiveProjectPath = projectPath || proj?.path
      if (!effectiveProjectPath) {
        toast.error(`Can't resume: project "${projectName}" not found`)
        return
      }
      const session = await window.api.createTerminal({
        agentType,
        projectName,
        projectPath: effectiveProjectPath,
        branch,
        useWorktree,
        resumeSessionId: agentSessionId,
        remoteHostId
      })
      addTerminal(session)
      setFocusedTerminal(session.id)
      handleClose()
    },
    [addTerminal, setFocusedTerminal, handleClose]
  )

  const handleClickTask = useCallback(
    (taskId: string) => {
      setSelectedTaskId(taskId)
      handleClose()
    },
    [setSelectedTaskId, handleClose]
  )

  if (!isActive) return null

  const editorContent = (
    <>
      {/* Top bar */}
      <div
        className={`shrink-0 h-[40px] flex items-center justify-between px-3 border-b border-white/[0.08] titlebar-drag`}
        style={
          inline && !isSidebarOpen && isMac && !isWeb
            ? { paddingLeft: `${TRAFFIC_LIGHT_PAD_PX}px` }
            : undefined
        }
      >
        <div
          className="flex items-center gap-1 titlebar-no-drag"
          style={!inline && isMac && !isWeb ? { paddingLeft: '70px' } : undefined}
        >
          {inline && !isSidebarOpen && (
            <>
              <SidebarToggleButton />
              <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
              <MainViewPills />
              <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
            </>
          )}
          {!inline && (
            <button
              onClick={handleClose}
              aria-label="Back"
              className="text-gray-400 hover:text-white p-1.5 rounded-md transition-colors"
            >
              <ArrowLeft size={16} />
            </button>
          )}
          <div className="relative">
            <button
              onClick={() => setShowIconPicker(!showIconPicker)}
              className="p-1.5 rounded-md hover:bg-white/[0.08] transition-colors"
              title="Change icon"
            >
              {(() => {
                const WfIcon = ICON_MAP[icon] || Zap
                return <WfIcon size={16} color={iconColor} strokeWidth={1.5} />
              })()}
            </button>
            {showIconPicker && (
              <div
                ref={iconPickerRef}
                className="absolute top-full left-0 mt-1 p-2 rounded-lg border border-white/[0.08] shadow-xl z-50 w-[220px] space-y-2"
                style={{ background: '#1e1e22' }}
              >
                <div className="grid grid-cols-8 gap-1">
                  {PROJECT_ICON_OPTIONS.map((opt) => {
                    const IconComp = ICON_MAP[opt.name] || Zap
                    return (
                      <button
                        key={opt.name}
                        onClick={() => setIcon(opt.name)}
                        className={`p-1.5 rounded ${
                          icon === opt.name
                            ? 'bg-white/[0.1] ring-1 ring-white/[0.2]'
                            : 'hover:bg-white/[0.06]'
                        }`}
                        title={opt.label}
                      >
                        <IconComp
                          size={12}
                          color={icon === opt.name ? iconColor : '#9ca3af'}
                          strokeWidth={1.5}
                        />
                      </button>
                    )
                  })}
                </div>
                <div className="flex gap-1.5">
                  {ICON_COLOR_PALETTE.map((color) => (
                    <button
                      key={color}
                      onClick={() => setIconColor(color)}
                      className={`w-5 h-5 rounded-full border ${
                        iconColor === color
                          ? 'border-white scale-110'
                          : 'border-transparent hover:border-white/30'
                      }`}
                      style={{ backgroundColor: color }}
                    />
                  ))}
                </div>
              </div>
            )}
          </div>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="text-[15px] font-medium text-white bg-transparent border-none outline-none
                       hover:bg-white/[0.04] focus:bg-white/[0.06] px-2 py-1 rounded-md transition-colors
                       w-[240px]"
            placeholder="Workflow name"
          />
        </div>

        <div className="flex items-center gap-1 titlebar-no-drag">
          <Tooltip label="Run workflow" position="bottom">
            <button
              onClick={handleRun}
              aria-label="Run workflow"
              className="text-gray-400 hover:text-white p-1.5 rounded-md hover:bg-white/[0.06] transition-colors"
            >
              <Play size={15} />
            </button>
          </Tooltip>

          {editingId && (
            <Tooltip
              label={`Run history${executionHistory.length > 0 ? ` (${executionHistory.length})` : ''}`}
              position="bottom"
            >
              <button
                onClick={() => {
                  setShowRunHistory(!showRunHistory)
                  if (!showRunHistory) setSelectedNodeId(null)
                }}
                aria-label={`Run history (${executionHistory.length})`}
                aria-pressed={showRunHistory}
                className={`p-1.5 rounded-md transition-colors ${
                  showRunHistory
                    ? 'text-white bg-white/[0.08]'
                    : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
                }`}
              >
                <History size={15} />
              </button>
            </Tooltip>
          )}

          <button
            onClick={handleSave}
            className="px-3 py-1.5 text-[12px] font-medium text-white
                       bg-white/[0.12] hover:bg-white/[0.18] rounded-md transition-colors ml-1
                       flex items-center gap-1.5"
          >
            <Save size={13} strokeWidth={1.5} />
            Save
          </button>

          <div className="relative ml-0.5">
            <Tooltip label="More options" position="bottom">
              <button
                onClick={() => setShowOverflowMenu(!showOverflowMenu)}
                aria-label="More options"
                aria-haspopup="menu"
                aria-expanded={showOverflowMenu}
                className={`p-1.5 rounded-md transition-colors ${
                  showOverflowMenu
                    ? 'text-white bg-white/[0.08]'
                    : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
                }`}
              >
                <MoreHorizontal size={15} />
              </button>
            </Tooltip>
            {showOverflowMenu && (
              <div
                ref={overflowMenuRef}
                className="absolute right-0 top-full mt-1 z-50 min-w-[180px] py-1 border border-white/[0.08] rounded-lg shadow-xl"
                style={{ background: '#141416' }}
              >
                <button
                  onClick={() => {
                    setSelectedNodeId(null)
                    setShowRunHistory(false)
                    setShowProperties(true)
                    setShowOverflowMenu(false)
                  }}
                  className="w-full px-3 py-2 text-left text-[12px] text-gray-300 hover:text-white
                             hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
                >
                  <Settings size={12} strokeWidth={1.5} />
                  Workflow settings
                </button>
                {editingId && (
                  <>
                    <div className="my-1 border-t border-white/[0.06]" />
                    <button
                      onClick={() => {
                        setShowOverflowMenu(false)
                        handleDelete()
                      }}
                      className="w-full px-3 py-2 text-left text-[12px] text-red-400 hover:text-red-300
                                 hover:bg-white/[0.06] flex items-center gap-2 transition-colors"
                    >
                      <Trash2 size={12} strokeWidth={1.5} />
                      Delete workflow
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
          {inline && <WindowControls />}
        </div>
      </div>

      <div className={`flex-1 flex overflow-hidden ${inline ? '' : 'titlebar-no-drag'}`}>
        <WorkflowCanvas
          nodes={nodes}
          edges={edges}
          onNodeClick={handleNodeClick}
          onInsertNode={handleInsertNode}
          onAddParallelBranch={handleAddParallelBranch}
          selectedNodeId={selectedNodeId}
        />

        {showRunHistory && (
          <RunHistoryPanel
            executions={executionHistory}
            nodes={nodes}
            tasks={tasks}
            onClose={() => setShowRunHistory(false)}
            onClickTask={handleClickTask}
            onResumeSession={handleResumeSession}
          />
        )}

        {selectedNode && !showRunHistory && (
          <NodeConfigPanel
            node={selectedNode}
            allNodes={nodes}
            onChange={handleNodeConfigChange}
            onLabelChange={handleNodeLabelChange}
            onDelete={handleDeleteNode}
            onClose={() => setSelectedNodeId(null)}
            triggerType={triggerType}
            isContextualTrigger={isContextualTrigger}
            stepGroups={stepGroups}
          />
        )}

        {!selectedNode && !showRunHistory && showProperties && (
          <WorkflowPropertiesPanel
            enabled={enabled}
            onEnabledChange={setEnabled}
            staggerDelayMs={staggerDelayMs}
            onStaggerChange={setStaggerDelayMs}
            autoCleanupWorktrees={autoCleanupWorktrees}
            onCleanupChange={setAutoCleanupWorktrees}
            cleanupDisabled={allWorktreesInherited}
            triggerNode={nodes.find((n) => n.type === 'trigger') ?? null}
            onSelectTrigger={() => {
              const t = nodes.find((n) => n.type === 'trigger')
              if (t) setSelectedNodeId(t.id)
            }}
            lastRun={executionHistory[0] ?? null}
            onClose={() => setShowProperties(false)}
          />
        )}
      </div>
    </>
  )

  if (inline) {
    return (
      <div className="flex-1 flex flex-col min-h-0" style={{ background: '#1a1a1e' }}>
        {editorContent}
      </div>
    )
  }

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 flex flex-col titlebar-no-drag"
      style={{
        background: '#1a1a1e',
        paddingTop: 'var(--safe-top, 0px)',
        paddingRight: 'var(--safe-right, 0px)',
        paddingBottom: 'var(--safe-bottom, 0px)',
        paddingLeft: 'var(--safe-left, 0px)'
      }}
    >
      {editorContent}
    </motion.div>
  )
}
