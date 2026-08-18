import crypto from 'node:crypto'
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { WorkspaceConfig } from '@vornrun/shared/types'
import { V } from '../validation'
import {
  dbListWorkspaces,
  dbInsertWorkspace,
  dbUpdateWorkspace,
  dbDeleteWorkspace,
  dbSignalChange
} from '../data-access'

export function registerWorkspaceTools(server: McpServer): void {
  server.tool('list_workspaces', 'List all workspaces', async () => {
    const workspaces = await dbListWorkspaces()
    return { content: [{ type: 'text', text: JSON.stringify(workspaces, null, 2) }] }
  })

  server.tool(
    'create_workspace',
    'Create a new workspace for organizing projects',
    {
      name: V.name.describe('Workspace name'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon')
    },
    async (args) => {
      const existing = await dbListWorkspaces()
      const maxOrder = existing.reduce((max, w) => Math.max(max, w.order), 0)

      const workspace: WorkspaceConfig = {
        id: crypto.randomUUID(),
        name: args.name,
        order: maxOrder + 1,
        ...(args.icon && { icon: args.icon }),
        ...(args.icon_color && { iconColor: args.icon_color })
      }

      await dbInsertWorkspace(workspace)
      dbSignalChange()

      return { content: [{ type: 'text', text: JSON.stringify(workspace, null, 2) }] }
    }
  )

  server.tool(
    'update_workspace',
    "Update a workspace's properties",
    {
      id: V.id.describe('Workspace ID'),
      name: V.name.optional().describe('New name'),
      icon: V.shortText.optional().describe('Lucide icon name'),
      icon_color: V.hexColor.optional().describe('Hex color for icon'),
      order: z.number().int().min(0).optional().describe('Sort order')
    },
    async (args) => {
      const existing = await dbListWorkspaces()
      if (!existing.find((w) => w.id === args.id)) {
        return {
          content: [{ type: 'text', text: `Error: workspace "${args.id}" not found` }],
          isError: true
        }
      }

      const updates: Partial<WorkspaceConfig> = {}
      if (args.name !== undefined) updates.name = args.name
      if (args.icon !== undefined) updates.icon = args.icon
      if (args.icon_color !== undefined) updates.iconColor = args.icon_color
      if (args.order !== undefined) updates.order = args.order

      await dbUpdateWorkspace(args.id, updates)
      dbSignalChange()

      const updated = (await dbListWorkspaces()).find((w) => w.id === args.id)
      return { content: [{ type: 'text', text: JSON.stringify(updated, null, 2) }] }
    }
  )

  server.tool(
    'delete_workspace',
    'Delete a workspace',
    { id: V.id.describe('Workspace ID') },
    async (args) => {
      if (args.id === 'personal') {
        return {
          content: [{ type: 'text', text: 'Error: cannot delete the default workspace' }],
          isError: true
        }
      }
      const existing = await dbListWorkspaces()
      const workspace = existing.find((w) => w.id === args.id)
      if (!workspace) {
        return {
          content: [{ type: 'text', text: `Error: workspace "${args.id}" not found` }],
          isError: true
        }
      }
      await dbDeleteWorkspace(args.id)
      dbSignalChange()
      return { content: [{ type: 'text', text: `Deleted workspace: ${workspace.name}` }] }
    }
  )
}
