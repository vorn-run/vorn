import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { AiAgentType } from '@vornrun/shared/types'
import { V } from '../validation'
import {
  dbListProjects,
  dbGetProject,
  dbInsertProject,
  dbUpdateProject,
  dbDeleteProject,
  dbSignalChange
} from '../data-access'

/**
 * Deliberately `AiAgentType`, not `AgentType`.
 *
 * `AgentType` also admits `'shell'`, which is a plain PTY and not something a
 * task or project can be assigned to — the config types have always said so. The
 * list below never contained it either, so the wider annotation only ever
 * described these values incorrectly, and every use of it needed a cast that
 * quietly disagreed with the field being assigned.
 */
const AGENT_TYPES: [AiAgentType, ...AiAgentType[]] = [
  'claude',
  'copilot',
  'codex',
  'opencode',
  'gemini'
]

export function registerProjectTools(server: McpServer): void {
  server.tool(
    'list_projects',
    'List all projects, optionally filtered by workspace',
    {
      workspace_id: V.id.optional().describe('Filter by workspace ID (e.g. "personal")')
    },
    async (args) => {
      let projects = await dbListProjects()
      if (args.workspace_id) {
        projects = projects.filter((p) => (p.workspaceId ?? 'personal') === args.workspace_id)
      }
      return { content: [{ type: 'text', text: JSON.stringify(projects, null, 2) }] }
    }
  )

  server.tool(
    'create_project',
    'Create a new project',
    {
      name: V.name.describe('Project name (unique identifier)'),
      path: V.absolutePath.describe('Absolute path to project directory'),
      preferred_agents: z.array(z.enum(AGENT_TYPES)).optional().describe('Preferred agent types'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon')
    },
    async (args) => {
      if (await dbGetProject(args.name)) {
        return {
          content: [{ type: 'text', text: `Error: project "${args.name}" already exists` }],
          isError: true
        }
      }

      const project = {
        name: args.name,
        path: args.path,
        preferredAgents: (args.preferred_agents as AiAgentType[]) ?? [],
        ...(args.icon && { icon: args.icon }),
        ...(args.icon_color && { iconColor: args.icon_color })
      }

      await dbInsertProject(project)
      dbSignalChange()

      return { content: [{ type: 'text', text: JSON.stringify(project, null, 2) }] }
    }
  )

  server.tool(
    'update_project',
    "Update a project's properties",
    {
      name: V.name.describe('Project name (identifier, cannot be changed)'),
      path: V.absolutePath.optional().describe('New project path'),
      preferred_agents: z.array(z.enum(AGENT_TYPES)).optional().describe('Preferred agent types'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon')
    },
    async (args) => {
      if (!(await dbGetProject(args.name))) {
        return {
          content: [{ type: 'text', text: `Error: project "${args.name}" not found` }],
          isError: true
        }
      }

      const updates: Record<string, unknown> = {}
      if (args.path !== undefined) updates.path = args.path
      if (args.preferred_agents !== undefined)
        updates.preferredAgents = args.preferred_agents as AiAgentType[]
      if (args.icon !== undefined) updates.icon = args.icon
      if (args.icon_color !== undefined) updates.iconColor = args.icon_color

      await dbUpdateProject(args.name, updates)
      dbSignalChange()

      const updated = await dbGetProject(args.name)
      return { content: [{ type: 'text', text: JSON.stringify(updated, null, 2) }] }
    }
  )

  server.tool(
    'delete_project',
    'Delete a project and all its tasks',
    { name: V.name.describe('Project name') },
    async (args) => {
      if (!(await dbGetProject(args.name))) {
        return {
          content: [{ type: 'text', text: `Error: project "${args.name}" not found` }],
          isError: true
        }
      }

      await dbDeleteProject(args.name)
      dbSignalChange()

      return { content: [{ type: 'text', text: `Deleted project: ${args.name}` }] }
    }
  )
}
