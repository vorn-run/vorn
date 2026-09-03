/**
 * In-memory cache of live MCP stdio clients, one per connection.
 *
 * Each MCP connection points at an external MCP server (npx …, node …, etc.).
 * We spawn the child process lazily on first use and keep it alive for the
 * lifetime of the server process, so tool invocations don't pay a startup
 * cost per call. The map is keyed by `connectionId` so `connection:delete`
 * can terminate the right process.
 *
 * Secret env values flow in via the usual decrypted-creds path: the server
 * never sees the encrypted ciphertext, only the plaintext the main process
 * pushes via `credentials:setDecrypted`.
 */
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import {
  SDK_FILTER_KEYS,
  connectionConnectorId,
  type SourceConnection
} from '@vornrun/shared/types'
import { dbListSourceConnections } from '../database'
import { getDecryptedCreds } from './decrypted-creds'
import { localLaunchSpec } from './catalog'
import { installedLaunch } from './packs'
import { borrowedSecrets } from './auth-rung'
import { mightBorrow, resolveBorrow } from './connector-auth'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

interface LiveClient {
  client: Client
  transport: StdioClientTransport
}

const clients = new Map<string, LiveClient>()
// In-flight startups, so two concurrent `getOrStartClient` calls for the
// same connection share one spawn instead of racing two children.
const pending = new Map<string, Promise<Client>>()

function tryParseJson<T>(raw: unknown, guard: (v: unknown) => v is T, fallback: T): T {
  if (typeof raw !== 'string' || raw === '') return fallback
  try {
    const parsed: unknown = JSON.parse(raw)
    return guard(parsed) ? parsed : fallback
  } catch {
    return fallback
  }
}

const isStringMap = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v)

function parseJsonObject(raw: unknown): Record<string, string> {
  const obj = tryParseJson<Record<string, unknown>>(raw, isStringMap, {})
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(obj)) out[k] = String(v)
  return out
}

function parseJsonArray(raw: unknown): string[] {
  const arr = tryParseJson<unknown[]>(raw, (v): v is unknown[] => Array.isArray(v), [])
  return arr.map((v) => String(v))
}

/** Before the database has resolved a data directory there is nowhere to look. */
function installedPackLaunch(id: string): { command: string; args: string[] } | undefined {
  try {
    return installedLaunch(id)
  } catch {
    return undefined
  }
}

/** Checkout, then installed pack, then stored command; a pack must beat stale args. */
export function resolveLaunch(conn: SourceConnection): { command: string; args: string[] } {
  const sdkId = String(conn.filters[SDK_FILTER_KEYS.connectorId] ?? '').trim()
  if (sdkId) {
    const resolved = localLaunchSpec(sdkId) ?? installedPackLaunch(sdkId)
    if (resolved) return resolved
  }
  const command = String(conn.filters.command ?? '').trim()
  if (!command) throw new Error('MCP connection is missing a command')
  return { command, args: parseJsonArray(conn.filters.args) }
}

interface SpawnConfig {
  command: string
  args: string[]
  env: Record<string, string>
}

/** Everything about a spawn that needs nothing asked of another tool. */
function spawnBase(conn: SourceConnection): SpawnConfig {
  const { command, args } = resolveLaunch(conn)
  const env = parseJsonObject(conn.filters.env)
  // Decrypted secret env (pushed from main via safeStorage) overrides plain env.
  const decrypted = getDecryptedCreds(conn.id) ?? {}
  const secretEnv = parseJsonObject(decrypted.secretEnv)
  return { command, args, env: { ...env, ...secretEnv } }
}

/** The connector a connection runs, when it names a packaged one. */
function sdkIdOf(conn: SourceConnection): string {
  return String(conn.filters[SDK_FILTER_KEYS.connectorId] ?? '').trim()
}

/**
 * What a connector's child is started with, including anything borrowed.
 *
 * A borrowed token is fetched fresh here rather than written to the connection,
 * so signing out of the tool takes effect the next time the child starts. It
 * sits lowest: a value someone entered by hand is a deliberate override of what
 * the signed-in tool would have handed over.
 */
export async function buildSpawnConfig(conn: SourceConnection): Promise<SpawnConfig> {
  const base = spawnBase(conn)
  const borrow = await resolveBorrow(sdkIdOf(conn))
  if (!borrow) return base
  const borrowed = await borrowedSecrets(borrow.auth, borrow.declared)
  return { ...base, env: { ...borrowed, ...base.env } }
}

export async function getOrStartClient(conn: SourceConnection): Promise<Client> {
  const existing = clients.get(conn.id)
  if (existing) return existing.client
  const inFlight = pending.get(conn.id)
  if (inFlight) return inFlight

  // Recorded before anything can suspend, so a second caller in the same tick
  // joins this startup rather than beginning a second one. Nothing may be
  // awaited above this line.
  const startup = startClient(conn).finally(() => {
    pending.delete(conn.id)
  })
  pending.set(conn.id, startup)
  return startup
}

async function startClient(conn: SourceConnection): Promise<Client> {
  // Only a connector that borrows a login waits on one: everything else is
  // spawned in this tick, which is what keeps one child per connection when
  // two callers arrive together. Both routes assemble through the same pair of
  // helpers, so neither can drift from what a caller of buildSpawnConfig sees.
  const { command, args, env } = mightBorrow(sdkIdOf(conn))
    ? await buildSpawnConfig(conn)
    : spawnBase(conn)
  // Inherit PATH and friends from the parent via getSafeEnv() — same
  // sanitization the rest of the server uses for child processes — so
  // GH/Linear/NPM tokens etc. don't leak into arbitrary MCP servers
  // the user adds. Explicit per-connection env still wins over the base.
  const transport = new StdioClientTransport({
    command,
    args,
    env: { ...getSafeEnv(), ...env }
  })

  const client = new Client({ name: 'vorn', version: '0.1.0' }, { capabilities: {} })

  try {
    await client.connect(transport)
  } catch (err) {
    // Transport already spawned the child; close it so we don't leak.
    try {
      await transport.close()
    } catch {
      /* ignore */
    }
    throw err
  }

  const live: LiveClient = { client, transport }
  clients.set(conn.id, live)

  // If the child exits, drop it from the cache so the next call respawns.
  transport.onclose = () => {
    const current = clients.get(conn.id)
    if (current === live) {
      clients.delete(conn.id)
      log.info(`[mcp-clients] ${conn.id} transport closed, will respawn on next call`)
    }
  }
  transport.onerror = (err) => {
    log.warn(`[mcp-clients] ${conn.id} transport error: ${err}`)
  }

  return client
}

export async function stopClient(connectionId: string): Promise<void> {
  const live = clients.get(connectionId)
  if (!live) return
  clients.delete(connectionId)
  try {
    await live.client.close()
  } catch (err) {
    log.warn(`[mcp-clients] ${connectionId} close failed: ${err}`)
  }
}

export async function stopAllClients(): Promise<void> {
  const ids = [...clients.keys()]
  await Promise.allSettled(ids.map(stopClient))
}

export function hasClient(connectionId: string): boolean {
  return clients.has(connectionId)
}

/** Which connections a pack change affects, which for a package is not by `connectorId`. */
export function connectionIdsForConnector(connectorId: string): string[] {
  return dbListSourceConnections()
    .filter((conn) => connectionConnectorId(conn) === connectorId)
    .map((conn) => conn.id)
}

/** A child started before a pack change keeps running the old files until stopped. */
export async function stopClientsForConnector(connectorId: string): Promise<void> {
  await Promise.allSettled(connectionIdsForConnector(connectorId).map(stopClient))
}
