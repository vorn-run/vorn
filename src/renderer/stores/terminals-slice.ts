import { StateCreator } from 'zustand'
import { AppStore, TerminalsSlice, TerminalState } from './types'

export const createTerminalsSlice: StateCreator<AppStore, [], [], TerminalsSlice> = (set) => ({
  terminals: new Map(),

  addTerminal: (session) =>
    set((state) => {
      const next = new Map(state.terminals)
      next.set(session.id, {
        id: session.id,
        session,
        status: session.status,
        lastOutputTimestamp: Date.now()
      })
      const order = state.terminalOrder.includes(session.id)
        ? state.terminalOrder
        : [...state.terminalOrder, session.id]
      window.api.notifyWidgetStatus()
      return { terminals: next, terminalOrder: order }
    }),

  removeTerminal: (id) =>
    set((state) => {
      const next = new Map(state.terminals)
      next.delete(id)
      const order = state.terminalOrder.filter((tid) => tid !== id)
      const minimized = new Set(state.minimizedTerminals)
      minimized.delete(id)
      const gitDiffStats = new Map(state.gitDiffStats)
      gitDiffStats.delete(id)
      window.api.notifyWidgetStatus()
      const extra = state.diffSidebarTerminalId === id ? { diffSidebarTerminalId: null } : {}
      return {
        terminals: next,
        terminalOrder: order,
        minimizedTerminals: minimized,
        gitDiffStats,
        ...extra
      }
    }),

  updateStatus: (id, status) =>
    set((state) => {
      const next = new Map(state.terminals)
      const term = next.get(id)
      if (term) next.set(id, { ...term, status })
      window.api.notifyWidgetStatus()
      return { terminals: next }
    }),

  updateLastOutput: (id, timestamp) =>
    set((state) => {
      const next = new Map(state.terminals)
      const term = next.get(id)
      if (term) next.set(id, { ...term, lastOutputTimestamp: timestamp })
      return { terminals: next }
    }),

  renameTerminal: (id, displayName) =>
    set((state) => {
      const term = state.terminals.get(id)
      if (!term || term.session.displayName === displayName) return state
      const next = new Map(state.terminals)
      next.set(id, { ...term, session: { ...term.session, displayName } })
      window.api.renameSession(id, displayName)
      return { terminals: next }
    }),

  updateSessionBranch: (id, branch) =>
    set((state) => {
      const term = state.terminals.get(id)
      if (!term || term.session.branch === branch) {
        return state
      }
      const next = new Map(state.terminals)
      next.set(id, { ...term, session: { ...term.session, branch } })
      return { terminals: next }
    }),

  updateSessionCwd: (id, shellCwd) =>
    set((state) => {
      const term = state.terminals.get(id)
      if (!term || term.session.shellCwd === shellCwd) return state
      const next = new Map(state.terminals)
      next.set(id, { ...term, session: { ...term.session, shellCwd } })
      return { terminals: next }
    }),

  setBranchForCwd: (cwd, branch) =>
    set((state) => {
      let next: Map<string, TerminalState> | null = null
      for (const [id, term] of state.terminals) {
        const sessionCwd = term.session.worktreePath ?? term.session.projectPath
        if (sessionCwd !== cwd || term.session.branch === branch) continue
        if (!next) next = new Map(state.terminals)
        next.set(id, { ...term, session: { ...term.session, branch } })
      }
      return next ? { terminals: next } : state
    }),

  updateSessionWorktree: (id, updates) =>
    set((state) => {
      const term = state.terminals.get(id)
      if (!term) return state
      const s = term.session
      const changed =
        (updates.worktreePath !== undefined && updates.worktreePath !== s.worktreePath) ||
        (updates.worktreeName !== undefined && updates.worktreeName !== s.worktreeName)
      if (!changed) return state
      const next = new Map(state.terminals)
      next.set(id, { ...term, session: { ...s, ...updates } })
      return { terminals: next }
    }),

  // Headless agent tracking
  headlessSessions: [],
  headlessLastOutput: new Map(),
  headlessDismissed: new Set(),

  setHeadlessSessions: (sessions) =>
    set((state) => {
      // Rebuild from server list, preserving local-only fields from existing entries
      const dismissed = state.headlessDismissed
      const serverIds = new Set(sessions.map((s) => s.id))
      const existing = new Map(state.headlessSessions.map((s) => [s.id, s]))
      const next: typeof state.headlessSessions = []

      for (const s of sessions) {
        if (dismissed.has(s.id)) continue
        const prev = existing.get(s.id)
        if (prev) {
          next.push({ ...prev, status: s.status, exitCode: s.exitCode, endedAt: s.endedAt })
        } else {
          next.push(s)
        }
      }

      // Keep local sessions not on the server: running ones (just created, not synced yet)
      // AND recently exited ones (server cleans up after 30s, but renderer retains per TTL)
      for (const s of state.headlessSessions) {
        if (!serverIds.has(s.id) && !dismissed.has(s.id)) {
          if (s.status === 'running' || (s.status === 'exited' && s.endedAt)) {
            next.push(s)
          }
        }
      }

      // Clean up output entries for sessions no longer in the retained list
      const nextIds = new Set(next.map((s) => s.id))
      const nextOutput = new Map(state.headlessLastOutput)
      for (const id of nextOutput.keys()) {
        if (!nextIds.has(id)) nextOutput.delete(id)
      }

      return { headlessSessions: next, headlessLastOutput: nextOutput }
    }),

  addHeadlessSession: (session) =>
    set((state) => {
      if (state.headlessSessions.some((s) => s.id === session.id)) return state
      return { headlessSessions: [...state.headlessSessions, session] }
    }),

  updateHeadlessSession: (id, updates) =>
    set((state) => ({
      headlessSessions: state.headlessSessions.map((s) => (s.id === id ? { ...s, ...updates } : s))
    })),

  dismissHeadlessSession: (id) =>
    set((state) => {
      const dismissed = new Set(state.headlessDismissed)
      dismissed.add(id)
      const lastOutput = new Map(state.headlessLastOutput)
      lastOutput.delete(id)
      return {
        headlessSessions: state.headlessSessions.filter((s) => s.id !== id),
        headlessDismissed: dismissed,
        headlessLastOutput: lastOutput
      }
    }),

  pruneExitedHeadless: (retentionMs) =>
    set((state) => {
      const now = Date.now()
      const pruned = new Set<string>()
      const remaining = state.headlessSessions.filter((s) => {
        const keep = s.status === 'running' || !s.endedAt || now - s.endedAt < retentionMs
        if (!keep) pruned.add(s.id)
        return keep
      })
      if (pruned.size === 0) return state
      // Clean up lastOutput and dismissed for pruned sessions
      const lastOutput = new Map(state.headlessLastOutput)
      const dismissed = new Set(state.headlessDismissed)
      for (const id of pruned) {
        lastOutput.delete(id)
        dismissed.delete(id)
      }
      return {
        headlessSessions: remaining,
        headlessLastOutput: lastOutput,
        headlessDismissed: dismissed
      }
    }),

  setHeadlessLastOutput: (id, line) =>
    set((state) => {
      const next = new Map(state.headlessLastOutput)
      next.set(id, line)
      return { headlessLastOutput: next }
    })
})
