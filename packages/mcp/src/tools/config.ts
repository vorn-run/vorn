import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { rpcCall } from '../rpc-client'
import { dbListTasks, dbListWorkflows } from '../data-access'

/**
 * The configuration, with its tasks and workflows read beside it and put
 * where the configuration once carried them: the server keeps those apart now.
 */
async function wholeConfig(): Promise<Record<string, unknown>> {
  const config = await rpcCall<Record<string, unknown>>('config:load')
  const [workflows, tasks] = [await dbListWorkflows(), await dbListTasks()]
  const whole: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(config)) {
    whole[key] = value
    if (key === 'agentCommands') whole.workflows = workflows
    if (key === 'remoteHosts') whole.tasks = tasks
  }
  return whole
}

export function registerConfigTools(server: McpServer): void {
  server.tool(
    'get_config',
    'Get the full Vorn configuration (projects, tasks, workflows, settings)',
    async () => {
      const config = await wholeConfig()
      return { content: [{ type: 'text', text: JSON.stringify(config, null, 2) }] }
    }
  )
}
