import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { isServerRunning } from './ws-client'

/**
 * Refuse the tools that read SQLite directly when this machine does not own the
 * data they would read.
 *
 * Tasks, projects, workspaces and workflows bypass the socket and open
 * `~/.vorn/vorn.db` themselves. That was harmless while the only server was this
 * machine's. It is not once the desktop can be pointed at a host: the database is
 * over there, and these tools would keep working against the local file — reading
 * a stale task list and writing to one nobody is serving.
 *
 * That failure is silent, which makes it worse than the socket-backed tools, which
 * simply error when there is nothing to reach. An agent would read the wrong
 * backlog and report confidently on it.
 *
 * The test is whether a local server is running, not what mode the desktop is in.
 * MCP cannot read the desktop's host settings — the credential there is sealed
 * with an OS keychain this process has no access to — but it does not need to.
 * Host mode means nothing wrote a port file, and a database with no server behind
 * it is one nobody is coordinating writes to, whatever the reason.
 */
const REFUSAL =
  'This tool reads Vorn data directly from this machine, and no Vorn server is ' +
  'running here. If Vorn is connected to a server on another machine, its tasks, ' +
  'projects and workflows live there — use the tools that go over the connection, ' +
  'or run Vorn on this machine to work with its own data.'

type ToolHandler = (...args: unknown[]) => Promise<unknown>

/**
 * A stand-in for the server that checks before each call.
 *
 * Wrapping at registration rather than adding a line to every handler: there are
 * two dozen of them across four files, and one forgotten is exactly the silent
 * case this exists to prevent.
 */
export function guardLocalData(server: McpServer): McpServer {
  return new Proxy(server, {
    get(target, prop, receiver) {
      const original = Reflect.get(target, prop, receiver)
      if (prop !== 'tool' || typeof original !== 'function') return original

      return (...args: unknown[]) => {
        const handlerIndex = args.length - 1
        const handler = args[handlerIndex]
        if (typeof handler !== 'function') return original.apply(target, args)

        const guarded = async (...handlerArgs: unknown[]): Promise<unknown> => {
          if (!isServerRunning()) {
            return { content: [{ type: 'text', text: REFUSAL }], isError: true }
          }
          return (handler as ToolHandler)(...handlerArgs)
        }

        return original.apply(target, [...args.slice(0, handlerIndex), guarded])
      }
    }
  })
}
