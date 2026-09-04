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
  activeWorktreePath: loadView().activeWorktreePath,

  setConfig: (config) =>
    set({
      config,
      rowHeight: config.defaults.rowHeight || 208,
      activeWorkspace: config.defaults.activeWorkspace ?? 'personal'
    }),

  setActiveProject: (name) => {
    saveView({ activeProject: name, activeWorktreePath: null })
    set({ activeProject: name, activeWorktreePath: null })
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
