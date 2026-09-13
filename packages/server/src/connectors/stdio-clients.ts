import log from '../logger'

// Children spawned once per key and kept until they exit or are stopped; a connection or an extension's project is the key.

export interface SpawnConfig {
  command: string
  args: string[]
  env: Record<string, string>
  cwd?: string
}

/** A running child: how to stop it, and how to hear that it ended on its own. */
export interface ChildHandle {
  close(): Promise<void>
  onExit(listener: () => void): void
}

export interface ChildCache<Handle, Meta, Config> {
  getOrStart(key: string, spawn: () => Promise<{ config: Config; meta: Meta }>): Promise<Handle>
  stop(key: string): Promise<void>
  stopWhere(matches: (meta: Meta) => boolean): Promise<void>
  stopAll(): Promise<void>
  has(key: string): boolean
  get(key: string): Meta | undefined
  find(matches: (meta: Meta) => boolean): Meta | undefined
  entries(): Meta[]
}

interface Live<Handle, Meta> {
  handle: Handle
  meta: Meta
}

export function createChildCache<Handle extends ChildHandle, Meta, Config = SpawnConfig>(
  label: string,
  open: (config: Config, key: string) => Promise<Handle>
): ChildCache<Handle, Meta, Config> {
  const live = new Map<string, Live<Handle, Meta>>()
  // In-flight startups, so two callers for one key share a child rather than racing two.
  const pending = new Map<string, Promise<Handle>>()

  async function start(
    key: string,
    spawn: () => Promise<{ config: Config; meta: Meta }>
  ): Promise<Handle> {
    const { config, meta } = await spawn()
    const handle = await open(config, key)
    const entry: Live<Handle, Meta> = { handle, meta }
    live.set(key, entry)
    handle.onExit(() => {
      if (live.get(key) !== entry) return
      live.delete(key)
      log.info(`[${label}] ${key} exited, will start again when needed`)
    })
    return handle
  }

  const stop = async (key: string): Promise<void> => {
    const entry = live.get(key)
    if (!entry) return
    live.delete(key)
    try {
      await entry.handle.close()
    } catch (err) {
      log.warn(`[${label}] closing ${key} failed: ${err}`)
    }
  }

  const keysWhere = (matches: (meta: Meta) => boolean): string[] =>
    [...live.entries()].filter(([, entry]) => matches(entry.meta)).map(([key]) => key)

  return {
    getOrStart(key, spawn) {
      const existing = live.get(key)
      if (existing) return Promise.resolve(existing.handle)
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
