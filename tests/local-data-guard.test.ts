import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Tasks, projects, workspaces and workflows open this machine's SQLite directly
 * rather than going over the socket. Harmless while the only server was this
 * machine's; not once the desktop can be pointed at a host, where the data lives
 * elsewhere and the local file is stale.
 *
 * The socket-backed tools already error when there is nothing to reach. These
 * would not — they would answer, from the wrong database.
 */

const running = { value: true }
vi.mock('../packages/mcp/src/ws-client', () => ({
  isServerRunning: () => running.value
}))

import { guardLocalData } from '../packages/mcp/src/local-data-guard'

type Registered = { name: string; handler: (...a: unknown[]) => Promise<unknown> }

/** Enough of McpServer to register a tool and call it back. */
function fakeServer() {
  const registered: Registered[] = []
  return {
    registered,
    tool: (...args: unknown[]) => {
      registered.push({
        name: args[0] as string,
        handler: args[args.length - 1] as Registered['handler']
      })
    },
    somethingElse: () => 'untouched'
  }
}

beforeEach(() => {
  running.value = true
})

describe('tools that read this machine directly', () => {
  it('runs normally while a local server is serving that data', async () => {
    const server = fakeServer()
    const inner = vi.fn().mockResolvedValue({ content: [{ type: 'text', text: 'the tasks' }] })

    guardLocalData(server as never).tool('list_tasks', 'desc', inner)
    const result = await server.registered[0].handler({ project: 'vorn' })

    expect(inner).toHaveBeenCalledWith({ project: 'vorn' })
    expect(result).toEqual({ content: [{ type: 'text', text: 'the tasks' }] })
  })

  it('refuses instead of answering from a database nobody is serving', async () => {
    // The silent case: without this it returns the local file's contents, and an
    // agent reads the wrong backlog and reports confidently on it.
    running.value = false
    const server = fakeServer()
    const inner = vi.fn().mockResolvedValue({ content: [{ type: 'text', text: 'stale tasks' }] })

    guardLocalData(server as never).tool('list_tasks', 'desc', inner)
    const result = await server.registered[0].handler({})

    expect(inner).not.toHaveBeenCalled()
    expect(result).toMatchObject({ isError: true })
  })

  it('says where the data actually is', async () => {
    running.value = false
    const server = fakeServer()

    guardLocalData(server as never).tool('list_tasks', 'desc', vi.fn())
    const result = (await server.registered[0].handler({})) as {
      content: Array<{ text: string }>
    }

    expect(result.content[0].text).toMatch(/another machine/)
  })

  it('guards a tool registered with a schema as well as one without', async () => {
    // `server.tool` is variadic — name/desc/handler, or name/desc/schema/handler.
    // Missing the four-argument form would leave most tools unguarded.
    running.value = false
    const server = fakeServer()
    const inner = vi.fn()

    guardLocalData(server as never).tool('create_task', 'desc', { title: {} }, inner)
    await server.registered[0].handler({})

    expect(inner).not.toHaveBeenCalled()
  })

  it('registers under the real name and keeps the schema', () => {
    const server = fakeServer()
    const schema = { title: {} }
    const calls: unknown[][] = []
    const spy = { ...server, tool: (...a: unknown[]) => calls.push(a) }

    guardLocalData(spy as never).tool('create_task', 'desc', schema, vi.fn())

    expect(calls[0][0]).toBe('create_task')
    expect(calls[0][1]).toBe('desc')
    expect(calls[0][2]).toBe(schema)
  })

  it('leaves everything else on the server alone', () => {
    const server = fakeServer()

    expect((guardLocalData(server as never) as unknown as typeof server).somethingElse()).toBe(
      'untouched'
    )
  })
})
