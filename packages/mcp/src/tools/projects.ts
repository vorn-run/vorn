import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { AiAgentType } from '@vornrun/shared/types'
import { V } from '../validation'
import { agentsOff } from './agents'
import {
  dbListProjects,
  dbGetProject,
  dbInsertProject,
  dbUpdateProject,
  dbDeleteProject,
  dbSignalChange
} from '../data-access'

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
      preferred_agents: z.array(V.agentId).optional().describe('Preferred agent types'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon')
    },
    async (args) => {
      const off = await agentsOff(args.preferred_agents ?? [])
      if (off) return off
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
      preferred_agents: z.array(V.agentId).optional().describe('Preferred agent types'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon')
    },
    async (args) => {
      const off = await agentsOff(args.preferred_agents ?? [])
      if (off) return off
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
