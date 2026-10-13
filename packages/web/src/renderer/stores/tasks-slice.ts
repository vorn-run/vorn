import { StateCreator } from 'zustand'
import { TaskConfig, isTerminalTaskStatus } from '../../shared/types'
import { AppStore, TasksSlice } from './types'

export const createTasksSlice: StateCreator<AppStore, [], [], TasksSlice> = (set, get) => ({
  getTasksForProject: (projectName) => {
    const tasks = get().config?.tasks || []
    return tasks.filter((t) => t.projectName === projectName)
  },

  getTaskQueue: (projectName) => {
    const tasks = get().config?.tasks || []
    return tasks
      .filter((t) => t.projectName === projectName && t.status === 'todo')
      .sort((a, b) => a.order - b.order)
  },

  getNextTask: (projectName) => {
    return get().getTaskQueue(projectName)[0]
  },

  addTask: (task) =>
    set((state) => {
      if (!state.config) return {}
      const updated = {
        ...state.config,
        tasks: [...(state.config.tasks || []), task]
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  removeTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const task = (state.config.tasks || []).find((t) => t.id === id)
      if (task?.images?.length) {
        window.api.cleanupTaskImages(id)
      }
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).filter((t) => t.id !== id)
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  updateTask: (id, updates) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const clearArchived = updates.status !== undefined && !isTerminalTaskStatus(updates.status)
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped: TaskConfig = {
            ...t,
            ...updates,
            updatedAt: now,
            ...(clearArchived && { archivedAt: undefined })
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  reorderTask: (id, newOrder) =>
    set((state) => {
      if (!state.config) return {}
      const tasks = [...(state.config.tasks || [])]
      const task = tasks.find((t) => t.id === id)
      if (!task) return {}

      const projectTodos = tasks
        .filter((t) => t.projectName === task.projectName && t.status === 'todo' && t.id !== id)
        .sort((a, b) => a.order - b.order)

      const clamped = Math.max(0, Math.min(newOrder, projectTodos.length))
      projectTodos.splice(clamped, 0, task)

      const reordered = new Map<string, number>()
      projectTodos.forEach((t, i) => reordered.set(t.id, i))

      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: tasks.map((t) =>
          reordered.has(t.id) ? { ...t, order: reordered.get(t.id)!, updatedAt: now } : t
        )
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  startTask: (id, sessionId, agentType, worktreePath) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped = {
            ...t,
            status: 'in_progress' as const,
            assignedSessionId: sessionId,
            assignedAgent: agentType,
            worktreePath: worktreePath || t.worktreePath,
            updatedAt: now,
            archivedAt: undefined
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  completeTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped = {
            ...t,
            status: 'done' as const,
            completedAt: now,
            updatedAt: now,
            assignedSessionId: undefined
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  reviewTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped = {
            ...t,
            status: 'in_review' as const,
            updatedAt: now,
            archivedAt: undefined,
            assignedSessionId: undefined
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  cancelTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped = {
            ...t,
            status: 'cancelled' as const,
            completedAt: now,
            updatedAt: now,
            assignedSessionId: undefined
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  reopenTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: (state.config.tasks || []).map((t) => {
          if (t.id !== id) return t
          const mapped = {
            ...t,
            status: 'todo' as const,
            updatedAt: now,
            completedAt: undefined,
            archivedAt: undefined,
            assignedSessionId: undefined,
            assignedAgent: undefined
          }
          return mapped
        })
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  archiveTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const task = (state.config.tasks || []).find((t) => t.id === id)
      if (!task || !isTerminalTaskStatus(task.status) || task.archivedAt) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: state.config.tasks!.map((t) =>
          t.id === id ? { ...t, archivedAt: now, updatedAt: now } : t
        )
      }
      window.api.saveConfig(updated)
      return { config: updated }
    }),

  unarchiveTask: (id) =>
    set((state) => {
      if (!state.config) return {}
      const task = (state.config.tasks || []).find((t) => t.id === id)
      if (!task || !task.archivedAt) return {}
      const now = new Date().toISOString()
      const updated = {
        ...state.config,
        tasks: state.config.tasks!.map((t) =>
          t.id === id ? { ...t, archivedAt: undefined, updatedAt: now } : t
        )
      }
      window.api.saveConfig(updated)
      return { config: updated }
    })
})
