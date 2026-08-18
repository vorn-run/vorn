import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { registerTaskTools } from './tools/tasks'
import { registerProjectTools } from './tools/projects'
import { registerSessionTools } from './tools/sessions'
import { registerWorkflowTools } from './tools/workflows'
import { registerConfigTools } from './tools/config'
import { registerWorkspaceTools } from './tools/workspaces'
import { registerConnectorTools } from './tools/connectors'
import { registerBrowserTools } from './tools/browser'
import { registerDeviceTools } from './tools/device'
import { guardLocalData } from './local-data-guard'

export function createMcpServer(version: string): McpServer {
  const server = new McpServer({ name: 'vorn', version }, { capabilities: { tools: {} } })

  registerConfigTools(server)
  registerSessionTools(server)
  registerConnectorTools(server)
  registerBrowserTools(server)
  registerDeviceTools(server)

  // These four open this machine's SQLite themselves rather than going over the
  // socket, so they only mean anything when this machine is the one serving that
  // data. Guarded rather than trusted: pointed at a host they would keep answering
  // from the local file, and a wrong task list returned confidently is worse than
  // an error. See local-data-guard.ts.
  const localOnly = guardLocalData(server)
  registerProjectTools(localOnly)
  registerTaskTools(localOnly)
  registerWorkflowTools(localOnly)
  registerWorkspaceTools(localOnly)

  return server
}
