// One child per connection, kept until it exits or is stopped: MCP servers in MCP, SDK connectors in Vorn's connector protocol.
import type { Client } from '@modelcontextprotocol/sdk/client/index.js'
import {
  SDK_FILTER_KEYS,
  connectionConnectorId,
  type ConnectorConfigField,
  type SdkBrowserSignIn,
  type SourceConnection
} from '@vornrun/shared/types'
import { dbListSourceConnections } from '../database'
import { getDecryptedCreds } from './decrypted-creds'
import { localLaunchSpec } from './catalog'
import { installedLaunch } from './packs'
import { borrowedSecrets } from './auth-rung'
import { resolveConnectorAuth } from './connector-auth'
import { openMcpChild, type McpChild } from './mcp-child'
import { createSdkChildCache, type SdkClient } from './sdk-client'
import { mintSessionGrant, type SessionGrant } from './session-bridge'
import { createChildCache } from './stdio-clients'

const clients = createChildCache<McpChild, { connectionId: string }>('mcp-clients', openMcpChild)

const sdkClients = createSdkChildCache<{ connectionId: string; grant?: SessionGrant }>('connectors')

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

type LaunchField = 'command' | 'args' | 'env' | 'secretEnv'

const LAUNCH_AUTH_FIELDS: Record<LaunchField, ConnectorConfigField> = {
  command: { key: 'command', label: 'Command', type: 'text' },
  args: {
    key: 'args',
    label: 'Arguments (JSON array)',
    type: 'textarea',
    description: 'JSON array of args passed to the command.'
  },
  env: {
    key: 'env',
    label: 'Environment (JSON object)',
    type: 'textarea',
    description: 'Non-secret env vars. JSON object of string values.'
  },
  secretEnv: {
    key: 'secretEnv',
    label: 'Secret env (JSON object)',
    type: 'password',
    description: 'Secret env vars encrypted via OS keychain. JSON object of string values.'
  }
}

/** The fields that say how a connection's child starts; `secretEnv` is a password, so it stays encrypted and masked. */
export function launchAuthFields(
  overrides: Partial<Record<LaunchField, Partial<ConnectorConfigField>>> = {}
): ConnectorConfigField[] {
  return (Object.keys(LAUNCH_AUTH_FIELDS) as LaunchField[]).map((key) => ({
    ...LAUNCH_AUTH_FIELDS[key],
    ...overrides[key]
  }))
}

/** The connector a connection runs, when it names a packaged one. */
export function sdkIdOf(conn: SourceConnection): string {
  return String(conn.filters[SDK_FILTER_KEYS.connectorId] ?? '').trim()
}

export type LaunchSource = 'checkout' | 'pack' | 'command'

/** How a connection's child starts, where that came from, and what an installed pack says of itself. */
export interface LaunchSpec {
  command: string
  args: string[]
  source: LaunchSource
  protocol?: number
  /** The name an installed pack gives itself. */
  name?: string
}

/** An installed pack's launch; none before the database has resolved a data directory. */
export function packLaunchSpec(id: string): LaunchSpec | undefined {
  try {
    const launch = installedLaunch(id)
    return launch && { ...launch, source: 'pack' }
  } catch {
    return undefined
  }
}

/** Checkout, then installed pack, then stored command; a pack must beat stale args. */
export function resolveLaunchSource(conn: SourceConnection): LaunchSpec {
  const sdkId = sdkIdOf(conn)
  if (sdkId) {
    const local = localLaunchSpec(sdkId)
    if (local) return { command: local.command, args: local.args, source: 'checkout' }
    const pack = packLaunchSpec(sdkId)
    if (pack) return pack
  }
  const command = String(conn.filters.command ?? '').trim()
  if (!command) throw new Error('MCP connection is missing a command')
  return { command, args: parseJsonArray(conn.filters.args), source: 'command' }
}

export function resolveLaunch(conn: SourceConnection): { command: string; args: string[] } {
  const { command, args } = resolveLaunchSource(conn)
  return { command, args }
}

/** Everything a connection's child starts with; `browser` is set when it acts through a signed-in window. */
export interface SpawnSpec extends LaunchSpec {
  env: Record<string, string>
  browser?: SdkBrowserSignIn
}

// A borrowed token is fetched fresh at spawn, never stored, and sits under anything entered by hand.
export async function buildSpawnConfig(conn: SourceConnection): Promise<SpawnSpec> {
  const launch = resolveLaunchSource(conn)
  const env = parseJsonObject(conn.filters.env)
  // Decrypted secret env (pushed from main via safeStorage) overrides plain env.
  const secretEnv = parseJsonObject((getDecryptedCreds(conn.id) ?? {}).secretEnv)
  const source = await resolveConnectorAuth(sdkIdOf(conn))
  const borrowed = source ? await borrowedSecrets(source) : {}
  const browser = source?.auth?.rung === 'browser' ? source.auth.browser : undefined
  return { ...launch, env: { ...borrowed, ...env, ...secretEnv }, ...(browser && { browser }) }
}

export async function getOrStartClient(conn: SourceConnection): Promise<Client> {
  const child = await clients.getOrStart(conn.id, async () => {
    const { command, args, env } = await buildSpawnConfig(conn)
    return { config: { command, args, env }, meta: { connectionId: conn.id } }
  })
  return child.client
}

/** An SDK connection's child, started on first use; its signed-in window is reached with a token minted for this child alone. */
export async function getOrStartSdkClient(conn: SourceConnection): Promise<SdkClient> {
  return sdkClients.getOrStart(conn.id, async () => {
    const { browser, name, ...launch } = await buildSpawnConfig(conn)
    const session = browser && mintSessionGrant(conn.id, browser)
    return {
      config: { ...launch, env: { ...launch.env, ...session?.env }, name: name || conn.name },
      meta: { connectionId: conn.id, ...(session && { grant: session.grant }) }
    }
  })
}

/** The signed-in window grant of a connection's running child, if it has one. */
export function sessionGrantFor(connectionId: string): SessionGrant | undefined {
  return sdkClients.get(connectionId)?.grant
}

export async function stopClient(connectionId: string): Promise<void> {
  await Promise.all([clients.stop(connectionId), sdkClients.stop(connectionId)])
}

export async function stopAllClients(): Promise<void> {
  await Promise.all([clients.stopAll(), sdkClients.stopAll()])
}

export function hasClient(connectionId: string): boolean {
  return clients.has(connectionId) || sdkClients.has(connectionId)
}

/** Which connections a pack change affects, which for a package is not by `connectorId`. */
export function connectionsForConnector(connectorId: string): SourceConnection[] {
  return dbListSourceConnections().filter((conn) => connectionConnectorId(conn) === connectorId)
}

/** A child started before a pack change keeps running the old files until stopped. */
export async function stopClientsForConnector(connectorId: string): Promise<void> {
  await Promise.allSettled(connectionsForConnector(connectorId).map((conn) => stopClient(conn.id)))
}
