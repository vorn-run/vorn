// A connector that only speaks MCP, answered through the same SdkClient the native protocol uses.
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { SESSION_CALL_META } from '@vornrun/shared/types'
import {
  PROTOCOL_ERROR_CODES,
  type ConnectorOptionsResult,
  type ExtensionFooterResult,
  type JsonValue,
  type TriggerPollItem
} from '@vornrun/shared/connector-protocol'
import { SdkCallError, isRecord, type ChildExit } from './native-client'
import type { SdkClient } from './sdk-client'
import type { SpawnConfig } from './stdio-clients'
import { manifestPayload, textContent } from './sdk-probe'
import {
  MANIFEST_TOOL,
  OPTIONS_TOOL,
  PREFLIGHT_TOOL,
  footerToolName,
  handlerToolName,
  pollToolName
} from './sdk-tools'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

/** How the adapter stops its child and hears it go; absent when someone else owns the client. */
export interface LegacyLifecycle {
  readonly exited: boolean
  close(): Promise<void>
  onExit(listener: (exit: ChildExit) => void): void
}

// Copied from mcp.ts BROWSER_TOOL_TIMEOUT_MS: a tool may make several calls through its window.
const SESSION_CALL_TIMEOUT_MS = 120_000

type ToolResult = Awaited<ReturnType<Client['callTool']>>

function structuredOf(result: ToolResult): Record<string, unknown> | undefined {
  const value = (result as { structuredContent?: unknown }).structuredContent
  return isRecord(value) ? value : undefined
}

// The MCP schema takes every action argument as a string and the connector applies the declared type.
export function toolArgs(args: Record<string, JsonValue>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [key, value] of Object.entries(args)) {
    if (value === null) continue
    out[key] =
      typeof value === 'string'
        ? value
        : typeof value === 'object'
          ? JSON.stringify(value)
          : String(value)
  }
  return out
}

export function legacyMcpSdkClient(client: Client, lifecycle?: LegacyLifecycle): SdkClient {
  const call = (
    name: string,
    args: Record<string, unknown>,
    sessionCall?: string
  ): Promise<ToolResult> =>
    sessionCall
      ? client.callTool(
          { name, arguments: args, _meta: { [SESSION_CALL_META]: sessionCall } },
          undefined,
          { timeout: SESSION_CALL_TIMEOUT_MS }
        )
      : client.callTool({ name, arguments: args })

  const failure = (method: string, name: string, result: ToolResult): SdkCallError =>
    new SdkCallError(
      method,
      PROTOCOL_ERROR_CODES.connectorError,
      textContent(result) ?? `${name} failed`,
      {
        output: result as unknown as Record<string, unknown>
      }
    )

  const malformed = (method: string, name: string, what: string): SdkCallError =>
    new SdkCallError(method, PROTOCOL_ERROR_CODES.connectorError, `${name} returned ${what}`)

  const answer = async (
    method: string,
    name: string,
    args: Record<string, unknown>,
    sessionCall?: string
  ): Promise<Record<string, unknown>> => {
    const result = await call(name, args, sessionCall)
    if (result.isError) throw failure(method, name, result)
    const payload = structuredOf(result)
    if (!payload) throw malformed(method, name, 'no structured result')
    return payload
  }

  return {
    protocol: 'mcp',

    // A client someone else owns is theirs to watch.
    get exited() {
      return lifecycle?.exited ?? false
    },

    async manifest() {
      const result = await call(MANIFEST_TOOL, {})
      if (result.isError) throw failure('connector/manifest', MANIFEST_TOOL, result)
      const payload = manifestPayload(result)
      if (!payload) throw malformed('connector/manifest', MANIFEST_TOOL, 'no manifest')
      return payload
    },

    // Copied from mcp.ts preflightMcpConnection, which this cannot import without the database.
    async preflight() {
      const tools = await client.listTools()
      if (!(tools.tools ?? []).some((tool) => tool.name === PREFLIGHT_TOOL)) return { ok: null }
      const result = await call(PREFLIGHT_TOOL, {})
      const payload = structuredOf(result)
      if (result.isError || !payload) {
        return {
          ok: false,
          message: textContent(result) ?? 'The connector could not report whether it is ready.'
        }
      }
      const message = typeof payload.message === 'string' ? payload.message : undefined
      return { ok: payload.ok === true, ...(message && { message }) }
    },

    async options({ name, sessionCall }) {
      const payload = await answer('connector/options', OPTIONS_TOOL, { name }, sessionCall)
      if (!Array.isArray(payload.options)) {
        throw malformed('connector/options', OPTIONS_TOOL, 'no options')
      }
      return { options: payload.options as ConnectorOptionsResult['options'] }
    },

    async poll({ trigger, cursor, since, limit, sessionCall }) {
      const tool = pollToolName(trigger)
      const payload = await answer(
        'trigger/poll',
        tool,
        {
          ...(since !== undefined && { since }),
          ...(cursor !== undefined && { cursor }),
          ...(limit !== undefined && { limit: String(limit) })
        },
        sessionCall
      )
      if (!Array.isArray(payload.items)) throw malformed('trigger/poll', tool, 'no items')
      return {
        items: payload.items as TriggerPollItem[],
        ...(typeof payload.nextCursor === 'string' && { nextCursor: payload.nextCursor }),
        hasMore: payload.hasMore === true
      }
    },

    // Copied from mcp.ts invokeMcpTool: a declared output comes back as structuredContent, anything else whole.
    async action({ action, args, sessionCall }) {
      const result = await call(action, toolArgs(args), sessionCall)
      const output = structuredOf(result) ?? (result as unknown as Record<string, unknown>)
      if (result.isError) {
        throw new SdkCallError(
          'action/run',
          PROTOCOL_ERROR_CODES.connectorError,
          textContent(result) ?? `MCP tool ${action} reported an error`,
          { output }
        )
      }
      return output
    },

    async footer({ footer, sessionId, worktreePath, agent }) {
      const tool = footerToolName(footer)
      const payload = await answer('extension/footer', tool, { sessionId, worktreePath, agent })
      if (!Array.isArray(payload.items)) throw malformed('extension/footer', tool, 'no items')
      return { items: payload.items as ExtensionFooterResult['items'] }
    },

    async handler({ handler, sessionId, worktreePath, agent, url }) {
      const tool = handlerToolName(handler)
      const result = await call(tool, { sessionId, worktreePath, agent, url })
      if (result.isError) throw failure('extension/handler', tool, result)
      const openPane = structuredOf(result)?.openPane
      return typeof openPane === 'string' && openPane !== '' ? { openPane } : {}
    },

    close: () => (lifecycle ? lifecycle.close() : client.close()),

    onExit(listener) {
      lifecycle?.onExit(listener)
    }
  }
}

export async function startLegacyMcpSdkClient(
  config: SpawnConfig,
  options: { label: string; key: string }
): Promise<SdkClient> {
  const name = `[${options.label}] ${options.key}`
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

  // The transport does not say how the child ended, only that it did.
  const ended: ChildExit = { code: null, signal: null }
  const listeners: Array<(exit: ChildExit) => void> = []
  let exited = false
  client.onclose = () => {
    exited = true
    log.info(`${name} exited`)
    for (const listener of listeners) listener(ended)
  }
  client.onerror = (err) => log.warn(`${name}: ${err.message}`)

  return legacyMcpSdkClient(client, {
    get exited() {
      return exited
    },
    async close() {
      try {
        await client.close()
      } catch (err) {
        log.warn(`${name}: closing failed: ${err instanceof Error ? err.message : String(err)}`)
      }
    },
    onExit(listener) {
      if (exited) queueMicrotask(() => listener(ended))
      else listeners.push(listener)
    }
  })
}
