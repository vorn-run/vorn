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
import { resolveConnectorAuth } from './connector-auth'
import { createStdioClientCache } from './stdio-clients'

const clients = createStdioClientCache<{ connectionId: string }>('mcp-clients')

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

/** The connector a connection runs, when it names a packaged one. */
function sdkIdOf(conn: SourceConnection): string {
  return String(conn.filters[SDK_FILTER_KEYS.connectorId] ?? '').trim()
}

/** Checkout, then installed pack, then stored command; a pack must beat stale args. */
export function resolveLaunch(conn: SourceConnection): { command: string; args: string[] } {
  const sdkId = sdkIdOf(conn)
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

// A borrowed token is fetched fresh at spawn, never stored, and sits under anything entered by hand.
export async function buildSpawnConfig(conn: SourceConnection): Promise<SpawnConfig> {
  const { command, args } = resolveLaunch(conn)
  const env = parseJsonObject(conn.filters.env)
  // Decrypted secret env (pushed from main via safeStorage) overrides plain env.
  const decrypted = getDecryptedCreds(conn.id) ?? {}
  const secretEnv = parseJsonObject(decrypted.secretEnv)
  const source = await resolveConnectorAuth(sdkIdOf(conn))
  const borrowed = source ? await borrowedSecrets(source) : {}
  return { command, args, env: { ...borrowed, ...env, ...secretEnv } }
}

export async function getOrStartClient(conn: SourceConnection): Promise<Client> {
  return clients.getOrStart(conn.id, async () => {
    // The spawn config carries the connection's own environment, which wins over
    // the sanitized base every child starts from.
    const { command, args, env } = await buildSpawnConfig(conn)
    return { config: { command, args, env }, meta: { connectionId: conn.id } }
  })
}

export async function stopClient(connectionId: string): Promise<void> {
  await clients.stop(connectionId)
}

export async function stopAllClients(): Promise<void> {
  await clients.stopAll()
}

export function hasClient(connectionId: string): boolean {
  return clients.has(connectionId)
}

/** Which connections a pack change affects, which for a package is not by `connectorId`. */
export function connectionsForConnector(connectorId: string): SourceConnection[] {
  return dbListSourceConnections().filter((conn) => connectionConnectorId(conn) === connectorId)
}

export function connectionIdsForConnector(connectorId: string): string[] {
  return connectionsForConnector(connectorId).map((conn) => conn.id)
}

/** A child started before a pack change keeps running the old files until stopped. */
export async function stopClientsForConnector(connectorId: string): Promise<void> {
  await Promise.allSettled(connectionIdsForConnector(connectorId).map(stopClient))
}
