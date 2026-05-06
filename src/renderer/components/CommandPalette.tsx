import { useState, useMemo, useEffect, useRef } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { getProjectHostIds, getProjectRemoteHostId, type RecentSession } from '../../shared/types'
import { SortMode, StatusFilter } from '../stores/types'
import { AGENT_DEFINITIONS, AGENT_LIST } from '../lib/agent-definitions'
import { AgentIcon } from './AgentIcon'
import { getDisplayName } from '../lib/terminal-display'
import {
  resolveActiveProject,
  formatRecentSessionActivity,
  resolveProjectName,
  createSessionFromProject,
  createShellInProject
} from '../lib/session-utils'
import { runWorkflowFromGlobalSurface } from '../lib/workflow-menu-items'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import {
  Search,
  Plus,
  Settings,
  PanelLeft,
  FolderPlus,
  Zap,
  Monitor,
  Filter,
  ArrowUpDown,
  Server,
  Keyboard,
  ListTodo,
  BookOpen,
  Terminal,
  Plug,
  LayoutDashboard
} from 'lucide-react'

type CommandCategory =
  | 'actions'
  | 'terminals'
  | 'recent'
  | 'projects'
  | 'workflows'
  | 'quicklaunch'
  | 'filter'

interface Command {
  id: string
  label: string
  sublabel?: string
  category: CommandCategory
  icon?: React.ReactNode
  shortcutDisplay?: string
  keywords?: string[]
  onExecute: () => void | Promise<void>
}

const CATEGORY_ORDER: CommandCategory[] = [
  'actions',
  'terminals',
  'recent',
  'projects',
  'workflows',
  'quicklaunch',
  'filter'
]

const CATEGORY_LABELS: Record<CommandCategory, string> = {
  actions: 'Actions',
  terminals: 'Terminals',
  recent: 'Recent Sessions',
  projects: 'Projects',
  workflows: 'Workflows',
  quicklaunch: 'Quick Launch',
  filter: 'Filter & Sort'
}

function timeAgo(ts: number): string {
  const sec = Math.floor((Date.now() - ts) / 1000)
  if (sec < 60) return 'just now'
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`
  if (sec < 604800) return `${Math.floor(sec / 86400)}d ago`
  return new Date(ts).toLocaleDateString()
}

function highlightMatch(text: string, query: string): React.ReactNode {
  if (!query.trim()) return text
  const idx = text.toLowerCase().indexOf(query.toLowerCase())
  if (idx === -1) return text
  return (
    <>
      {text.slice(0, idx)}
      <span className="text-white font-semibold">{text.slice(idx, idx + query.length)}</span>
      {text.slice(idx + query.length)}
    </>
  )
}

function scoreMatch(query: string, command: Command): number {
  const q = query.toLowerCase()
  const label = command.label.toLowerCase()

  if (label === q) return 100
  if (label.startsWith(q)) return 80
  if (label.split(/\s+/).some((w) => w.startsWith(q))) return 60
  if (label.includes(q)) return 40
  if ((command.keywords ?? []).join(' ').toLowerCase().includes(q)) return 20
  return 0
}

function useCommands(
  recentSessions: RecentSession[],
  installStatus: Record<string, boolean>,
  gitRepoStatus: Record<string, boolean>
): Command[] {
  const config = useAppStore((s) => s.config)
  const terminals = useAppStore((s) => s.terminals)
  const worktreeCache = useAppStore((s) => s.worktreeCache)
  const addTerminal = useAppStore((s) => s.addTerminal)
  const setFocusedTerminal = useAppStore((s) => s.setFocusedTerminal)
  const setActiveProject = useAppStore((s) => s.setActiveProject)
  const setNewAgentDialogOpen = useAppStore((s) => s.setNewAgentDialogOpen)
  const setAddProjectDialogOpen = useAppStore((s) => s.setAddProjectDialogOpen)
  const setWorkflowEditorOpen = useAppStore((s) => s.setWorkflowEditorOpen)
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen)
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  const setSortMode = useAppStore((s) => s.setSortMode)
  const setStatusFilter = useAppStore((s) => s.setStatusFilter)
  const setMainViewMode = useAppStore((s) => s.setMainViewMode)
  const setTaskDialogOpen = useAppStore((s) => s.setTaskDialogOpen)
  const setOnboardingOpen = useAppStore((s) => s.setOnboardingOpen)

  return useMemo(() => {
    const commands: Command[] = []

    // --- Actions ---
    commands.push({
      id: 'action:new-session',
      label: 'New Session',
      category: 'actions',
      icon: <Plus size={14} strokeWidth={1.5} />,
      shortcutDisplay: '\u2318N',
      keywords: ['create', 'add', 'agent', 'terminal', 'launch'],
      onExecute: () => setNewAgentDialogOpen(true)
    })
    commands.push({
      id: 'action:settings',
      label: 'Open Settings',
      category: 'actions',
      icon: <Settings size={14} strokeWidth={1.5} />,
      shortcutDisplay: '\u2318,',
      keywords: ['preferences', 'config', 'options'],
      onExecute: () => setSettingsOpen(true)
    })
    commands.push({
      id: 'action:toggle-sidebar',
      label: 'Toggle Sidebar',
      category: 'actions',
      icon: <PanelLeft size={14} strokeWidth={1.5} />,
      shortcutDisplay: '\u2318B',
      keywords: ['hide', 'show', 'panel'],
      onExecute: () => toggleSidebar()
    })
    commands.push({
      id: 'action:shortcuts-panel',
      label: 'Keyboard Shortcuts',
      category: 'actions',
      icon: <Keyboard size={14} strokeWidth={1.5} />,
      shortcutDisplay: '\u2318?',
      keywords: ['shortcuts', 'keys', 'help', 'reference', 'bindings', 'hotkeys'],
      onExecute: () => useAppStore.getState().setShortcutsPanelOpen(true)
    })
    commands.push({
      id: 'action:add-project',
      label: 'Add Project',
      category: 'actions',
      icon: <FolderPlus size={14} strokeWidth={1.5} />,
      keywords: ['new project', 'create project'],
      onExecute: () => setAddProjectDialogOpen(true)
    })
    commands.push({
      id: 'action:add-workflow',
      label: 'Add Workflow',
      category: 'actions',
      icon: <Zap size={14} strokeWidth={1.5} />,
      keywords: ['new workflow', 'create workflow', 'schedule'],
      onExecute: () => setWorkflowEditorOpen(true)
    })
    commands.push({
      id: 'action:open-tasks',
      label: 'Open Tasks View',
      category: 'actions',
      icon: <ListTodo size={14} strokeWidth={1.5} />,
      keywords: ['task', 'kanban', 'queue', 'todo', 'board'],
      onExecute: () => setMainViewMode('tasks')
    })
    commands.push({
      id: 'action:create-task',
      label: 'Create Task',
      category: 'actions',
      icon: <Plus size={14} strokeWidth={1.5} />,
      keywords: ['new task', 'add task', 'todo'],
      onExecute: () => setTaskDialogOpen(true)
    })
    commands.push({
      id: 'action:toggle-layout',
      label: 'Toggle Layout (Grid/Tabs)',
      category: 'actions',
      icon: <LayoutDashboard size={14} strokeWidth={1.5} />,
      keywords: ['grid', 'tabs', 'layout', 'switch', 'view'],
      onExecute: () => {
        const cfg = useAppStore.getState().config
        if (!cfg) return
        const next = (cfg.defaults.layoutMode ?? 'grid') === 'grid' ? 'tabs' : 'grid'
        const updated = {
          ...cfg,
          defaults: { ...cfg.defaults, layoutMode: next as 'grid' | 'tabs' }
        }
        useAppStore.getState().setConfig(updated)
        window.api.saveConfig(updated)
      }
    })
    commands.push({
      id: 'action:welcome-guide',
      label: 'Show Welcome Guide',
      category: 'actions',
      icon: <BookOpen size={14} strokeWidth={1.5} />,
      keywords: ['welcome', 'guide', 'help', 'onboarding', 'tour'],
      onExecute: () => setOnboardingOpen(true)
    })
    commands.push({
      id: 'action:new-terminal',
      label: 'New Terminal Session',
      category: 'actions',
      icon: <Terminal size={14} strokeWidth={1.5} />,
      shortcutDisplay: 'Ctrl+`',
      keywords: ['shell', 'terminal', 'zsh', 'bash'],
      onExecute: () => {
        const project = resolveActiveProject()
        return createShellInProject(project?.path)
      }
    })
    commands.push({
      id: 'action:copy-mcp-url',
      label: 'Copy MCP Server URL',
      category: 'actions',
      icon: <Plug size={14} strokeWidth={1.5} />,
      keywords: ['mcp', 'api', 'integration', 'claude', 'cursor', 'server', 'url'],
      onExecute: () => navigator.clipboard.writeText('http://localhost:56433/mcp')
    })
    commands.push({
      id: 'action:manage-ssh',
      label: 'SSH & Hosts',
      category: 'actions',
      icon: <Server size={14} strokeWidth={1.5} />,
      keywords: ['ssh', 'remote', 'host', 'server', 'key', 'credential', 'vault'],
      onExecute: () => {
        setSettingsOpen(true)
        useAppStore.getState().setSettingsCategory('ssh')
      }
    })

    // --- Terminals ---
    for (const [id, term] of terminals) {
      const name = getDisplayName(term.session)
      const agentType = term.session.agentType
      const agentDef = agentType === 'shell' ? null : AGENT_DEFINITIONS[agentType]
      const kindLabel = agentDef?.displayName ?? 'Shell'
      commands.push({
        id: `terminal:${id}`,
        label: name,
        category: 'terminals',
        icon: <AgentIcon agentType={agentType} size={14} />,
        keywords: [kindLabel, term.session.projectName, term.status],
        onExecute: () => setFocusedTerminal(id)
      })
    }

    // --- Recent Sessions ---
    for (const session of recentSessions) {
      if (!session.canResumeExact) continue

      const projectName = resolveProjectName(session, config?.projects)
      commands.push({
        id: `recent:${session.sessionId}`,
        label: session.display || 'Untitled session',
        sublabel: `${projectName} · ${timeAgo(session.timestamp)} · ${formatRecentSessionActivity(session)}`,
        category: 'recent',
        icon: <AgentIcon agentType={session.agentType} size={14} />,
        keywords: ['resume', 'recent', 'history', projectName, session.agentType],
        onExecute: async () => {
          try {
            const proj = config?.projects.find((p) => p.name === projectName)
            const remoteHostId = proj ? getProjectRemoteHostId(proj) : undefined
            const result = await window.api.createTerminal({
              agentType: session.agentType,
              projectName,
              projectPath: session.projectPath,
              resumeSessionId: session.sessionId,
              remoteHostId
            })
            addTerminal(result)
          } catch (err) {
            console.error('[CommandPalette] failed to resume session:', err)
          }
        }
      })
    }

    // --- Projects ---
    commands.push({
      id: 'project:all',
      label: 'All Projects',
      category: 'projects',
      icon: <Monitor size={14} strokeWidth={1.5} />,
      keywords: ['show all', 'clear filter'],
      onExecute: () => setActiveProject(null)
    })
    for (const project of config?.projects ?? []) {
      commands.push({
        id: `project:${project.name}`,
        label: project.name,
        category: 'projects',
        keywords: [project.path],
        onExecute: () => setActiveProject(project.name)
      })
    }

    // --- Workflows ---
    for (const wf of config?.workflows ?? []) {
      const actionNodes = wf.nodes.filter((n) => n.type === 'launchAgent')
      commands.push({
        id: `workflow:${wf.id}`,
        label: wf.name,
        category: 'workflows',
        icon: <Zap size={14} strokeWidth={1.5} />,
        keywords: actionNodes
          .map((n) => (n.config as unknown as Record<string, unknown>)?.projectName as string)
          .filter(Boolean),
        onExecute: () => {
          runWorkflowFromGlobalSurface(wf)
        }
      })
    }

    // --- Quick Launch (agent x project) + Worktree variants — local projects only, installed agents only ---
    const localProjects = (config?.projects ?? []).filter((p) =>
      getProjectHostIds(p).includes('local')
    )
    const installedAgents = AGENT_LIST.filter((a) => installStatus[a.type])
    for (const agent of installedAgents) {
      for (const project of localProjects) {
        commands.push({
          id: `quicklaunch:${agent.type}:${project.name}`,
          label: `${agent.displayName} on ${project.name}`,
          category: 'quicklaunch',
          icon: <AgentIcon agentType={agent.type} size={14} />,
          keywords: ['launch', 'start', 'run'],
          onExecute: () => createSessionFromProject(project, { agentType: agent.type })
        })
        if (gitRepoStatus[project.path] !== true) continue

        // Existing worktrees — one command per cached worktree
        const worktrees = (worktreeCache.get(project.path) ?? []).filter((wt) => !wt.isMain)
        for (const wt of worktrees) {
          commands.push({
            id: `quicklaunch:${agent.type}:${project.name}:wt:${wt.path}`,
            label: `${agent.displayName} on ${project.name} › ${wt.name}`,
            sublabel: wt.branch === wt.name ? 'Existing worktree' : `Branch: ${wt.branch}`,
            category: 'quicklaunch',
            icon: <AgentIcon agentType={agent.type} size={14} />,
            keywords: ['launch', 'start', 'run', 'worktree', 'branch', wt.name, wt.branch],
            onExecute: () =>
              createSessionFromProject(project, {
                agentType: agent.type,
                branch: wt.branch,
                existingWorktreePath: wt.path
              })
          })
        }

        // New worktree from current branch
        commands.push({
          id: `quicklaunch:${agent.type}:${project.name}:worktree`,
          label: `${agent.displayName} on ${project.name} (new worktree)`,
          sublabel: 'Isolated worktree from current branch',
          category: 'quicklaunch',
          icon: <AgentIcon agentType={agent.type} size={14} />,
          keywords: ['launch', 'start', 'run', 'worktree', 'branch', 'isolated', 'fork', 'new'],
          onExecute: () =>
            createSessionFromProject(project, { agentType: agent.type, useWorktree: true })
        })
      }
    }

    // --- Quick Launch Terminal (project x worktree) ---
    for (const project of localProjects) {
      commands.push({
        id: `terminal:project:${project.name}`,
        label: `Terminal in ${project.name}`,
        category: 'quicklaunch',
        icon: <Terminal size={14} strokeWidth={1.5} />,
        keywords: ['terminal', 'shell', 'launch', project.name],
        onExecute: () => createShellInProject(project.path, { project })
      })
      if (gitRepoStatus[project.path] !== true) continue
      const worktrees = (worktreeCache.get(project.path) ?? []).filter((wt) => !wt.isMain)
      for (const wt of worktrees) {
        commands.push({
          id: `terminal:project:${project.name}:wt:${wt.path}`,
          label: `Terminal in ${project.name} › ${wt.name}`,
          sublabel: wt.branch === wt.name ? 'Existing worktree' : `Branch: ${wt.branch}`,
          category: 'quicklaunch',
          icon: <Terminal size={14} strokeWidth={1.5} />,
          keywords: ['terminal', 'shell', 'launch', 'worktree', wt.name, wt.branch],
          onExecute: () =>
            createShellInProject(wt.path, {
              project,
              worktreePath: wt.path,
              worktreeName: wt.name,
              branch: wt.branch
            })
        })
      }
    }

    // --- Filter & Sort ---
    const statusOptions: { value: StatusFilter; label: string; shortcut?: string }[] = [
      { value: 'all', label: 'Show All', shortcut: '\u23181' },
      { value: 'running', label: 'Show Running', shortcut: '\u23182' },
      { value: 'waiting', label: 'Show Waiting', shortcut: '\u23183' },
      { value: 'idle', label: 'Show Idle', shortcut: '\u23184' },
      { value: 'error', label: 'Show Errors', shortcut: '\u23185' }
    ]
    for (const opt of statusOptions) {
      commands.push({
        id: `filter:status:${opt.value}`,
        label: opt.label,
        category: 'filter',
        icon: <Filter size={14} strokeWidth={1.5} />,
        shortcutDisplay: opt.shortcut,
        keywords: ['filter', 'status'],
        onExecute: () => setStatusFilter(opt.value)
      })
    }
    const sortOptions: { value: SortMode; label: string }[] = [
      { value: 'manual', label: 'Sort: Manual' },
      { value: 'created', label: 'Sort: Created' },
      { value: 'recent', label: 'Sort: Recent Activity' }
    ]
    for (const opt of sortOptions) {
      commands.push({
        id: `filter:sort:${opt.value}`,
        label: opt.label,
        category: 'filter',
        icon: <ArrowUpDown size={14} strokeWidth={1.5} />,
        keywords: ['sort', 'order'],
        onExecute: () => setSortMode(opt.value)
      })
    }

    return commands
  }, [
    terminals,
    config,
    recentSessions,
    installStatus,
    gitRepoStatus,
    worktreeCache,
    addTerminal,
    setFocusedTerminal,
    setActiveProject,
    setNewAgentDialogOpen,
    setAddProjectDialogOpen,
    setWorkflowEditorOpen,
    setSettingsOpen,
    toggleSidebar,
    setSortMode,
    setStatusFilter,
    setMainViewMode,
    setTaskDialogOpen,
    setOnboardingOpen
  ])
}

export function CommandPalette() {
  const isOpen = useAppStore((s) => s.isCommandPaletteOpen)
  const setOpen = useAppStore((s) => s.setCommandPaletteOpen)

  const config = useAppStore((s) => s.config)
  const loadWorktrees = useAppStore((s) => s.loadWorktrees)
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const [recentSessions, setRecentSessions] = useState<RecentSession[]>([])
  const [gitRepoStatus, setGitRepoStatus] = useState<Record<string, boolean>>({})
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const { status: installStatus } = useAgentInstallStatus()

  useEffect(() => {
    if (isOpen) {
      window.api
        .getRecentSessions()
        .then(setRecentSessions)
        .catch(() => setRecentSessions([]))

      const localProjects = (config?.projects ?? []).filter((p) =>
        getProjectHostIds(p).includes('local')
      )
      Promise.allSettled(
        localProjects.map(async (p) => {
          const isRepo = await window.api.isGitRepo(p.path)
          if (isRepo) loadWorktrees(p.path)
          return [p.path, isRepo] as const
        })
      ).then((results) => {
        const entries = results.map((r, i) =>
          r.status === 'fulfilled' ? r.value : ([localProjects[i].path, false] as const)
        )
        setGitRepoStatus(Object.fromEntries(entries))
      })
    }
  }, [isOpen, config?.projects, loadWorktrees])

  const commands = useCommands(recentSessions, installStatus, gitRepoStatus)

  const filtered = useMemo(() => {
    const q = query.trim()
    if (!q) {
      // Show all except quicklaunch when empty
      return commands.filter((c) => c.category !== 'quicklaunch')
    }
    return commands
      .map((cmd) => ({ cmd, score: scoreMatch(q, cmd) }))
      .filter(({ score }) => score > 0)
      .sort((a, b) => b.score - a.score)
      .map(({ cmd }) => cmd)
  }, [commands, query])

  // Grouped view (empty query) vs flat view (with query)
  const hasQuery = query.trim().length > 0

  const groupedItems = useMemo(() => {
    if (hasQuery) return null
    const groups: { category: CommandCategory; commands: Command[] }[] = []
    for (const cat of CATEGORY_ORDER) {
      const items = filtered.filter((c) => c.category === cat)
      if (items.length > 0) {
        groups.push({ category: cat, commands: items })
      }
    }
    return groups
  }, [filtered, hasQuery])

  // Build a flat list for keyboard navigation (includes headers for index math)
  const flatItems = useMemo(() => {
    if (hasQuery) return filtered
    const items: Command[] = []
    for (const group of groupedItems ?? []) {
      items.push(...group.commands)
    }
    return items
  }, [filtered, groupedItems, hasQuery])

  // Reset state when opening/closing
  useEffect(() => {
    if (isOpen) {
      setQuery('') // eslint-disable-line react-hooks/set-state-in-effect -- resetting ephemeral UI state on open/close
      setActiveIndex(0)
    }
  }, [isOpen])

  // Reset active index when query changes
  useEffect(() => {
    setActiveIndex(0) // eslint-disable-line react-hooks/set-state-in-effect -- derived reset
  }, [query])

  // Scroll active item into view
  useEffect(() => {
    const el = listRef.current?.querySelector(`[data-cmd-index="${activeIndex}"]`)
    el?.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  const close = () => setOpen(false)

  const executeCommand = (cmd: Command) => {
    close()
    cmd.onExecute()
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActiveIndex((i) => (i + 1) % Math.max(flatItems.length, 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActiveIndex((i) => (i - 1 + flatItems.length) % Math.max(flatItems.length, 1))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const cmd = flatItems[activeIndex]
      if (cmd) executeCommand(cmd)
    }
  }

  // Build render list with category headers
  const renderItems = useMemo(() => {
    if (hasQuery) {
      return filtered.map((cmd, idx) => ({ type: 'command' as const, cmd, flatIndex: idx }))
    }
    let flatIndex = 0
    const result: (
      | { type: 'header'; label: string }
      | { type: 'command'; cmd: Command; flatIndex: number }
    )[] = []
    for (const group of groupedItems ?? []) {
      result.push({ type: 'header', label: CATEGORY_LABELS[group.category] })
      for (const cmd of group.commands) {
        result.push({ type: 'command', cmd, flatIndex })
        flatIndex++
      }
    }
    return result
  }, [filtered, groupedItems, hasQuery])

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div
            className="fixed inset-0 bg-black/40 z-[60]"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={close}
          />

          <motion.div
            className="fixed top-[20%] left-1/2 z-[60] w-[560px] max-h-[420px] border border-white/[0.08]
                       rounded-xl shadow-2xl overflow-hidden flex flex-col"
            style={{ background: '#1e1e22' }}
            initial={{ opacity: 0, scale: 0.98, x: '-50%', y: -8 }}
            animate={{ opacity: 1, scale: 1, x: '-50%', y: 0 }}
            exit={{ opacity: 0, scale: 0.98, x: '-50%', y: -8 }}
            transition={{ duration: 0.15 }}
          >
            {/* Search input */}
            <div className="px-4 py-3 border-b border-white/[0.06] flex items-center gap-3">
              <Search size={16} className="text-gray-500 shrink-0" strokeWidth={1.5} />
              <input
                ref={inputRef}
                autoFocus
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder="Type a command..."
                className="flex-1 bg-transparent text-sm text-gray-200
                           placeholder-gray-600 outline-none"
              />
              <kbd
                className="text-[10px] text-gray-600 bg-white/[0.04]
                              border border-white/[0.06] px-1.5 py-0.5 rounded font-mono"
              >
                ESC
              </kbd>
            </div>

            {/* Results */}
            <div ref={listRef} className="overflow-auto flex-1 py-1">
              {flatItems.length === 0 && (
                <div className="px-4 py-8 text-center text-sm text-gray-600">
                  No matching commands
                </div>
              )}

              {renderItems.map((item) => {
                if (item.type === 'header') {
                  return (
                    <div
                      key={`header-${item.label}`}
                      className="px-3 py-1.5 text-[11px] font-medium text-gray-500
                                 uppercase tracking-wider sticky top-0"
                      style={{ background: '#1e1e22' }}
                    >
                      {item.label}
                    </div>
                  )
                }

                const { cmd, flatIndex } = item
                const isActive = flatIndex === activeIndex

                return (
                  <button
                    key={cmd.id}
                    data-cmd-index={flatIndex}
                    onClick={() => executeCommand(cmd)}
                    onMouseEnter={() => setActiveIndex(flatIndex)}
                    className={`w-full px-3 py-2 flex items-center gap-3 text-left
                                transition-colors text-[13px] ${
                                  isActive
                                    ? 'bg-white/[0.06] text-white'
                                    : 'text-gray-300 hover:bg-white/[0.04]'
                                }`}
                  >
                    <span className="shrink-0 w-5 flex justify-center text-gray-500">
                      {cmd.icon}
                    </span>
                    <span className="flex-1 min-w-0">
                      <span className="truncate block">
                        {hasQuery ? highlightMatch(cmd.label, query) : cmd.label}
                      </span>
                      {cmd.sublabel && (
                        <span className="text-[10px] text-gray-600 truncate block">
                          {cmd.sublabel}
                        </span>
                      )}
                    </span>
                    {cmd.shortcutDisplay && (
                      <kbd
                        className="text-[10px] text-gray-500 bg-white/[0.04]
                                      border border-white/[0.06] px-1.5 py-0.5 rounded font-mono shrink-0"
                      >
                        {cmd.shortcutDisplay}
                      </kbd>
                    )}
                  </button>
                )
              })}
            </div>

            {/* Footer hints */}
            <div
              className="px-4 py-2 border-t border-white/[0.06] flex items-center gap-4
                            text-[11px] text-gray-600"
            >
              <span className="flex items-center gap-1">
                <kbd className="px-1 bg-white/[0.04] border border-white/[0.06] rounded text-[10px] font-mono">
                  &uarr;&darr;
                </kbd>
                navigate
              </span>
              <span className="flex items-center gap-1">
                <kbd className="px-1 bg-white/[0.04] border border-white/[0.06] rounded text-[10px] font-mono">
                  &crarr;
                </kbd>
                select
              </span>
              <span className="flex items-center gap-1">
                <kbd className="px-1 bg-white/[0.04] border border-white/[0.06] rounded text-[10px] font-mono">
                  esc
                </kbd>
                close
              </span>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}
