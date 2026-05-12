import crypto from 'node:crypto'
import path from 'node:path'
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { isTerminalTaskStatus } from '@vornrun/shared/types'
import type { TaskConfig, TaskStatus, AgentType } from '@vornrun/shared/types'
import { V } from '../validation'
import {
  dbListTasks,
  dbGetTask,
  dbInsertTask,
  dbUpdateTask,
  dbDeleteTask,
  dbGetMaxTaskOrder,
  dbGetProject,
  dbListProjects,
  dbSignalChange
} from '@vornrun/server/database'
import type { ProjectConfig } from '@vornrun/shared/types'

const TASK_STATUSES: TaskStatus[] = ['todo', 'in_progress', 'in_review', 'done', 'cancelled']
const AGENT_TYPES: [AgentType, ...AgentType[]] = [
  'claude',
  'copilot',
  'codex',
  'opencode',
  'gemini'
]

export function registerTaskTools(server: McpServer): void {
  server.tool(
    'list_tasks',
    'List tasks, optionally filtered by project, status, assigned agent, or workspace. Archived tasks are excluded by default; pass include_archived=true to include them.',
    {
      project_name: V.name.optional().describe('Filter by project name'),
      status: z
        .enum(TASK_STATUSES as [string, ...string[]])
        .optional()
        .describe('Filter by status'),
      assigned_agent: z.enum(AGENT_TYPES).optional().describe('Filter by assigned agent type'),
      workspace_id: V.id
        .optional()
        .describe('Filter by workspace ID (returns tasks from all projects in that workspace)'),
      include_archived: z
        .boolean()
        .optional()
        .describe('Include archived tasks in the result (default: false)')
    },
    async (args) => {
      let tasks = dbListTasks(args.project_name, args.status)

      if (!args.include_archived) {
        tasks = tasks.filter((t) => !t.archivedAt)
      }

      if (args.workspace_id) {
        const projects = dbListProjects()
        const wsProjectNames = new Set(
          projects
            .filter((p: ProjectConfig) => (p.workspaceId ?? 'personal') === args.workspace_id)
            .map((p: ProjectConfig) => p.name)
        )
        tasks = tasks.filter((t) => wsProjectNames.has(t.projectName))
      }

      if (args.assigned_agent) {
        tasks = tasks.filter((t) => t.assignedAgent === args.assigned_agent)
      }

      return { content: [{ type: 'text', text: JSON.stringify(tasks, null, 2) }] }
    }
  )

  server.tool(
    'create_task',
    'Create a new task in a project',
    {
      project_name: V.name.describe('Project name (must match existing project)'),
      title: V.title.describe('Task title'),
      description: V.description.optional().describe('Task description (markdown)'),
      status: z
        .enum(TASK_STATUSES as [string, ...string[]])
        .optional()
        .describe('Task status (default: todo)'),
      branch: V.shortText.optional().describe('Git branch for this task'),
      use_worktree: z.boolean().optional().describe('Create a git worktree for this task'),
      assigned_agent: z.enum(AGENT_TYPES).optional().describe('Assign to an agent type')
    },
    async (args) => {
      const project = dbGetProject(args.project_name)
      if (!project) {
        return {
          content: [{ type: 'text', text: `Error: project "${args.project_name}" not found` }],
          isError: true
        }
      }

      const maxOrder = dbGetMaxTaskOrder(args.project_name)
      const now = new Date().toISOString()
      const status = (args.status as TaskStatus) ?? 'todo'

      const task: TaskConfig = {
        id: crypto.randomUUID(),
        projectName: args.project_name,
        title: args.title,
        description: args.description ?? '',
        status,
        order: maxOrder + 1,
        createdAt: now,
        updatedAt: now,
        ...(args.branch && { branch: args.branch }),
        ...(args.use_worktree && { useWorktree: args.use_worktree }),
        ...(args.assigned_agent && { assignedAgent: args.assigned_agent as AgentType }),
        ...((status === 'done' || status === 'cancelled') && { completedAt: now })
      }

      dbInsertTask(task)
      dbSignalChange()

      return { content: [{ type: 'text', text: JSON.stringify(task, null, 2) }] }
    }
  )

  server.tool('get_task', 'Get a task by ID', { id: V.id.describe('Task ID') }, async (args) => {
    const task = dbGetTask(args.id)
    if (!task) {
      return {
        content: [{ type: 'text', text: `Error: task "${args.id}" not found` }],
        isError: true
      }
    }
    return { content: [{ type: 'text', text: JSON.stringify(task, null, 2) }] }
  })

  server.tool(
    'update_task',
    "Update a task's properties",
    {
      id: V.id.describe('Task ID'),
      title: V.title.optional().describe('New title'),
      description: V.description.optional().describe('New description'),
      status: z
        .enum(TASK_STATUSES as [string, ...string[]])
        .optional()
        .describe('New status'),
      branch: V.shortText.optional().describe('Git branch'),
      use_worktree: z.boolean().optional().describe('Use git worktree'),
      assigned_agent: z.enum(AGENT_TYPES).optional().describe('Assigned agent type'),
      order: z.number().optional().describe('Queue order')
    },
    async (args) => {
      const task = dbGetTask(args.id)
      if (!task) {
        return {
          content: [{ type: 'text', text: `Error: task "${args.id}" not found` }],
          isError: true
        }
      }

      const updates: Partial<TaskConfig> = { updatedAt: new Date().toISOString() }
      if (args.title !== undefined) updates.title = args.title
      if (args.description !== undefined) updates.description = args.description
      if (args.branch !== undefined) updates.branch = args.branch
      if (args.use_worktree !== undefined) updates.useWorktree = args.use_worktree
      if (args.assigned_agent !== undefined)
        updates.assignedAgent = args.assigned_agent as AgentType
      if (args.order !== undefined) updates.order = args.order

      if (args.status !== undefined) {
        const newStatus = args.status as TaskStatus
        const wasDone = isTerminalTaskStatus(task.status)
        const isDone = isTerminalTaskStatus(newStatus)
        updates.status = newStatus
        if (isDone && !wasDone) updates.completedAt = new Date().toISOString()
        if (!isDone && wasDone) {
          updates.completedAt = undefined
          updates.archivedAt = undefined
        }
      }

      dbUpdateTask(args.id, updates)
      dbSignalChange()

      const updated = dbGetTask(args.id)
      return { content: [{ type: 'text', text: JSON.stringify(updated, null, 2) }] }
    }
  )

  server.tool(
    'delete_task',
    'Delete a task by ID',
    { id: V.id.describe('Task ID') },
    async (args) => {
      const task = dbGetTask(args.id)
      if (!task) {
        return {
          content: [{ type: 'text', text: `Error: task "${args.id}" not found` }],
          isError: true
        }
      }
      dbDeleteTask(args.id)
      dbSignalChange()

      return { content: [{ type: 'text', text: `Deleted task: ${task.title}` }] }
    }
  )

  server.tool(
    'archive_task',
    'Archive a finished task (status must be done or cancelled). Archived tasks are hidden from default views but preserved and restorable.',
    { id: V.id.describe('Task ID') },
    async (args) => {
      const task = dbGetTask(args.id)
      if (!task) {
        return {
          content: [{ type: 'text', text: `Error: task "${args.id}" not found` }],
          isError: true
        }
      }
      if (!isTerminalTaskStatus(task.status)) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: only done or cancelled tasks can be archived (status: ${task.status})`
            }
          ],
          isError: true
        }
      }
      const now = new Date().toISOString()
      dbUpdateTask(args.id, { archivedAt: now, updatedAt: now })
      dbSignalChange()
      const updated = dbGetTask(args.id)
      return { content: [{ type: 'text', text: JSON.stringify(updated, null, 2) }] }
    }
  )

  server.tool(
    'unarchive_task',
    'Restore an archived task so it shows in default views again.',
    { id: V.id.describe('Task ID') },
    async (args) => {
      const task = dbGetTask(args.id)
      if (!task) {
        return {
          content: [{ type: 'text', text: `Error: task "${args.id}" not found` }],
          isError: true
        }
      }
      const now = new Date().toISOString()
      dbUpdateTask(args.id, { archivedAt: undefined, updatedAt: now })
      dbSignalChange()
      const updated = dbGetTask(args.id)
      return { content: [{ type: 'text', text: JSON.stringify(updated, null, 2) }] }
    }
  )

  server.tool(
    'get_my_context',
    'Get your current task and project context. Auto-detects based on your working directory. ' +
      'Call this at the start of a session to understand what you are working on.',
    {
      cwd: V.absolutePath
        .optional()
        .describe(
          'Your current working directory (auto-detected if omitted). ' +
            'Used to match against known projects and task worktrees.'
        ),
      task_id: V.id
        .optional()
        .describe('Specific task ID to get context for (overrides auto-detection)')
    },
    async (args) => {
      // If a specific task ID is provided, return its context directly
      if (args.task_id) {
        const task = dbGetTask(args.task_id)
        if (!task) {
          return {
            content: [{ type: 'text', text: `Error: task "${args.task_id}" not found` }],
            isError: true
          }
        }
        const project = dbGetProject(task.projectName)
        const siblingTasks = dbListTasks(task.projectName)
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  task,
                  project: project ?? undefined,
                  siblingTasks: siblingTasks
                    .filter((t) => t.id !== task.id)
                    .map((t) => ({
                      id: t.id,
                      title: t.title,
                      status: t.status,
                      branch: t.branch
                    }))
                },
                null,
                2
              )
            }
          ]
        }
      }

      // Auto-detect by matching cwd to projects and task worktrees
      const cwd = args.cwd || process.cwd()
      const normalizedCwd = path.resolve(cwd)
      const projects = dbListProjects()

      // Find the best matching project (longest path prefix match)
      let matchedProject = null
      let matchLen = 0
      for (const p of projects) {
        const normalizedPath = path.resolve(p.path)
        if (normalizedCwd.startsWith(normalizedPath) && normalizedPath.length > matchLen) {
          matchedProject = p
          matchLen = normalizedPath.length
        }
      }

      if (!matchedProject) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  message: 'No matching project found for current directory.',
                  cwd: normalizedCwd,
                  hint: 'Use list_projects to see available projects, or pass a task_id directly.'
                },
                null,
                2
              )
            }
          ]
        }
      }

      // Get all tasks for this project
      const projectTasks = dbListTasks(matchedProject.name)

      // Try to find the specific task by matching worktree path or assigned session
      let matchedTask: TaskConfig | null = null

      // 1. Check if cwd matches a task's worktree path
      for (const t of projectTasks) {
        if (t.worktreePath) {
          const normalizedWorktree = path.resolve(t.worktreePath)
          if (normalizedCwd.startsWith(normalizedWorktree)) {
            matchedTask = t
            break
          }
        }
      }

      // 2. If no worktree match, look for an in_progress task assigned to this project
      if (!matchedTask) {
        matchedTask = projectTasks.find((t) => t.status === 'in_progress') ?? null
      }

      const result: Record<string, unknown> = {
        project: {
          name: matchedProject.name,
          path: matchedProject.path,
          preferredAgents: matchedProject.preferredAgents
        },
        cwd: normalizedCwd
      }

      if (matchedTask) {
        result.task = matchedTask
        result.siblingTasks = projectTasks
          .filter((t) => t.id !== matchedTask!.id)
          .map((t) => ({ id: t.id, title: t.title, status: t.status, branch: t.branch }))
      } else {
        result.message = 'No specific task matched. Showing all project tasks.'
        result.tasks = projectTasks.map((t) => ({
          id: t.id,
          title: t.title,
          status: t.status,
          branch: t.branch
        }))
      }

      return { content: [{ type: 'text', text: JSON.stringify(result, null, 2) }] }
    }
  )
}
