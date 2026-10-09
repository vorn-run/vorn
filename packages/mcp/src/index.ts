#!/usr/bin/env node

declare const __MCP_VERSION__: string | undefined

import { createRequire } from 'node:module'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { readLocalToken, rpcCall } from './rpc-client'
import { relay, relayHeaders, vorndMcpUrl, type Upstream } from './relay'
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
  // Asked again after a restart: the server re-reads its port, the credential is re-read here.
  const locate = async (): Promise<Upstream | null> => {
    const url = await vorndMcpUrl({ vorndStatus: () => rpcCall('server:vornd'), fetch })
    return url && { url, headers: relayHeaders(readLocalToken(), process.cwd(), process.env) }
  }
  const upstream = await locate()
  if (upstream) {
    const transport = new StdioServerTransport()
    process.stdin.once('end', () => void transport.close())
    await relay(transport, upstream, { locate, fetch })
    process.exit(0)
  }

  const version =
    typeof __MCP_VERSION__ !== 'undefined'
      ? __MCP_VERSION__
      : (createRequire(import.meta.url)('../package.json') as { version: string }).version
  const server = createMcpServer(version)
  const transport = new StdioServerTransport()
  await server.connect(transport)

  transport.onclose = () => process.exit(0)
}

main().catch((err) => {
  console.error('Failed to start MCP server:', err)
  process.exit(1)
})
