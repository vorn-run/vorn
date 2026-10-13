import { StateCreator } from 'zustand'
import { loadView, saveView } from './ui-slice'
import { AppConfig } from '../../shared/types'
import { AppStore, ProjectsSlice } from './types'

/** Build a config patch via `fn`, persist it, and return the new state. */
function patchConfig(
  config: AppConfig | null,
  fn: (cfg: AppConfig) => Partial<AppConfig>
): Partial<AppStore> {
  if (!config) return {}
  const updated = { ...config, ...fn(config) }
  window.api.saveConfig(updated)
  return { config: updated }
}

export const createProjectsSlice: StateCreator<AppStore, [], [], ProjectsSlice> = (set) => ({
  config: null,
  activeProject: loadView().activeProject,
  activeGroupId: loadView().activeGroupId,
  activeWorktreePath: loadView().activeWorktreePath,

  setConfig: (config) =>
    set({
      config,
      rowHeight: config.defaults.rowHeight || 208,
      activeWorkspace: config.defaults.activeWorkspace ?? 'personal'
    }),

  setActiveProject: (name) => {
    saveView({ activeProject: name, activeGroupId: null, activeWorktreePath: null })
    set({ activeProject: name, activeGroupId: null, activeWorktreePath: null })
  },
  setActiveGroup: (id) => {
    saveView({ activeGroupId: id, activeProject: null, activeWorktreePath: null })
    set({ activeGroupId: id, activeProject: null, activeWorktreePath: null })
  },
  setActiveWorktreePath: (path) => {
    saveView({ activeWorktreePath: path })
    set({ activeWorktreePath: path })
  },

  addProject: (project) =>
    set((s) => patchConfig(s.config, (c) => ({ projects: [...c.projects, project] }))),

  removeProject: (name) =>
    set((s) =>
      patchConfig(s.config, (c) => ({ projects: c.projects.filter((p) => p.name !== name) }))
    ),

  updateProject: (originalName, project) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        projects: c.projects.map((p) => (p.name === originalName ? project : p))
      }))
    ),

  addSessionGroup: (group) =>
    set((s) =>
      patchConfig(s.config, (c) => ({ sessionGroups: [...(c.sessionGroups || []), group] }))
    ),

  updateSessionGroup: (id, updates) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        sessionGroups: (c.sessionGroups || []).map((g) => (g.id === id ? { ...g, ...updates } : g))
      }))
    ),

  removeSessionGroup: (id) =>
    set((state) => {
      if (!state.config) return {}
      const clearSelection = state.activeGroupId === id
      const updated = {
        ...state.config,
        sessionGroups: (state.config.sessionGroups || []).filter((g) => g.id !== id)
      }
      window.api.saveConfig(updated)
      // The server owns membership, so the sessions are let go through it.
      for (const [sessionId, t] of state.terminals) {
        if (t.session.groupId === id) void window.api.setSessionGroup(sessionId, null)
      }
      if (clearSelection) saveView({ activeGroupId: null })
      return { config: updated, ...(clearSelection && { activeGroupId: null }) }
    }),

  moveSessionToGroup: (sessionId, groupId) => {
    void window.api.setSessionGroup(sessionId, groupId)
    set((state) => {
      const t = state.terminals.get(sessionId)
      if (!t) return {}
      const terminals = new Map(state.terminals)
      const session = { ...t.session }
      if (groupId) session.groupId = groupId
      else delete session.groupId
      terminals.set(sessionId, { ...t, session })
      return { terminals }
    })
  },

  addWorkflow: (workflow) =>
    set((s) => patchConfig(s.config, (c) => ({ workflows: [...(c.workflows || []), workflow] }))),

  removeWorkflow: (id) =>
    set((s) =>
      patchConfig(s.config, (c) => ({ workflows: (c.workflows || []).filter((w) => w.id !== id) }))
    ),

  updateWorkflow: (id, workflow) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        workflows: (c.workflows || []).map((w) => (w.id === id ? workflow : w))
      }))
    ),

  addRemoteHost: (host) =>
    set((s) => patchConfig(s.config, (c) => ({ remoteHosts: [...(c.remoteHosts || []), host] }))),

  removeRemoteHost: (id) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        remoteHosts: (c.remoteHosts || []).filter((h) => h.id !== id)
      }))
    ),

  updateRemoteHost: (id, host) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        remoteHosts: (c.remoteHosts || []).map((h) => (h.id === id ? host : h))
      }))
    ),

  addWorkspace: (workspace) =>
    set((s) =>
      patchConfig(s.config, (c) => ({ workspaces: [...(c.workspaces || []), workspace] }))
    ),

  removeWorkspace: (id) =>
    set((state) => {
      if (!state.config || id === 'personal') return {}
      const switchToPersonal = state.activeWorkspace === id
      // Move projects and workflows from deleted workspace to 'personal'
      const updated = {
        ...state.config,
        workspaces: (state.config.workspaces || []).filter((ws) => ws.id !== id),
        // A group belongs to one workspace, so it goes with it. Its sessions keep
        // a group_id pointing at nothing, which every reader treats as ungrouped.
        sessionGroups: (state.config.sessionGroups || []).filter((g) => g.workspaceId !== id),
        projects: state.config.projects.map((p) =>
          (p.workspaceId ?? 'personal') === id ? { ...p, workspaceId: 'personal' } : p
        ),
        workflows: (state.config.workflows || []).map((w) =>
          (w.workspaceId ?? 'personal') === id ? { ...w, workspaceId: 'personal' } : w
        ),
        defaults: {
          ...state.config.defaults,
          ...(switchToPersonal && { activeWorkspace: 'personal' })
        }
      }
      window.api.saveConfig(updated)
      return { config: updated, ...(switchToPersonal && { activeWorkspace: 'personal' }) }
    }),

  updateWorkspace: (id, updates) =>
    set((s) =>
      patchConfig(s.config, (c) => ({
        workspaces: (c.workspaces || []).map((ws) => (ws.id === id ? { ...ws, ...updates } : ws))
      }))
    )
})
