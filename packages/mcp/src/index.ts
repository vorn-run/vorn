#!/usr/bin/env node

declare const __MCP_VERSION__: string | undefined

import { createRequire } from 'node:module'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { configManager } from '@vornrun/server/config-manager'
import { readLocalToken, rpcCall } from '@vornrun/server/rpc-client'
import { relay, relayHeaders, vorndMcpUrl } from './relay'
import { createMcpServer } from './server'

// Redirect all console methods to stderr (stdout is reserved for JSON-RPC)
const _origError = console.error
console.log = (...args: unknown[]) => _origError('[mcp]', ...args)
console.info = (...args: unknown[]) => _origError('[mcp]', ...args)
console.debug = (...args: unknown[]) => _origError('[mcp:debug]', ...args)
console.warn = (...args: unknown[]) => _origError('[mcp:warn]', ...args)
console.error = (...args: unknown[]) => _origError('[mcp:error]', ...args)

async function main() {
  // When vornd serves the tools, this process only relays to it.
  const url = await vorndMcpUrl({ vorndStatus: () => rpcCall('server:vornd'), fetch })
  if (url) {
    const transport = new StdioServerTransport()
    process.stdin.once('end', () => void transport.close())
    await relay(transport, url, relayHeaders(readLocalToken(), process.cwd(), process.env))
    process.exit(0)
  }

  // Initialize database only (lightweight — no PTY, no scheduler)
  configManager.init()

  const version =
    typeof __MCP_VERSION__ !== 'undefined'
      ? __MCP_VERSION__
      : (createRequire(import.meta.url)('../package.json') as { version: string }).version
  const server = createMcpServer(version)
  const transport = new StdioServerTransport()
  await server.connect(transport)

  // Graceful shutdown
  transport.onclose = () => {
    configManager.close()
    process.exit(0)
  }
}

main().catch((err) => {
  console.error('Failed to start MCP server:', err)
  process.exit(1)
})
