import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import type { ChildHandle, SpawnConfig } from './stdio-clients'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

/** An MCP server's client, with the child it talks to. */
export interface McpChild extends ChildHandle {
  client: Client
}

export async function openMcpChild(config: SpawnConfig, key: string): Promise<McpChild> {
  // The same sanitized base every child gets; what a caller names still wins.
  const transport = new StdioClientTransport({
    command: config.command,
    args: config.args,
    env: { ...getSafeEnv(), ...config.env }
  })
  const client = new Client({ name: 'vorn', version: '0.1.0' }, { capabilities: {} })
  try {
    await client.connect(transport)
  } catch (err) {
    try {
      await transport.close()
    } catch {
      /* the child is going away either way */
    }
    throw err
  }

  const listeners: Array<() => void> = []
  let exited = false
  transport.onclose = () => {
    exited = true
    for (const listener of listeners) listener()
  }
  transport.onerror = (err) => log.warn(`[mcp-clients] ${key}: ${err}`)
  return {
    client,
    close: () => client.close(),
    onExit(listener) {
      if (exited) queueMicrotask(listener)
      else listeners.push(listener)
    }
  }
}
