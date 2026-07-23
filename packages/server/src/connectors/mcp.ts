/**
 * Generic MCP (Model Context Protocol) connector.
 *
 * Where the github / linear connectors wrap a single upstream API, the MCP
 * connector is polymorphic: each connection points at some MCP server (any
 * stdio process — Filesystem, Azure DevOps, custom) and inherits that
 * server's tool surface dynamically via `tools/list`. Tool definitions are
 * stored on the connection row at discovery time (see `discoverTools`) so
 * the UI can render an invoke form without spawning the child each render.
 *
 * Auth model for the spike: static env vars. A non-secret `env` JSON object
 * is stored plaintext in `filters.env`, and a `secretEnv` JSON object is
 * encrypted through the same safeStorage path used by Linear's `apiKey`.
 * The decrypted values are merged in at spawn time via `getOrStartClient`.
 */
import type {
  VornConnector,
  ConnectorManifest,
  ConnectorActionDef,
  ConnectorConfigField,
  ActionResult,
  PollResult,
  TriggerEvent,
  SourceConnection
} from '@vornrun/shared/types'
import { schemaProperties, schemaTypeHint, schemaRequired } from '@vornrun/shared/json-schema-utils'
import { getOrStartClient } from './mcp-clients'

/** Stable id for the generic MCP connector. Used everywhere the server
 *  needs to distinguish MCP from static connectors. */
export const MCP_CONNECTOR_ID = 'mcp'

/** The single trigger type the MCP connector exposes. Its behavior is driven
 *  entirely by the connection's poll config (pollTool/itemsPath/…), so one
 *  static event covers every MCP server rather than a per-server manifest. */
export const MCP_POLL_EVENT = 'mcpPoll'

export interface McpDiscoveredTool {
  name: string
  description?: string
  inputSchema?: Record<string, unknown>
  outputSchema?: Record<string, unknown>
}

/**
 * Map an MCP tool's JSON-Schema `inputSchema` to the generic
 * `ConnectorActionDef` shape so the workflow editor can render tool args
 * with the same form it uses for every other connector. Types are coerced
 * back at execute time by `coerceMcpArgs` — here we pick input widgets.
 */
export function mcpToolToConnectorAction(tool: McpDiscoveredTool): ConnectorActionDef {
  const configFields: ConnectorConfigField[] = Object.entries(
    schemaProperties(tool.inputSchema)
  ).map(([key, raw]) => {
    const prop = raw as { description?: string; enum?: unknown[]; default?: unknown }
    const declaredType = schemaTypeHint(raw)
    const fieldBase = {
      key,
      label: key,
      required: schemaRequired(tool.inputSchema, key),
      ...(prop.description && { description: prop.description }),
      ...(prop.default !== undefined && { placeholder: JSON.stringify(prop.default) }),
      supportsTemplates: true
    }
    if (Array.isArray(prop.enum) && prop.enum.length > 0) {
      return {
        ...fieldBase,
        type: 'select' as const,
        options: prop.enum.map((v) => ({ value: String(v), label: String(v) }))
      }
    }
    // Non-scalar values live in a textarea and are JSON-parsed at execute time.
    if (declaredType === 'object' || declaredType === 'array') {
      return { ...fieldBase, type: 'textarea' as const, placeholder: '{} or []' }
    }
    return { ...fieldBase, type: 'text' as const }
  })
  return {
    type: tool.name,
    label: tool.name,
    ...(tool.description && { description: tool.description }),
    configFields,
    ...(tool.outputSchema && { outputSchema: tool.outputSchema })
  }
}

/**
 * Convert string form values back into the types the MCP tool expects,
 * using the stored inputSchema. Strings stay as strings; numeric/bool/
 * object/array fields are parsed. Invalid inputs pass through so the MCP
 * server's validator surfaces a meaningful error.
 */
function coerceMcpArgs(
  inputSchema: Record<string, unknown> | undefined,
  args: Record<string, unknown>
): Record<string, unknown> {
  if (!inputSchema) return args
  const properties = schemaProperties(inputSchema)
  if (Object.keys(properties).length === 0) return args
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(args)) {
    const t = schemaTypeHint(properties[key])
    if (typeof value !== 'string') {
      out[key] = value
      continue
    }
    // Empty-string handling depends on the declared type. For strings we
    // keep `""` (the user may legitimately want to send an empty value, or
    // wants the server to surface its own validation error). For non-string
    // types `""` is never a valid coerced payload — dropping the key lets
    // the MCP server apply its own default for optional fields, and lets
    // its validator complain explicitly for required ones rather than us
    // forwarding an obviously-wrong value.
    if (value === '' && t !== undefined && t !== 'string') {
      if (!schemaRequired(inputSchema, key)) continue
    }
    if (t === 'number' || t === 'integer') {
      if (value === '') {
        out[key] = value
        continue
      }
      const n = Number(value)
      out[key] = Number.isFinite(n) ? n : value
    } else if (t === 'boolean') {
      out[key] = value === 'true' ? true : value === 'false' ? false : value
    } else if (t === 'object' || t === 'array') {
      if (value === '') {
        out[key] = value
        continue
      }
      try {
        out[key] = JSON.parse(value)
      } catch {
        out[key] = value
      }
    } else {
      out[key] = value
    }
  }
  return out
}

/** Spawn the MCP server (if not already running) and run `tools/list`. */
export async function discoverTools(conn: SourceConnection): Promise<McpDiscoveredTool[]> {
  const client = await getOrStartClient(conn)
  const result = await client.listTools()
  return (result.tools ?? []).map((t) => {
    const tool = t as typeof t & { outputSchema?: Record<string, unknown> }
    return {
      name: tool.name,
      ...(tool.description && { description: tool.description }),
      ...(tool.inputSchema && { inputSchema: tool.inputSchema as Record<string, unknown> }),
      ...(tool.outputSchema && { outputSchema: tool.outputSchema })
    }
  })
}

/** Return the actions a given MCP connection exposes, in the same shape as
 *  any other connector's static manifest. Empty until discovery completes. */
export function mcpConnectionActions(conn: SourceConnection): ConnectorActionDef[] {
  const tools = conn.filters.discoveredTools
  if (!Array.isArray(tools)) return []
  return (tools as McpDiscoveredTool[]).map(mcpToolToConnectorAction)
}

/** Invoke a single MCP tool. Separate from `VornConnector.execute` because
 *  we need the `SourceConnection` itself to start/address the per-connection
 *  client, not just the merged args the generic execute path provides. */
export async function invokeMcpTool(
  conn: SourceConnection,
  toolName: string,
  args: Record<string, unknown>
): Promise<ActionResult> {
  try {
    const client = await getOrStartClient(conn)
    // Look up this tool's discovered inputSchema so we can coerce string form
    // values back to the types the tool actually expects.
    const tools = conn.filters.discoveredTools
    const tool = Array.isArray(tools)
      ? (tools as McpDiscoveredTool[]).find((t) => t.name === toolName)
      : undefined
    const callArgs = coerceMcpArgs(tool?.inputSchema, args)
    const result = await client.callTool({ name: toolName, arguments: callArgs })
    // When the tool declared an outputSchema, MCP returns the typed payload
    // under `structuredContent`. Surface that as `output` so downstream
    // workflow steps can reference the declared fields directly
    // (`{{steps.x.fieldName}}`) without needing to drill through the
    // `structuredContent` wrapper. Tools without an outputSchema fall back
    // to the raw {content, isError} envelope.
    const structured = (result as { structuredContent?: Record<string, unknown> }).structuredContent
    const output = (structured ?? (result as unknown as Record<string, unknown>)) as Record<
      string,
      unknown
    >
    if (result.isError) {
      return {
        success: false,
        error: extractTextError(result.content) ?? `MCP tool ${toolName} reported an error`,
        output
      }
    }
    return { success: true, output }
  } catch (err) {
    return { success: false, error: err instanceof Error ? err.message : String(err) }
  }
}

// --- Poll / trigger support -------------------------------------------------

/** Per-connection poll config, read from `conn.filters`. All optional; without
 *  a `pollTool` the connection simply isn't a trigger source. */
interface McpPollConfig {
  pollTool?: string
  pollArgs?: string
  itemsPath?: string
  idField?: string
  timestampField?: string
  titleField?: string
  urlField?: string
}

function readPollConfig(conn: SourceConnection): McpPollConfig {
  const f = conn.filters
  const str = (v: unknown): string | undefined => {
    const s = typeof v === 'string' ? v.trim() : ''
    return s || undefined
  }
  return {
    pollTool: str(f.pollTool),
    pollArgs: str(f.pollArgs),
    itemsPath: str(f.itemsPath),
    idField: str(f.idField),
    timestampField: str(f.timestampField),
    titleField: str(f.titleField),
    urlField: str(f.urlField)
  }
}

/** Walk a dotted path (e.g. `data.pullRequests`) into a value. An empty path
 *  returns the root so a tool that returns the array at top level still works. */
function walkPath(root: unknown, path: string | undefined): unknown {
  if (!path) return root
  let current: unknown = root
  for (const segment of path.split('.')) {
    if (current == null || typeof current !== 'object') return undefined
    current = (current as Record<string, unknown>)[segment]
  }
  return current
}

function fieldString(item: Record<string, unknown>, field: string | undefined): string | undefined {
  if (!field) return undefined
  const v = item[field]
  return v == null ? undefined : String(v)
}

/**
 * Poll an MCP connection: invoke its configured `pollTool`, pull the array at
 * `itemsPath` out of the result, and emit one `TriggerEvent` per new item.
 *
 * Kept as a standalone function (not `mcpConnector.poll`) because — like
 * `invokeMcpTool` — it needs the full `SourceConnection` to spawn/address the
 * per-connection stdio client, which the generic `poll(config)` signature (a
 * flattened filters object) can't provide. The scheduler special-cases MCP to
 * call this directly.
 *
 * Cursor semantics mirror the GitHub connector: when a `timestampField` is
 * configured, only items with `timestampField > cursor` are emitted and the
 * cursor advances to the newest timestamp seen. Without a `timestampField`
 * every item is emitted each poll; that stays at-most-once because
 * `createTaskFromItem` upserts by `externalId` (from `idField`).
 */
export async function pollMcpConnection(
  conn: SourceConnection,
  cursor?: string
): Promise<PollResult> {
  const cfg = readPollConfig(conn)
  if (!cfg.pollTool) return { events: [] }

  let pollArgs: Record<string, unknown> = {}
  if (cfg.pollArgs) {
    try {
      const parsed = JSON.parse(cfg.pollArgs)
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        pollArgs = parsed as Record<string, unknown>
      } else {
        throw new Error('pollArgs must be a JSON object')
      }
    } catch (err) {
      throw new Error(
        `Invalid pollArgs JSON: ${err instanceof Error ? err.message : String(err)}`,
        {
          cause: err
        }
      )
    }
  }

  const result = await invokeMcpTool(conn, cfg.pollTool, pollArgs)
  if (!result.success) {
    throw new Error(result.error || `MCP poll tool ${cfg.pollTool} failed`)
  }

  const rawItems = walkPath(result.output, cfg.itemsPath)
  if (!Array.isArray(rawItems)) {
    const where = cfg.itemsPath ? `itemsPath "${cfg.itemsPath}"` : 'the tool result'
    throw new Error(`MCP poll: ${where} did not resolve to an array`)
  }

  const items = rawItems.filter(
    (it): it is Record<string, unknown> => !!it && typeof it === 'object' && !Array.isArray(it)
  )

  const events: TriggerEvent[] = []
  let newest = cursor
  for (const item of items) {
    const ts = fieldString(item, cfg.timestampField)
    if (cfg.timestampField) {
      // With ordering configured, an item that lacks the timestamp field can't
      // be placed relative to the cursor — emit it and it would re-fire every
      // poll. Skip it rather than churn.
      if (ts === undefined) continue
      // Skip items at or before the cursor.
      if (cursor && ts <= cursor) continue
    }
    if (ts !== undefined && (newest === undefined || ts > newest)) newest = ts

    const idVal = fieldString(item, cfg.idField)
    const id = idVal ?? JSON.stringify(item)
    const url = fieldString(item, cfg.urlField)
    const title = fieldString(item, cfg.titleField)

    events.push({
      id,
      type: MCP_POLL_EVENT,
      timestamp: ts ?? new Date().toISOString(),
      // Spread the raw item so nothing is lost, then normalize the fields the
      // scheduler reads to build the connectorItem (externalId/url/title).
      data: {
        ...item,
        externalId: id,
        ...(url !== undefined && { url }),
        ...(title !== undefined && { title })
      }
    })
  }

  // Advance the cursor only when we can order by timestamp; otherwise leave it
  // unset and rely on upsert dedup.
  return cfg.timestampField ? { events, nextCursor: newest ?? cursor } : { events }
}

function extractTextError(content: unknown): string | undefined {
  if (!Array.isArray(content)) return undefined
  for (const block of content) {
    if (block && typeof block === 'object' && 'type' in block && block.type === 'text') {
      const text = (block as { text?: unknown }).text
      if (typeof text === 'string') return text
    }
  }
  return undefined
}

export const mcpConnector: VornConnector = {
  id: 'mcp',
  name: 'MCP',
  icon: 'mcp',
  // 'triggers' is always declared; whether a given connection actually fires
  // depends on it having a `pollTool` configured (see pollMcpConnection).
  capabilities: ['actions', 'triggers'],

  describe(): ConnectorManifest {
    return {
      auth: [
        {
          key: 'command',
          label: 'Command',
          type: 'text',
          required: true,
          placeholder: 'npx',
          description: 'Executable to run the MCP server (npx, node, uv, python, …).'
        },
        {
          key: 'args',
          label: 'Arguments (JSON array)',
          type: 'textarea',
          required: true,
          placeholder: '["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]',
          description: 'JSON array of args passed to the command.'
        },
        {
          key: 'env',
          label: 'Environment (JSON object)',
          type: 'textarea',
          placeholder: '{"MCP_LOG_LEVEL": "info"}',
          description: 'Non-secret env vars. JSON object of string values.'
        },
        {
          key: 'secretEnv',
          label: 'Secret env (JSON object)',
          type: 'password',
          placeholder: '{"AZURE_DEVOPS_EXT_PAT": "<token>"}',
          description: 'Secret env vars encrypted via OS keychain. JSON object of string values.'
        },
        // --- Poll trigger (optional) ---
        // Setting `pollTool` turns this connection into a trigger source: a
        // `connectorPoll` trigger with event `mcpPoll` runs the tool on a cron
        // and fires one workflow per new item. Leave blank for action-only use.
        {
          key: 'pollTool',
          label: 'Poll tool (optional)',
          type: 'text',
          placeholder: 'list_pull_requests',
          description:
            'MCP tool to run on the poll interval. Setting this makes the connection a ' +
            'trigger source (event: MCP Poll).'
        },
        {
          key: 'pollArgs',
          label: 'Poll args (JSON object)',
          type: 'textarea',
          placeholder: '{"status": "active"}',
          description: 'Static arguments passed to the poll tool. JSON object.'
        },
        {
          key: 'itemsPath',
          label: 'Items path',
          type: 'text',
          placeholder: 'pullRequests',
          description:
            'Dotted path into the tool result that holds the array of items (e.g. `value` ' +
            'or `data.pullRequests`). Blank = the result is itself the array.'
        },
        {
          key: 'idField',
          label: 'ID field',
          type: 'text',
          placeholder: 'pullRequestId',
          description: 'Field on each item used as the dedup key (recommended for at-most-once).'
        },
        {
          key: 'timestampField',
          label: 'Timestamp field',
          type: 'text',
          placeholder: 'creationDate',
          description:
            'Field used to advance the cursor and only fire for items newer than the last ' +
            'poll. Blank = every item fires each poll (deduped by ID field).'
        },
        {
          key: 'titleField',
          label: 'Title field',
          type: 'text',
          placeholder: 'title',
          description: 'Field mapped into the created task title.'
        },
        {
          key: 'urlField',
          label: 'URL field',
          type: 'text',
          placeholder: 'url',
          description: 'Field mapped into the created task URL.'
        }
      ],
      triggers: [
        {
          type: MCP_POLL_EVENT,
          label: 'MCP Poll',
          description:
            "Runs this connection's configured poll tool on a schedule and fires once per new item.",
          // Config lives on the connection (poll tool/mapping), not the trigger.
          configFields: [],
          defaultIntervalMs: 300_000
        }
      ],
      // Actions are per-connection (discovered via tools/list). The static
      // list stays empty; callers query `connection:listActions` instead.
      actions: []
    }
  },

  /** VornConnector.execute is the generic entry point. The MCP execute path
   *  is routed through `invokeMcpTool` at the IPC layer because it needs the
   *  full SourceConnection to spawn/address the per-connection stdio client.
   *  This stub exists only so capabilities include 'actions'. */
  async execute(actionType: string): Promise<ActionResult> {
    return {
      success: false,
      error: `MCP actions must be invoked via connection:executeAction (tried ${actionType}).`
    }
  }
}
