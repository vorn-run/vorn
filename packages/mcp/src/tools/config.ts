import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { rpcCall } from '../rpc-client'

export function registerConfigTools(server: McpServer): void {
  server.tool(
    'get_config',
    'Get the full Vorn configuration (projects, tasks, workflows, settings)',
    async () => {
      const config = await rpcCall('config:load')
      return { content: [{ type: 'text', text: JSON.stringify(config, null, 2) }] }
    }
  )
}
