import { useEffect, useState, Suspense, lazy } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { AnimatePresence } from 'framer-motion'
import { useAppStore } from './stores'
import { GridView } from './components/GridView'
import { TabView } from './components/TabView'
import { MobileSinglePane } from './components/MobileSinglePane'
import { FocusedTerminal } from './components/FocusedTerminal'
import { TerminalHost } from './components/TerminalHost'
import { ProjectSidebar } from './components/project-sidebar/ProjectSidebar'
import { PromptLauncher } from './components/PromptLauncher'
import { AddProjectDialog } from './components/AddProjectDialog'
const WorkflowEditor = lazy(() =>
  import('./components/workflow-editor/WorkflowEditor').then((m) => ({ default: m.WorkflowEditor }))
)
const WorkflowsLandingView = lazy(() =>
  import('./components/workflow-runs/WorkflowsLandingView').then((m) => ({
    default: m.WorkflowsLandingView
  }))
)
import {
  executeWorkflow as runWorkflow,
  rescheduleWaitingGateTimers,
  reconcileRunningExecutions
} from './lib/workflow-execution'
import type { WorkflowExecution } from '../shared/types'
import { CommandPalette } from './components/CommandPalette'
import { SessionRestoredBanner } from './components/SessionRestoredBanner'
import { GridToolbar } from './components/GridToolbar'
import { ToolbarBreadcrumb } from './components/ToolbarBreadcrumb'
import { SettingsPage } from './components/SettingsPage'
import { SidebarToggleButton } from './components/SidebarToggleButton'
import { MainViewPills } from './components/MainViewPills'
import { ToolbarMinimizedStrip } from './components/ToolbarMinimizedStrip'
import { RecentSessionsButton } from './components/RecentSessionsButton'
import { Tooltip } from './components/Tooltip'
import { Plus, Menu } from 'lucide-react'
import { MobileBottomTabs } from './components/MobileBottomTabs'
import { TaskToolbar } from './components/TaskToolbar'
import { WorkflowsLandingHeader } from './components/workflow-runs/WorkflowsLandingHeader'
import { useKeyboardShortcuts } from './hooks/useKeyboardShortcuts'
import { useVirtualKeyboard } from './hooks/useVirtualKeyboard'
import { useGitDiffPolling } from './hooks/useGitDiffPolling'
import { consumePendingTerminalClose } from './lib/terminal-close'
import {
  setDefaultFontSize,
  initGlobalDataListener,
  disposeGlobalDataListener
} from './lib/terminal-registry'
import { WorktreeCleanupDialog } from './components/WorktreeCleanupDialog'
import { WorktreeCleanupToastBridge } from './components/WorktreeCleanupToastBridge'
import { RightPanel } from './components/RightPanel'
import { TaskBoardView } from './components/TaskBoardView'
import { TaskDetailPanel } from './components/TaskDetailPanel'
import { KeyboardShortcutsPanel } from './components/KeyboardShortcutsPanel'
import { MissedScheduleDialog } from './components/MissedScheduleDialog'
import { SourcePromptDialog } from './components/SourcePromptDialog'
import { OnboardingModal } from './components/OnboardingModal'
import { UpdateBanner } from './components/UpdateBanner'
import { ToastContainer } from './components/Toast'
import { AddTaskDialog } from './components/AddTaskDialog'
import { GridContextMenu } from './components/GridContextMenu'
import { WindowControls } from './components/WindowControls'
import { isMac, isWeb, TRAFFIC_LIGHT_PAD_PX } from './lib/platform'
import { useIsMobile } from './hooks/useIsMobile'
import { resolveResumeSessionId, buildRestorePayload } from './lib/session-utils'

export function App() {
  const {
    focusedId,
    previewId,
    showBanner,
    isSidebarOpen,
    isSettingsOpen,
    isShortcutsPanelOpen,
    isOnboardingOpen,
    isWorkflowEditorOpen,
    editingWorkflowId,
    layoutMode,
    mainViewMode,
    minimizedPlacement,
    selectedTaskId,
    diffSidebarTerminalId
  } = useAppStore(
    useShallow((s) => ({
      focusedId: s.focusedTerminalId,
      previewId: s.previewTerminalId,
      showBanner: s.showSessionBanner,
      isSidebarOpen: s.isSidebarOpen,
      isSettingsOpen: s.isSettingsOpen,
      isShortcutsPanelOpen: s.isShortcutsPanelOpen,
      isOnboardingOpen: s.isOnboardingOpen,
      isWorkflowEditorOpen: s.isWorkflowEditorOpen,
      editingWorkflowId: s.editingWorkflowId,
      layoutMode: s.config?.defaults?.layoutMode ?? 'grid',
      mainViewMode: s.config?.defaults?.mainViewMode ?? 'sessions',
      minimizedPlacement: s.config?.defaults?.minimizedPlacement ?? 'toolbar',
      selectedTaskId: s.selectedTaskId,
      diffSidebarTerminalId: s.diffSidebarTerminalId
    }))
  )
  const setDialogOpen = useAppStore((s) => s.setNewAgentDialogOpen)
  const toggleSidebar = useAppStore((s) => s.toggleSidebar)
  const [topPlusMenuPos, setTopPlusMenuPos] = useState<{ x: number; y: number } | null>(null)
  const isMobile = useIsMobile()
  const isInlineWorkflowEditor =
    mainViewMode === 'workflows' &&
    !isMobile &&
    (editingWorkflowId !== null || isWorkflowEditorOpen)

  const isTabToolbarMerged =
    layoutMode === 'tabs' && mainViewMode === 'sessions' && !isMobile && !focusedId && !previewId

  // Only hide toolbar on macOS focused view — Windows/Linux need it for window controls
  const isFocusedFullScreen = isMac && !isMobile && (!!focusedId || !!previewId)

  // On mobile, auto-close sidebar on initial load
  useEffect(() => {
    if (isMobile && isSidebarOpen) {
      toggleSidebar()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []) // Only on mount

  useKeyboardShortcuts()
  const { keyboardHeight } = useVirtualKeyboard()
  useGitDiffPolling()

  // Load config and previous sessions on mount
  useEffect(() => {
    initGlobalDataListener()
    ;(async () => {
      try {
        const [config, prev] = await Promise.all([
          window.api.loadConfig(),
          window.api.getPreviousSessions()
        ])
        useAppStore.getState().setConfig(config)
        if (config.defaults.fontSize) {
          setDefaultFontSize(config.defaults.fontSize)
        }

        // Request notification permission if enabled
        if (config.defaults.notifications?.enabled && Notification.permission === 'default') {
          Notification.requestPermission()
        }

        // Version 2 = redesigned 7-step wizard; show to users who haven't seen it
        const ONBOARDING_VERSION = 2
        if (Number(config.defaults.hasSeenOnboarding ?? 0) < ONBOARDING_VERSION) {
          useAppStore.getState().setOnboardingOpen(true)
        }
        // Web: hydrate already-running sessions that started before we connected
        if (isWeb && 'listActiveSessions' in window.api) {
          try {
            const active = (await (
              window.api as { listActiveSessions: () => Promise<unknown[]> }
            ).listActiveSessions()) as import('../shared/types').TerminalSession[]
            const state = useAppStore.getState()
            for (const session of active) {
              if (!state.terminals.has(session.id)) {
                state.addTerminal(session)
              }
            }
          } catch (err) {
            console.error('[App] failed to hydrate active sessions:', err)
          }
        }

        if (prev && prev.length > 0) {
          if (config.defaults.reopenSessions) {
            // Auto-restore sessions — prefer hook-correlated session ID (exact),
            // fall back to scanning agent history when hooks weren't active.
            // Shells restore as fresh PTYs in their saved cwd (no resume concept).
            const claimed = new Set<string>()
            for (const s of prev) {
              if (s.agentType === 'shell') {
                const cwd = s.shellCwd ?? s.worktreePath ?? s.projectPath
                const session = await window.api.createShellTerminal(cwd)
                const restored =
                  s.projectName && s.projectPath
                    ? {
                        ...session,
                        projectName: s.projectName,
                        projectPath: s.projectPath,
                        worktreePath: s.worktreePath,
                        worktreeName: s.worktreeName,
                        branch: s.branch,
                        isWorktree: s.isWorktree
                      }
                    : session
                useAppStore.getState().addTerminal(restored)
                continue
              }
              const resumeSessionId = await resolveResumeSessionId(s, claimed)
              if (resumeSessionId) claimed.add(resumeSessionId)
              const session = await window.api.createTerminal(
                buildRestorePayload(s, resumeSessionId)
              )
              useAppStore.getState().addTerminal(session)
            }
            window.api.clearPreviousSessions()
          } else {
            useAppStore.getState().setSessionBanner(true, prev)
          }
        }
      } catch (err) {
        console.error('[App] startup initialization failed:', err)
      }
    })()

    const removeExitListener = window.api.onTerminalExit(({ id }) => {
      const state = useAppStore.getState()
      if (consumePendingTerminalClose(id)) {
        const terminal = state.terminals.get(id)
        if (terminal) {
          state.removeTerminal(id)
          if (terminal.session.projectPath) {
            state.loadWorktrees(terminal.session.projectPath)
          }
        }
        const assignedTask = (state.config?.tasks || []).find(
          (t) => t.assignedSessionId === id && t.status === 'in_progress'
        )
        if (assignedTask) {
          state.reviewTask(assignedTask.id)
        }
        return
      }

      const terminal = state.terminals.get(id)
      if (!terminal) return

      state.updateStatus(id, 'idle')

      if (terminal.session.agentType !== 'shell') {
        const assignedTask = (state.config?.tasks || []).find(
          (t) => t.assignedSessionId === id && t.status === 'in_progress'
        )
        if (assignedTask) {
          state.reviewTask(assignedTask.id)
        }
      }
    })

    const removeSessionCreatedListener = window.api.onSessionCreated((session) => {
      const state = useAppStore.getState()
      if (!state.terminals.has(session.id)) {
        state.addTerminal(session)
        if (session.projectPath) {
          state.loadWorktrees(session.projectPath)
        }
      }
    })

    const removeConfigListener = window.api.onConfigChanged((config) => {
      useAppStore.getState().setConfig(config)
    })

    const removeMenuListener = window.api.onMenuNewAgent(() => {
      useAppStore.getState().setNewAgentDialogOpen(true)
    })

    const removeWidgetSelectListener = window.api.onWidgetSelectTerminal((terminalId) => {
      useAppStore.getState().setFocusedTerminal(terminalId)
    })

    // Scheduler: auto-execute workflows when triggered
    const removeSchedulerListener = window.api.onSchedulerExecute(
      async ({ workflowId, connectorItem }) => {
        const state = useAppStore.getState()
        const workflow = state.config?.workflows?.find((w) => w.id === workflowId)
        if (!workflow) return

        const context = connectorItem ? { connectorItem } : undefined
        await runWorkflow(workflow, context, { source: 'scheduler' })
      }
    )

    const removeUpdateListener = window.api.onUpdateDownloaded(({ version }) => {
      useAppStore.getState().setUpdateVersion(version)
    })

    const removeSessionUpdatedListener = window.api.onSessionUpdated((session) => {
      const store = useAppStore.getState()
      const existing = store.terminals.get(session.id)
      if (existing) {
        if (session.status !== existing.status) {
          store.updateStatus(session.id, session.status)
        }
        if (session.branch && existing.session.branch !== session.branch) {
          store.updateSessionBranch(session.id, session.branch)
          if (existing.session.projectPath) {
            store.loadWorktrees(existing.session.projectPath)
          }
        }
        if (session.displayName && existing.session.displayName !== session.displayName) {
          store.renameTerminal(session.id, session.displayName)
        }
        const wtUpdates: { worktreePath?: string; worktreeName?: string } = {}
        if (session.worktreePath && session.worktreePath !== existing.session.worktreePath) {
          wtUpdates.worktreePath = session.worktreePath
        }
        if (session.worktreeName && session.worktreeName !== existing.session.worktreeName) {
          wtUpdates.worktreeName = session.worktreeName
        }
        if (Object.keys(wtUpdates).length > 0) {
          store.updateSessionWorktree(session.id, wtUpdates)
        }
      } else {
        const updates: { branch?: string; worktreePath?: string; worktreeName?: string } = {}
        if (session.branch) updates.branch = session.branch
        if (session.worktreePath) updates.worktreePath = session.worktreePath
        if (session.worktreeName) updates.worktreeName = session.worktreeName
        if (Object.keys(updates).length > 0) {
          store.updateHeadlessSession(session.id, updates)
        }
      }
    })

    // Headless agent tracking
    const removeHeadlessExitListener = window.api.onHeadlessExit(({ id, exitCode }) => {
      useAppStore.getState().updateHeadlessSession(id, {
        status: 'exited',
        exitCode,
        endedAt: Date.now()
      })
    })

    const removeHeadlessDataListener = window.api.onHeadlessData(({ id, data }) => {
      const lines = data.split('\n').filter((l) => l.trim())
      if (lines.length > 0) {
        useAppStore.getState().setHeadlessLastOutput(id, lines[lines.length - 1])
      }
    })

    // Poll headless sessions every 5s for sync
    const pollHeadless = async (): Promise<void> => {
      try {
        const sessions = await window.api.listHeadlessSessions()
        useAppStore.getState().setHeadlessSessions(sessions)
      } catch {
        // ignore — server may not be ready yet
      }
    }
    pollHeadless()
    const headlessPollInterval = setInterval(pollHeadless, 5000)

    window.api
      .listRunsWithWaitingGates()
      .then((runs) => {
        const store = useAppStore.getState()
        const hydrated: WorkflowExecution[] = []
        for (const run of runs) {
          if (store.workflowExecutions.has(run.workflowId)) continue
          store.setWorkflowExecution(run.workflowId, run)
          hydrated.push(run)
        }
        rescheduleWaitingGateTimers(hydrated, store.config?.workflows ?? [])
      })
      .catch((err) => console.error('[App] failed to hydrate waiting gates:', err))

    // Resolve runs the previous renderer left in `running`. The main process
    // keeps headless agents alive past a renderer reload, but the in-memory
    // exit-promise dies — the run wedges. Reconcile against session_events
    // and close out anything that already exited.
    window.api
      .listRunningWorkflowRuns()
      .then((runs) => {
        const store = useAppStore.getState()
        for (const run of runs) {
          if (!store.workflowExecutions.has(run.workflowId)) {
            store.setWorkflowExecution(run.workflowId, run)
          }
        }
        return reconcileRunningExecutions(runs)
      })
      .catch((err) => console.error('[App] failed to reconcile running runs:', err))

    // Auto-prune exited headless sessions
    const pruneInterval = setInterval(() => {
      const retentionMinutes =
        useAppStore.getState().config?.defaults?.headlessRetentionMinutes ?? 1
      useAppStore.getState().pruneExitedHeadless(retentionMinutes * 60_000)
    }, 30_000)

    return () => {
      disposeGlobalDataListener()
      removeExitListener()
      removeSessionCreatedListener()
      removeConfigListener()
      removeMenuListener()
      removeSchedulerListener()
      removeWidgetSelectListener()
      removeUpdateListener()
      removeSessionUpdatedListener()
      removeHeadlessExitListener()
      removeHeadlessDataListener()
      clearInterval(headlessPollInterval)
      clearInterval(pruneInterval)
    }
  }, [])

  return (
    <div
      className="flex h-dvh text-gray-100"
      style={{
        background: '#1a1a1e',
        paddingTop: 'var(--safe-top)',
        paddingLeft: 'var(--safe-left)',
        paddingRight: 'var(--safe-right)',
        paddingBottom: 'calc(var(--safe-bottom) + var(--keyboard-height, 0px))'
      }}
    >
      <ProjectSidebar />

      <main
        className="flex-1 flex flex-col overflow-hidden"
        style={
          isMobile && keyboardHeight === 0
            ? { paddingBottom: 'calc(64px + var(--safe-bottom, 0px))' }
            : undefined
        }
      >
        {/* z-46 + opaque bg covers the TerminalHost overlay (z-45) when the grid scrolls up. */}
        {!isInlineWorkflowEditor && !isTabToolbarMerged && !isFocusedFullScreen && (
          <div
            className={`titlebar-drag shrink-0 border-b border-white/[0.06] relative z-[46] bg-[#1a1a1e]
                        flex items-center ${isMobile ? 'px-2 justify-between' : 'px-3'} ${isMobile ? 'h-[52px]' : 'h-[40px]'}`}
            style={
              isMac && !isWeb && !isSidebarOpen && !isMobile
                ? { paddingLeft: `${TRAFFIC_LIGHT_PAD_PX}px` }
                : undefined
            }
          >
            <div className={`flex items-center titlebar-no-drag ${isMobile ? 'gap-2.5' : 'gap-1'}`}>
              {isMobile && (
                <button
                  onClick={toggleSidebar}
                  className="text-gray-400 hover:text-white active:text-white p-2 transition-colors rounded-full"
                  style={{
                    background: 'var(--glass-bg, transparent)',
                    backdropFilter: 'var(--glass-blur, none)',
                    WebkitBackdropFilter: 'var(--glass-blur, none)',
                    boxShadow: 'var(--glass-shadow, none)'
                  }}
                  title="Show sidebar"
                >
                  <Menu size={20} strokeWidth={2} />
                </button>
              )}
              {!isMobile && !isSidebarOpen && (
                <>
                  <SidebarToggleButton />
                  <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
                  <MainViewPills />
                </>
              )}
              {!isMobile &&
                layoutMode === 'grid' &&
                mainViewMode === 'sessions' &&
                minimizedPlacement !== 'canvas' && <ToolbarMinimizedStrip />}
            </div>
            {!isMobile && (
              <div className="flex-1 flex justify-center min-w-0 titlebar-no-drag">
                <ToolbarBreadcrumb />
              </div>
            )}
            <div className={`flex items-center titlebar-no-drag ${isMobile ? 'gap-1.5' : 'gap-1'}`}>
              {mainViewMode === 'workflows' && !isMobile ? (
                editingWorkflowId === null && !isWorkflowEditorOpen ? (
                  <WorkflowsLandingHeader />
                ) : null
              ) : mainViewMode !== 'tasks' ? (
                <>
                  {!isMobile && (
                    <>
                      <GridToolbar />
                      <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
                      <RecentSessionsButton />
                    </>
                  )}
                  {isMobile ? (
                    <button
                      onClick={() => setDialogOpen(true)}
                      className="p-2.5 text-xs rounded-full font-medium text-gray-200 hover:text-white active:bg-white/[0.15] transition-colors"
                      style={{
                        background: 'var(--glass-bg, rgba(255,255,255,0.06))',
                        backdropFilter: 'var(--glass-blur, none)',
                        WebkitBackdropFilter: 'var(--glass-blur, none)',
                        boxShadow: 'var(--glass-shadow, none)'
                      }}
                    >
                      <Plus size={18} strokeWidth={2} />
                    </button>
                  ) : (
                    <Tooltip
                      label="New session"
                      shortcut={`${isMac ? '⌘' : 'Ctrl+'}N`}
                      position="bottom"
                    >
                      <button
                        onClick={(e) => {
                          if (topPlusMenuPos) {
                            setTopPlusMenuPos(null)
                            return
                          }
                          const rect = e.currentTarget.getBoundingClientRect()
                          setTopPlusMenuPos({ x: rect.right - 220, y: rect.bottom + 4 })
                        }}
                        className="p-1 text-gray-400 hover:text-white hover:bg-white/[0.06] rounded-md transition-colors"
                      >
                        <Plus size={16} strokeWidth={2} />
                      </button>
                    </Tooltip>
                  )}
                </>
              ) : (
                <>
                  {!isMobile && <TaskToolbar />}
                  {isMobile ? (
                    <button
                      onClick={() => useAppStore.getState().setTaskDialogOpen(true)}
                      className="p-2.5 text-xs rounded-full font-medium text-gray-200 hover:text-white active:bg-white/[0.15] transition-colors"
                      style={{
                        background: 'var(--glass-bg, rgba(255,255,255,0.06))',
                        backdropFilter: 'var(--glass-blur, none)',
                        WebkitBackdropFilter: 'var(--glass-blur, none)',
                        boxShadow: 'var(--glass-shadow, none)'
                      }}
                    >
                      <Plus size={18} strokeWidth={2} />
                    </button>
                  ) : (
                    <Tooltip label="Add task" position="bottom">
                      <button
                        onClick={() => useAppStore.getState().setTaskDialogOpen(true)}
                        className="p-1 text-gray-400 hover:text-white hover:bg-white/[0.06] rounded-md transition-colors"
                      >
                        <Plus size={16} strokeWidth={2} />
                      </button>
                    </Tooltip>
                  )}
                </>
              )}
              <WindowControls />
            </div>
          </div>
        )}

        {showBanner && <SessionRestoredBanner />}
        <UpdateBanner />
        <div className="flex-1 flex min-h-0">
          <div className="flex-1 min-w-0 flex flex-col min-h-0">
            {mainViewMode === 'tasks' ? (
              <TaskBoardView />
            ) : mainViewMode === 'workflows' && !isMobile ? (
              <Suspense fallback={null}>
                {editingWorkflowId !== null || isWorkflowEditorOpen ? (
                  <WorkflowEditor inline />
                ) : (
                  <WorkflowsLandingView />
                )}
              </Suspense>
            ) : mainViewMode === 'workflows' && isMobile ? (
              <div className="flex-1 flex items-center justify-center text-gray-500 text-sm px-6 text-center">
                Open the sidebar to pick a workflow
              </div>
            ) : isMobile ? (
              <MobileSinglePane />
            ) : focusedId || previewId ? (
              <FocusedTerminal />
            ) : layoutMode === 'tabs' ? (
              <TabView />
            ) : (
              <GridView />
            )}
          </div>
          {mainViewMode === 'tasks' && selectedTaskId && <TaskDetailPanel />}
          {mainViewMode !== 'tasks' &&
            mainViewMode !== 'workflows' &&
            !isMobile &&
            diffSidebarTerminalId && <RightPanel />}
        </div>
        {isMobile && <MobileBottomTabs hidden={keyboardHeight > 0} />}
      </main>

      {/* Focus overlay — mobile only (desktop renders inline in content area) */}
      {isMobile && focusedId && <FocusedTerminal />}

      <TerminalHost />

      <PromptLauncher mode="overlay" onClose={() => setDialogOpen(false)} />
      {topPlusMenuPos && (
        <GridContextMenu position={topPlusMenuPos} onClose={() => setTopPlusMenuPos(null)} />
      )}
      <AddProjectDialog />
      {isWorkflowEditorOpen && (mainViewMode !== 'workflows' || isMobile) && (
        <Suspense fallback={null}>
          <WorkflowEditor />
        </Suspense>
      )}
      <CommandPalette />
      <AddTaskDialog />
      <WorktreeCleanupDialog />
      <WorktreeCleanupToastBridge />
      <MissedScheduleDialog />
      <SourcePromptDialog />
      <AnimatePresence>{isShortcutsPanelOpen && <KeyboardShortcutsPanel />}</AnimatePresence>

      <AnimatePresence>{isSettingsOpen && <SettingsPage />}</AnimatePresence>

      <AnimatePresence>{isOnboardingOpen && <OnboardingModal />}</AnimatePresence>

      <ToastContainer />
    </div>
  )
}
