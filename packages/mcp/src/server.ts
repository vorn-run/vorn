import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { registerTaskTools } from './tools/tasks'
import { registerProjectTools } from './tools/projects'
import { registerSessionTools } from './tools/sessions'
import { registerWorkflowTools } from './tools/workflows'
import { registerDescribeNodesTool } from './tools/describe-nodes'
import { registerConfigTools } from './tools/config'
import { registerWorkspaceTools } from './tools/workspaces'
import { registerConnectorTools } from './tools/connectors'
import { registerBrowserTools } from './tools/browser'
import { registerDeviceTools } from './tools/device'
import { registerArtifactTools } from './tools/artifacts'

export function createMcpServer(version: string): McpServer {
  const server = new McpServer({ name: 'vorn', version }, { capabilities: { tools: {} } })

  registerConfigTools(server)
  registerSessionTools(server)
  registerConnectorTools(server)
  registerBrowserTools(server)
  registerDeviceTools(server)
  registerArtifactTools(server)

  // These four used to open this machine's SQLite directly and were guarded so a
  // host-mode desktop could not read a stale local file. They go over the socket
  // now, so they reach whichever server MCP is talking to and need no guard.
  registerProjectTools(server)
  registerTaskTools(server)
  registerWorkflowTools(server)
  registerDescribeNodesTool(server)
  registerWorkspaceTools(server)

  return server
}
