import { schedulerExecutionContext } from './lib/workflow-helpers'
import { useEffect, useState, Suspense, lazy } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { AnimatePresence } from 'framer-motion'
import { useAppStore } from './stores'
import { GridView } from './components/GridView'
import { TabView } from './components/TabView'
import { MobileSinglePane } from './components/MobileSinglePane'
import { FocusedStage } from './components/FocusedStage'
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
  adoptConnectorInboxLease,
  executeWorkflow as runWorkflow,
  rescheduleWaitingGateTimers,
  reconcileRunningExecutions,
  stopWorkflowRun,
  applyGateDecision
} from './lib/workflow-execution'
import type { WorkflowExecution } from '../shared/types'
import { CommandPalette } from './components/CommandPalette'
import { SessionRestoredBanner } from './components/SessionRestoredBanner'
import { GridToolbar } from './components/GridToolbar'
import { ToolbarBreadcrumb } from './components/ToolbarBreadcrumb'
import { SettingsPage } from './components/SettingsPage'
import { AppNavCluster } from './components/AppNavCluster'
import { useFocusedTitlebar } from './hooks/useFocusedTitlebar'
import { SessionDock } from './components/SessionDock'
import { HeadlessBadge } from './components/HeadlessBadge'
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
  disposeGlobalDataListener,
  setKeyRedirectHandler,
  setNotLiveReporter
} from './lib/terminal-registry'
import {
  setCwdReporter,
  getShellInputState,
  isAtPrompt,
  setDomBlockRendering
} from './lib/command-blocks'
import { focusIntentBar } from './lib/intent-bar-focus'
import { WorktreeCleanupDialog } from './components/WorktreeCleanupDialog'
import { WorktreeCleanupToastBridge } from './components/WorktreeCleanupToastBridge'
import { RightPanel } from './components/RightPanel'
import { TaskBoardView } from './components/TaskBoardView'
import { TaskDetailPanel } from './components/TaskDetailPanel'
import { KeyboardShortcutsPanel } from './components/KeyboardShortcutsPanel'
import { MissedScheduleDialog } from './components/MissedScheduleDialog'
import { SourcePromptDialog } from './components/SourcePromptDialog'
import { OnboardingModal } from './components/OnboardingModal'
import { ToastContainer, toast } from './components/Toast'
import { AddTaskDialog } from './components/AddTaskDialog'
import { GridContextMenu } from './components/GridContextMenu'
import { WindowControls } from './components/WindowControls'
import { isMac, isWeb, TRAFFIC_LIGHT_PAD_PX } from './lib/platform'
import { useIsMobile } from './hooks/useIsMobile'
import { syncBoard } from './lib/board-sync'
import { shouldNotifyBell, sendAgentNotification } from './lib/notifications'
import { restoreDevicePanes } from './lib/device-restore'
import { markPaneEnded } from './lib/session-resume'

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

  // Hidden only on macOS, where a focused session's own header takes the bar
  // over. Windows and Linux keep theirs — it holds their window controls.
  const { ownsTitlebar: isFocusedFullScreen } = useFocusedTitlebar()

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
  const domBlockSetting = useAppStore((s) => s.config?.defaults.domBlockRendering ?? true)
  useEffect(() => {
    setDomBlockRendering(domBlockSetting)
  }, [domBlockSetting])

  useEffect(() => {
    initGlobalDataListener()
    // An attach that finds nothing running is how a window opened onto a dead
    // terminal learns it is looking at a photograph. Start-up reconciliation
    // cannot tell it: the session was not there when this client started.
    setNotLiveReporter(markPaneEnded)
    // The capture path has to know before any command finishes, so it is read
    // from config rather than passed down through the view tree.
    setDomBlockRendering(useAppStore.getState().config?.defaults.domBlockRendering ?? true)
    setCwdReporter((terminalId, cwd) => {
      useAppStore.getState().updateSessionCwd(terminalId, cwd)
    })
    // Shell sessions: while the shell waits at its prompt, plain typing in
    // the raw terminal belongs to the intent bar — focus it so the character
    // lands there. Running commands, TUIs, and sessions without integration
    // markers keep raw input.
    setKeyRedirectHandler((terminalId, e) => {
      if (e.metaKey || e.ctrlKey) return false
      if (e.key.length !== 1) return false
      const session = useAppStore.getState().terminals.get(terminalId)?.session
      if (session?.agentType !== 'shell') return false
      if (!isAtPrompt(getShellInputState(terminalId))) return false
      return focusIntentBar(terminalId)
    })
    const configLoaded = (async () => {
      try {
        const config = await window.api.loadConfig()
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
        // What the server actually has, asked before what the database
        // remembers, because they are different questions and only one of them
        // is about the present. The desktop could not ask this until now, so a
        // restart read the saved list and launched a replacement for every
        // session -- including the ones that had never stopped.
        //
        // The same reconciliation runs again whenever the server is replaced;
        // see `board-sync`.
        const reopen = config.defaults.reopenSessions ?? true
        // On means both halves of what it has always meant: the panes come back,
        // and the sessions behind them are started again. Only the ones that were
        // stopped -- see `resumeAll`.
        await syncBoard({ showCold: reopen, resume: reopen })

        // After the sessions, because a device is claimed for one and there is
        // nothing to attach a pane to until they are on the board. Not awaited
        // with them either: each claim boots a simulator, and the window must
        // not wait on `simctl bootstatus` to finish opening.
        void restoreDevicePanes()

        if (!reopen) {
          // Panes stay off the board, and the banner offers to bring them in.
          // It no longer relaunches anything: taking it shows the last screen
          // each terminal drew, and resuming is still a separate decision.
          const carried = await window.api.getRestoredSessions().catch(() => [])
          if (carried.length > 0) {
            useAppStore.getState().setSessionBanner(
              true,
              carried.map((one) => one.session)
            )
          }
        }
      } catch (err) {
        console.error('[App] startup initialization failed:', err)
      }
    })()

    // The server behind this app has been replaced -- it crashed and the
    // launcher started another. The bridge reconnects by itself, but nothing
    // about a pane changes when that happens: its terminal keeps showing what it
    // was showing, because that content lives here. So a frozen pane looks
    // exactly like a quiet one, and goes on taking input for a process that is
    // gone. Asking again is the only way any of them find out.
    const removeReplacedListener = window.api.onServerReplaced?.(() => {
      // The same rule as start-up, deliberately: a session stopped by a server
      // dying is stopped whether or not the app happened to be open at the time,
      // so one setting decides both.
      const reopen = useAppStore.getState().config?.defaults.reopenSessions ?? true
      void syncBoard({ showCold: true, resume: reopen })
    })

    // Pointed at a host while a server is still running on this machine. Said
    // out loud, because an agent working on a machine whose app is showing
    // somebody else's is invisible — and with the one action that resolves it,
    // taken by the person whose work it is.
    const removeLocalServerListener = window.api.onLocalServerStillRunning?.((notice) => {
      const count = notice.sessions === null ? 'Sessions are' : `${notice.sessions} session(s) are`
      toast(
        `${count} still running on this machine. They keep working, and switching back to local reconnects to them.`,
        'warning',
        {
          duration: Number.POSITIVE_INFINITY,
          actions: [
            {
              label: 'End them',
              tone: 'danger',
              onClick: async () => {
                const result = await window.api.stopLocalServer?.()
                toast(
                  result?.ok ? 'Stopped.' : (result?.error ?? 'Could not stop it.'),
                  result?.ok ? 'success' : 'error'
                )
              }
            }
          ]
        }
      )
    })

    // One listener for the whole app, not one per pane. A pane only receives
    // bytes for a terminal it has attached, so a bell hung off that reached you
    // for the sessions you were already looking at and missed the one ringing
    // out of view -- which is the only one worth interrupting anybody for.
    const removeBellListener = window.api.onTerminalBell?.(({ id }) => {
      const state = useAppStore.getState()
      const terminal = state.terminals.get(id)
      if (!terminal || !shouldNotifyBell(state.config)) return
      sendAgentNotification(terminal, 'bell', state.config, () =>
        useAppStore.getState().setFocusedTerminal(id)
      )
    })

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
      // And say so. A terminal whose process has gone looked exactly like one
      // that was merely quiet -- a frozen buffer, an idle dot, nothing to tell
      // them apart. That is the state a pane restored from a previous run is
      // in, reached from the other side, so it says the same thing.
      state.markEnded(id, {
        reason: 'exited',
        at: Date.now(),
        // Nothing was replayed: this pane watched it happen.
        replayed: false,
        ...(terminal.session.shellExitCode !== undefined && {
          exitCode: terminal.session.shellExitCode
        }),
        ...(terminal.session.shellCwd !== undefined && { cwd: terminal.session.shellCwd })
      })

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
      async ({
        workflowId,
        connectorItem,
        connectorInboxId,
        connectorInboxLeaseToken,
        existingExecution,
        inputs
      }) => {
        const state = useAppStore.getState()
        const workflow = state.config?.workflows?.find((w) => w.id === workflowId)
        if (!workflow) {
          if (connectorInboxId !== undefined && connectorInboxLeaseToken) {
            await window.api.completeConnectorInbox({
              id: connectorInboxId,
              leaseToken: connectorInboxLeaseToken,
              disposition: 'defer'
            })
          }
          return
        }

        if (existingExecution && connectorItem) {
          await adoptConnectorInboxLease(existingExecution, connectorItem)
          rescheduleWaitingGateTimers([existingExecution], [workflow])
          await reconcileRunningExecutions([existingExecution], [workflow])
          return
        }

        const context = schedulerExecutionContext(connectorItem, inputs)
        try {
          const execution = await runWorkflow(workflow, context, { source: 'scheduler' })
          // A different run may be parked on an approval gate. It did not
          // accept this event, so release the row for a short retry instead of
          // holding its full delivery lease.
          if (
            connectorInboxId !== undefined &&
            connectorInboxLeaseToken &&
            execution.connectorInboxId !== connectorInboxId
          ) {
            await window.api.completeConnectorInbox({
              id: connectorInboxId,
              leaseToken: connectorInboxLeaseToken,
              disposition: 'defer'
            })
          } else if (
            connectorItem &&
            connectorInboxLeaseToken &&
            execution.connectorInboxLeaseToken !== connectorInboxLeaseToken
          ) {
            await adoptConnectorInboxLease(execution, connectorItem)
          }
        } catch (err) {
          // Another renderer may have won the workflow claim. It will
          // acknowledge the shared inbox row; otherwise the lease expires and
          // the server retries it.
          console.warn('[connector] scheduled workflow did not complete:', err)
        }
      }
    )

    // Stop requests are broadcast to every instance, so most arrive here for a
    // run this window has never heard of. stopWorkflowRun already treats an
    // unknown id as a no-op, which is what makes the broadcast safe.
    const removeStopRunListener = window.api.onSchedulerStopRun(({ runId }) => {
      void stopWorkflowRun(runId).catch((err) =>
        console.warn(`[workflow] stop request for ${runId} failed:`, err)
      )
    })

    // A gate answered elsewhere — a phone, or another window. Broadcast for the
    // same reason a stop is: whoever answered is usually not who is holding the
    // run. applyGateDecision no-ops unless this instance is.
    const removeGateListener = window.api.onWorkflowGateResolved(({ runId, nodeId, decision }) => {
      void applyGateDecision(runId, nodeId, decision).catch((err) =>
        console.warn(`[workflow] gate decision for ${runId} failed:`, err)
      )
    })

    // Seed from main first: the events fire once, and a window opened after
    // one has already passed would otherwise sit on 'unsupported' forever.
    useAppStore.getState().setAppUpdateStatus(window.api.getUpdateStatus())
    const removeUpdateListener = window.api.onUpdateStatus((status) => {
      useAppStore.getState().setAppUpdateStatus(status)
    })

    // The agent's browser tools reach the pane through these two: main can
    // drive a guest, but only the renderer can create one.
    const removeBrowserOpenListener = window.api.onBrowserOpenPane(({ sessionId, url }) => {
      // Main vetted this url — scheme, and for `file:` that the path is inside
      // the session's root. The renderer has no filesystem to re-check the
      // second half with, so re-normalizing here would drop it.
      useAppStore.getState().openBrowserPane(sessionId, url, { trusted: true })
    })

    const removeDeviceOpenListener = window.api.onDeviceOpenPane(({ sessionId, udid, name }) => {
      useAppStore.getState().openDevicePane(sessionId, { udid, name })
    })

    const removeBrowserTabListener = window.api.onBrowserTabCommand((cmd) => {
      const store = useAppStore.getState()
      if (cmd.action === 'add') store.addBrowserTab(cmd.sessionId, cmd.url, { trusted: true })
      else if (cmd.action === 'close') store.closeBrowserTab(cmd.sessionId, cmd.index ?? -1)
      else store.setActiveBrowserTab(cmd.sessionId, cmd.index ?? -1)
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
          if (store.workflowExecutions.has(run.runId)) continue
          store.setWorkflowExecution(run.runId, run)
          hydrated.push(run)
        }
        rescheduleWaitingGateTimers(hydrated, store.config?.workflows ?? [])
      })
      .catch((err) => console.error('[App] failed to hydrate waiting gates:', err))

    // Resolve runs the previous renderer left in `running`. The main process
    // keeps headless agents alive past a renderer reload, but the in-memory
    // exit-promise dies — the run wedges. Reconcile against session_events
    // and close out anything that already exited.
    configLoaded
      .then(() => window.api.listRunningWorkflowRuns())
      .then((runs) => {
        const store = useAppStore.getState()
        for (const run of runs) {
          if (!store.workflowExecutions.has(run.runId)) {
            store.setWorkflowExecution(run.runId, run)
          }
        }
        // After the config, so a step that said its failure was survivable is read that way.
        return reconcileRunningExecutions(runs, store.config?.workflows ?? [])
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
      removeReplacedListener?.()
      removeLocalServerListener?.()
      removeBellListener?.()
      removeExitListener()
      removeSessionCreatedListener()
      removeConfigListener()
      removeMenuListener()
      removeSchedulerListener()
      removeWidgetSelectListener()
      removeUpdateListener()
      removeBrowserOpenListener()
      removeDeviceOpenListener()
      removeBrowserTabListener()
      removeSessionUpdatedListener()
      removeHeadlessExitListener()
      removeHeadlessDataListener()
      removeStopRunListener()
      removeGateListener()
      clearInterval(headlessPollInterval)
      clearInterval(pruneInterval)
    }
  }, [])

  return (
    <div
      className="flex h-dvh text-gray-100"
      style={{
        background: 'var(--color-surface-base)',
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
            className={`titlebar-drag shrink-0 border-b border-white/[0.06] relative z-[46] bg-surface-base
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
              {!isMobile && !isSidebarOpen && <AppNavCluster />}
              {!isMobile && mainViewMode === 'sessions' && (
                <>
                  <SessionDock includeMinimized={layoutMode === 'grid'} />
                  <HeadlessBadge align="left" />
                </>
              )}
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
              <FocusedStage />
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
      {isMobile && focusedId && <FocusedStage />}

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
