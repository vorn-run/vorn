import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { Transport } from '@modelcontextprotocol/sdk/shared/transport.js'
import type { JSONRPCMessage } from '@modelcontextprotocol/sdk/types.js'
import type { VorndStatus } from '@vornrun/shared/types'

/**
 * The agent's side of vornd's MCP server.
 *
 * With the Native server switch on, vornd serves the tools at `/mcp` over
 * Streamable HTTP, and this process only carries messages between the agent's
 * stdio and that endpoint. The tools then run once, in vornd, for every agent,
 * instead of in a Node process per agent.
 *
 * What the TypeScript tools read from their own process -- the working
 * directory and `VORN_SESSION_ID` -- is the agent's, and vornd cannot see it,
 * so every request carries both.
 */

/** How long the server and vornd get to say whether vornd serves MCP. */
const ASK_TIMEOUT_MS = 3_000

export interface RelayDeps {
  /** `server:vornd` on the running server. */
  vorndStatus: () => Promise<VorndStatus>
  fetch: typeof fetch
}

/**
 * vornd's `/mcp`, when the running server's vornd serves MCP itself; null
 * when it does not or cannot be asked, and the TypeScript tools answer.
 */
export async function vorndMcpUrl(deps: RelayDeps): Promise<URL | null> {
  try {
    const status = await deps.vorndStatus()
    if (status.state !== 'on' || !status.nativeServer) return null
    const res = await deps.fetch(`http://127.0.0.1:${status.port}/vornd/health`, {
      signal: AbortSignal.timeout(ASK_TIMEOUT_MS)
    })
    // vornd answers 503 while the server is unreachable, with the same body.
    const health = (await res.json()) as { groups?: Record<string, { mode?: string }> }
    if (health.groups?.mcp?.mode !== 'native') return null
    return new URL(`http://127.0.0.1:${status.port}/mcp`)
  } catch {
    return null
  }
}

/** The headers every request to vornd's `/mcp` carries. */
export function relayHeaders(
  credential: string,
  cwd: string,
  env: NodeJS.ProcessEnv
): Record<string, string> {
  const headers: Record<string, string> = {
    Authorization: `Bearer ${credential}`,
    'Vorn-Cwd': encodeURIComponent(cwd)
  }
  const session = env.VORN_SESSION_ID
  if (session) headers['Vorn-Session-Id'] = session
  return headers
}

function isRequest(message: JSONRPCMessage): message is JSONRPCMessage & { id: string | number } {
  return 'method' in message && 'id' in message
}

/**
 * Carries messages between `local` (the agent's stdio) and vornd at `url`
 * until the agent closes its end. A request vornd cannot be reached for is
 * answered with an error rather than left waiting.
 */
export async function relay(
  local: Transport,
  url: URL,
  headers: Record<string, string>
): Promise<void> {
  const remote = new StreamableHTTPClientTransport(url, { requestInit: { headers } })
  const closed = new Promise<void>((resolve) => {
    local.onclose = () => {
      remote.terminateSession().catch(() => {})
      remote.close().catch(() => {})
      resolve()
    }
  })
  remote.onmessage = (message) => {
    local.send(message).catch(() => {})
  }
  local.onmessage = (message) => {
    remote.send(message).catch((err: unknown) => {
      if (!isRequest(message)) return
      const detail = err instanceof Error ? err.message : String(err)
      local
        .send({
          jsonrpc: '2.0',
          id: message.id,
          error: { code: -32603, message: `vornd's MCP server did not answer: ${detail}` }
        })
        .catch(() => {})
    })
  }
  await remote.start()
  await local.start()
  await closed
}
