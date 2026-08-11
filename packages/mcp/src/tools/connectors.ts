/**
 * Connector tools.
 *
 * Vorn's own connectors were invisible to agents: an agent could create a
 * workflow that calls a connector action but could not see which connectors
 * existed, which were set up, or why one was failing. These tools close that
 * gap — discovery, installation and invocation — so an agent can work on a
 * connector-backed workflow without a person reading the settings screen out
 * to it.
 *
 * Every tool is a thin call into the same server methods the UI uses, so
 * behaviour cannot drift between what an agent does and what a person does.
 */
import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { V } from '../validation'
import type {
  ActionResult,
  ConnectorActionDef,
  ConnectorCatalogSnapshot,
  ConnectorCatalogSummary,
  ConnectorManifest,
  SdkProbeResult,
  SourceConnection
} from '@vornrun/shared/types'
import { rpcCall } from '../ws-client'
import { SDK_FILTER_KEYS, connectionConnectorId } from '@vornrun/shared/types'

/**
 * Starting a connector package downloads it first, so the probe is allowed
 * far longer than an ordinary call. The server caps its own attempt below
 * this, so it reports a real error rather than being cut off here.
 */
const PROBE_TIMEOUT_MS = 120_000

interface ConnectorListEntry {
  id: string
  name: string
  capabilities: string[]
  manifest: ConnectorManifest
}

interface ConnectorStatus {
  connectorId: string
  authed: boolean
  message?: string
}

const json = (value: unknown) => ({
  content: [{ type: 'text' as const, text: JSON.stringify(value, null, 2) }]
})

const failure = (message: string) => ({
  content: [{ type: 'text' as const, text: `Error: ${message}` }],
  isError: true
})

/** An agent needs the label to decide and the type to call it; the rest is prose. */
function summarize(entry: ConnectorCatalogSummary): { type: string; label: string } {
  return { type: entry.type, label: entry.label }
}

export function registerConnectorTools(server: McpServer): void {
  server.tool(
    'list_connectors',
    'List every connector: the ones built into Vorn, the ones installable from a package, ' +
      'and how many connections each already has. Use this before creating a workflow that ' +
      'calls a connector action, or to find the id of a connector to install.',
    {
      installable_only: z.boolean().optional().describe('Only connectors that are not set up yet')
    },
    async (args) => {
      const [builtIns, snapshot, connections, statuses] = await Promise.all([
        rpcCall<ConnectorListEntry[]>('connector:list'),
        rpcCall<ConnectorCatalogSnapshot>('connector:catalog'),
        rpcCall<SourceConnection[]>('connection:list', { connectorId: undefined }),
        rpcCall<ConnectorStatus[]>('connector:status')
      ])

      const countFor = (id: string) =>
        connections.filter((conn) => connectionConnectorId(conn) === id).length
      const statusFor = (id: string) => statuses.find((s) => s.connectorId === id)

      const entries = [
        ...builtIns.map((c) => ({
          id: c.id,
          name: c.name,
          source: 'built-in' as const,
          capabilities: c.capabilities,
          connections: countFor(c.id),
          // Only meaningful for connectors that authenticate up front; the
          // rest report nothing rather than a misleading "not authed".
          ...(statusFor(c.id) && {
            authenticated: statusFor(c.id)!.authed,
            ...(statusFor(c.id)!.message && { authMessage: statusFor(c.id)!.message })
          })
        })),
        ...snapshot.items.map((entry) => ({
          id: entry.id,
          name: entry.name,
          source: 'package' as const,
          description: entry.description,
          package: entry.packageName,
          ...(entry.version && { version: entry.version }),
          capabilities: entry.capabilities,
          connections: countFor(entry.id),
          ...(entry.auth && { auth: entry.auth }),
          // Generated upstream from the connector's own manifest, so an agent
          // can tell whether a connector is worth installing without launching
          // it — which for a list of twenty would be twenty npx processes.
          ...(entry.triggers && { triggers: entry.triggers.map(summarize) }),
          ...(entry.actions && { actions: entry.actions.map(summarize) }),
          ...(entry.env && { env: entry.env.map((e) => e.name) })
        }))
      ]

      return json(args.installable_only ? entries.filter((e) => e.connections === 0) : entries)
    }
  )

  server.tool(
    'list_connections',
    'List configured connector connections, including when each last synced and the error ' +
      'from its last failure. Use this to diagnose a connector that is not producing tasks.',
    {
      connector_id: V.id.optional().describe('Only connections for this connector'),
      failing_only: z.boolean().optional().describe('Only connections whose last sync failed')
    },
    async (args) => {
      const connections = await rpcCall<SourceConnection[]>('connection:list', {
        connectorId: undefined
      })

      const visible = connections
        .filter((conn) => !args.connector_id || connectionConnectorId(conn) === args.connector_id)
        .filter((conn) => !args.failing_only || !!conn.lastSyncError)

      return json(
        visible.map((conn) => ({
          id: conn.id,
          name: conn.name,
          connectorId: connectionConnectorId(conn),
          project: conn.executionProject,
          syncIntervalMinutes: conn.syncIntervalMinutes,
          lastSyncAt: conn.lastSyncAt,
          lastSyncError: conn.lastSyncError,
          // Deliberately not the whole `filters` blob: it holds encrypted
          // credentials, and an agent has no use for ciphertext.
          config: publicFilters(conn)
        }))
      )
    }
  )

  server.tool(
    'list_connector_actions',
    'List the actions a connection can execute, with their input schemas. Call this before ' +
      'run_connector_action or before adding a callConnectorAction node to a workflow.',
    { connection_id: V.id.describe('Connection ID') },
    async (args) => {
      const actions = await rpcCall<ConnectorActionDef[]>(
        'connection:listActions',
        args.connection_id
      )
      if (actions.length === 0) {
        return failure(
          `No actions for connection "${args.connection_id}". Either the connection does not ` +
            'exist, or its connector exposes no actions yet — for an MCP connection, tool ' +
            'discovery may still be running.'
        )
      }
      return json(actions)
    }
  )

  server.tool(
    'inspect_connector_package',
    'Start a connector package and read what it offers — its triggers, actions and required ' +
      'environment variables — without installing it. Use this to review a connector before ' +
      'install_connector, or to check a local build.',
    {
      package: V.shortText.describe(
        'npm package name, or a command to run a local build (e.g. "node /path/to/dist/index.js")'
      )
    },
    async (args) => {
      const result = await probe(args.package)
      if (!result.ok) return failure(result.error)
      return json(result.manifest)
    }
  )

  server.tool(
    'install_connector',
    'Install a connector from the catalog or from an npm package, creating a connection ready ' +
      'to poll. Call list_connectors for catalog ids and inspect_connector_package to see which ' +
      'environment variables are needed. Secrets cannot be set this way — see the error it ' +
      'returns if the connector requires one.',
    {
      connector_id: V.id
        .optional()
        .describe('Catalog connector id (from list_connectors). Use this or package.'),
      package: V.shortText.optional().describe('npm package name or launch command'),
      name: V.title.optional().describe('Connection name (defaults to the connector name)'),
      project: V.name.optional().describe('Vorn project tasks should be created in'),
      trigger: V.shortText
        .optional()
        .describe('Trigger type to configure (defaults to the first the connector offers)'),
      env: z
        .record(z.string(), z.string())
        .optional()
        .describe('Non-secret environment variables the connector needs'),
      sync_interval_minutes: z.number().int().min(1).max(1440).optional()
    },
    async (args) => {
      const { items: catalog } = await rpcCall<ConnectorCatalogSnapshot>('connector:catalog')
      const entry = args.connector_id ? catalog.find((c) => c.id === args.connector_id) : undefined

      if (args.connector_id && !entry) {
        return failure(
          `No connector "${args.connector_id}" in the catalog. Known: ` +
            `${catalog.map((c) => c.id).join(', ') || '(none)'}. ` +
            'To install something not in the catalog, pass `package` instead.'
        )
      }
      const target = entry ? entry.launch : args.package
      if (!target) return failure('Provide either connector_id or package.')

      const result = await probe(target)
      if (!result.ok) return failure(result.error)
      const manifest = result.manifest

      const supplied = args.env ?? {}
      const unknown = Object.keys(supplied).filter(
        (name) => !manifest.env.some((e) => e.name === name)
      )
      if (unknown.length > 0) {
        return failure(
          `${manifest.name} does not use ${unknown.join(', ')}. It accepts: ` +
            `${manifest.env.map((e) => e.name).join(', ') || '(none)'}.`
        )
      }

      // Refused rather than stored in the clear: encryption runs in the
      // desktop process, which this one cannot reach, so the only way to
      // accept a secret here would be to write a credential to the database
      // unprotected. This covers a secret the connector demands and one the
      // caller offered unasked — an optional secret is no less a credential.
      const secrets = manifest.env.filter((e) => e.secret && (e.required || supplied[e.name]))
      if (secrets.length > 0) {
        return failure(
          `${manifest.name} uses the secret ${plural(secrets.length, 'value')} ` +
            `${secrets.map((e) => e.name).join(', ')}, which this tool cannot accept: it runs ` +
            'outside the desktop process, where encryption lives, so it could only store them ' +
            'unprotected. They must be entered by a person in Settings > Connectors to reach ' +
            'the OS keychain. Everything else about the connector is ready to install.'
        )
      }

      const missing = manifest.env.filter((e) => e.required && !supplied[e.name]?.trim())
      if (missing.length > 0) {
        return failure(
          `${manifest.name} needs ${missing.map((e) => describeEnv(e)).join(', ')}. ` +
            'Pass them in `env`.'
        )
      }

      const trigger = args.trigger
        ? manifest.triggers.find((t) => t.type === args.trigger)
        : manifest.triggers[0]
      if (args.trigger && !trigger) {
        return failure(
          `${manifest.name} has no trigger "${args.trigger}". It offers: ` +
            `${manifest.triggers.map((t) => t.type).join(', ') || '(none)'}.`
        )
      }

      const launch = typeof target === 'string' ? parseLaunch(target) : target
      const connection = await rpcCall<SourceConnection>('connection:create', {
        connectorId: 'mcp',
        name: args.name ?? (trigger ? `${manifest.name}: ${trigger.label}` : manifest.name),
        filters: {
          command: launch.command,
          args: JSON.stringify(launch.args),
          env: JSON.stringify(supplied),
          [SDK_FILTER_KEYS.connectorId]: manifest.id,
          [SDK_FILTER_KEYS.version]: manifest.version,
          ...(manifest.icon && { [SDK_FILTER_KEYS.icon]: JSON.stringify(manifest.icon) }),
          ...(trigger?.filters ?? {})
        },
        syncIntervalMinutes: args.sync_interval_minutes ?? 5,
        statusMapping: {},
        ...(args.project && { executionProject: args.project })
      })

      return json({
        installed: manifest.name,
        connectionId: connection.id,
        trigger: trigger?.type,
        note: 'Poll it now with backfill_connection, or reference it from a workflow.'
      })
    }
  )

  server.tool(
    'run_connector_action',
    'Execute one action on a connection — create an issue, run a query, close a work item. ' +
      'Call list_connector_actions first for the action name and its arguments.',
    {
      connection_id: V.id.describe('Connection ID'),
      action: V.shortText.describe('Action name from list_connector_actions'),
      args: z.record(z.string(), z.unknown()).optional().describe('Action arguments')
    },
    async (args) => {
      const result = await rpcCall<ActionResult>('connection:executeAction', {
        connectionId: args.connection_id,
        action: args.action,
        args: args.args ?? {}
      })
      // Returned as a tool error so a failure is not mistaken for a result.
      if (!result.success) return failure(result.error ?? 'Action failed')
      return json(result)
    }
  )

  server.tool(
    'backfill_connection',
    'Pull items from a connection now and turn them into tasks, without waiting for its poll ' +
      'interval. Use this to verify a connection works after installing it.',
    { connection_id: V.id.describe('Connection ID') },
    async (args) => {
      const result = await rpcCall<{ imported: number; updated: number; error?: string }>(
        'connection:backfill',
        { connectionId: args.connection_id },
        PROBE_TIMEOUT_MS
      )
      if (result.error) return failure(result.error)
      return json(result)
    }
  )
}

/** Read a connector by starting it, given a package name, command, or spec. */
async function probe(
  target: string | { command: string; args: string[] }
): Promise<SdkProbeResult> {
  const launch = typeof target === 'string' ? parseLaunch(target) : target
  return rpcCall<SdkProbeResult>('connector:probeSdk', launch, PROBE_TIMEOUT_MS)
}

/**
 * A bare package name runs through `npx`; anything with spaces is already a
 * command, which is how a local build is loaded.
 */
function parseLaunch(spec: string): { command: string; args: string[] } {
  const parts = spec.trim().split(/\s+/)
  if (parts.length === 1) return { command: 'npx', args: ['-y', parts[0]] }
  return { command: parts[0], args: parts.slice(1) }
}

/** Connection config minus anything holding a credential. */
function publicFilters(conn: SourceConnection): Record<string, unknown> {
  const hidden = new Set(['secretEnv', 'discoveredTools'])
  return Object.fromEntries(Object.entries(conn.filters ?? {}).filter(([key]) => !hidden.has(key)))
}

function describeEnv(entry: { name: string; description?: string }): string {
  return entry.description ? `${entry.name} (${entry.description})` : entry.name
}

function plural(count: number, word: string): string {
  return count === 1 ? word : `${word}s`
}
