import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

/**
 * Children spawned once per key and kept until they exit or are stopped.
 *
 * A connector keys on the connection it polls and an extension on the project
 * it answers about, but the machinery either one needs is the same: share a
 * startup that two callers ask for at once, sanitize the environment, close the
 * transport when connecting fails, and forget a child that exits so the next
 * call spawns a fresh one. That is this, and the two differ only in the key and
 * in what they put in the environment.
 */

export interface SpawnConfig {
  command: string
  args: string[]
  env: Record<string, string>
  cwd?: string
}

interface LiveClient<Meta> {
  client: Client
  transport: StdioClientTransport
  meta: Meta
}

export interface StdioClientCache<Meta> {
  getOrStart(
    key: string,
    spawn: () => Promise<{ config: SpawnConfig; meta: Meta }>
  ): Promise<Client>
  stop(key: string): Promise<void>
  stopWhere(matches: (meta: Meta) => boolean): Promise<void>
  stopAll(): Promise<void>
  has(key: string): boolean
  get(key: string): Meta | undefined
  find(matches: (meta: Meta) => boolean): Meta | undefined
  entries(): Meta[]
}

export function createStdioClientCache<Meta>(label: string): StdioClientCache<Meta> {
  const live = new Map<string, LiveClient<Meta>>()
  // In-flight startups, so two callers for one key share a child rather than racing two.
  const pending = new Map<string, Promise<Client>>()

  async function start(
    key: string,
    spawn: () => Promise<{ config: SpawnConfig; meta: Meta }>
  ): Promise<Client> {
    const { config, meta } = await spawn()
    // The same sanitized base every child gets; what a caller names still wins.
    const transport = new StdioClientTransport({
      command: config.command,
      args: config.args,
      ...(config.cwd !== undefined && { cwd: config.cwd }),
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

    const entry: LiveClient<Meta> = { client, transport, meta }
    live.set(key, entry)
    transport.onclose = () => {
      if (live.get(key) === entry) {
        live.delete(key)
        log.info(`[${label}] ${key} exited, will start again when needed`)
      }
    }
    transport.onerror = (err) => {
      log.warn(`[${label}] ${key}: ${err}`)
    }
    return client
  }

  const stop = async (key: string): Promise<void> => {
    const entry = live.get(key)
    if (!entry) return
    live.delete(key)
    try {
      await entry.client.close()
    } catch (err) {
      log.warn(`[${label}] closing ${key} failed: ${err}`)
    }
  }

  const keysWhere = (matches: (meta: Meta) => boolean): string[] =>
    [...live.entries()].filter(([, entry]) => matches(entry.meta)).map(([key]) => key)

  return {
    getOrStart(key, spawn) {
      const existing = live.get(key)
      if (existing) return Promise.resolve(existing.client)
      const inFlight = pending.get(key)
      if (inFlight) return inFlight
      // Recorded before anything can suspend, so a second caller joins this startup.
      const startup = start(key, spawn).finally(() => {
        pending.delete(key)
      })
      pending.set(key, startup)
      return startup
    },
    stop,
    async stopWhere(matches) {
      await Promise.allSettled(keysWhere(matches).map(stop))
    },
    async stopAll() {
      await Promise.allSettled([...live.keys()].map(stop))
    },
    has: (key) => live.has(key),
    get: (key) => live.get(key)?.meta,
    find: (matches) => [...live.values()].map((entry) => entry.meta).find(matches),
    entries: () => [...live.values()].map((entry) => entry.meta)
  }
}
