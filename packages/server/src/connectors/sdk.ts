// Connections to packages built with @vornrun/connector-sdk: one child per connection, spoken to in Vorn's connector protocol.
import {
  SDK_CONNECTOR_ID,
  SDK_FILTER_KEYS,
  type ActionResult,
  type ConnectorActionDef,
  type ConnectorConfigField,
  type ConnectorManifest,
  type ExternalItem,
  type PollResult,
  type SdkAction,
  type SdkActionInput,
  type SdkConnectorManifest,
  type SourceConnection,
  type TriggerEvent,
  type VornConnector
} from '@vornrun/shared/types'
import type {
  JsonValue,
  ProtocolErrorKind,
  TriggerPollItem
} from '@vornrun/shared/connector-protocol'
import { MCP_POLL_EVENT } from './mcp'
import { getOrStartSdkClient, resolveLaunchSource, sdkIdOf, sessionGrantFor } from './mcp-clients'
import { SdkCallError } from './native-client'
import { installedPack } from './packs'
import type { SdkClient } from './sdk-client'
import { toManifest } from './sdk-probe'
import {
  closeSessionCall,
  openSessionCall,
  sessionOutcome,
  type OpenSessionCall
} from './session-bridge'

export { SDK_CONNECTOR_ID }

export interface PreflightReport {
  /** `null` when the connector declares no preflight, which is not the same as passing one. */
  ok: boolean | null
  message?: string
}

const messageOf = (err: unknown): string => (err instanceof Error ? err.message : String(err))

/** The trigger a connection polls; absent on one made only for its actions. */
export function sdkTriggerOf(conn: SourceConnection): string {
  return String(conn.filters[SDK_FILTER_KEYS.trigger] ?? '').trim()
}

// Read from the running child of a checkout or command, and dropped with it.
const childManifests = new WeakMap<SdkClient, SdkConnectorManifest>()

/** What a connection's connector declares: its installed pack, or the child it runs. */
export async function sdkManifestFor(conn: SourceConnection): Promise<SdkConnectorManifest> {
  if (resolveLaunchSource(conn).source === 'pack') {
    const pack = installedPack(sdkIdOf(conn))
    if (pack) return pack
  }
  const client = await getOrStartSdkClient(conn)
  const known = childManifests.get(client)
  if (known) return known
  const manifest = toManifest(await client.manifest())
  childManifests.set(client, manifest)
  return manifest
}

function fieldFor(input: SdkActionInput): ConnectorConfigField {
  const base = {
    key: input.key,
    label: input.label,
    required: input.required,
    supportsTemplates: true,
    ...(input.description && { description: input.description })
  }
  if (input.type === 'select' && input.options && input.options.length > 0) {
    const options = input.options.map((option) => ({
      value: option.value,
      label: option.label ?? option.value
    }))
    return { ...base, type: 'select', options }
  }
  if (input.type === 'json') return { ...base, type: 'textarea', placeholder: '{} or []' }
  return { ...base, type: 'text' }
}

/** An action as the step editor draws it; its outputs become the fields a later step can name. */
export function sdkActionDef(action: SdkAction): ConnectorActionDef {
  const outputs = action.outputs ?? []
  return {
    type: action.type,
    label: action.label,
    ...(action.description && { description: action.description }),
    configFields: (action.inputs ?? []).map(fieldFor),
    ...(outputs.length > 0 && {
      outputSchema: {
        type: 'object',
        properties: Object.fromEntries(
          outputs.map((output) => [
            output.key,
            {
              ...(output.type && { type: output.type }),
              ...(output.description && { description: output.description })
            }
          ])
        )
      }
    })
  }
}

export async function sdkConnectionActions(conn: SourceConnection): Promise<ConnectorActionDef[]> {
  return (await sdkManifestFor(conn)).actions.map(sdkActionDef)
}

const ERROR_KINDS: Partial<Record<ProtocolErrorKind, ActionResult['errorKind']>> = {
  'signed-out': 'needs-sign-in',
  'app-offline': 'app-offline'
}

function failureOf(err: unknown): ActionResult {
  const errorKind = err instanceof SdkCallError && err.kind ? ERROR_KINDS[err.kind] : undefined
  return {
    success: false,
    error: messageOf(err),
    ...(err instanceof SdkCallError && err.output && { output: err.output }),
    ...(errorKind && { errorKind })
  }
}

export async function invokeSdkAction(
  conn: SourceConnection,
  action: string,
  args: Record<string, unknown>
): Promise<ActionResult> {
  let call: OpenSessionCall | undefined
  let outcome: ActionResult
  try {
    const client = await getOrStartSdkClient(conn)
    const grant = sessionGrantFor(conn.id)
    if (grant) call = openSessionCall(grant)
    const output = await client.action({
      action,
      args: args as Record<string, JsonValue>,
      ...(call && { sessionCall: call.key })
    })
    outcome = { success: true, output }
  } catch (err) {
    outcome = failureOf(err)
  }
  return call ? await sessionOutcome(conn, call, outcome) : outcome
}

/** Run a call that may go through the connection's signed-in window, reading a refusal the way an action's is read. */
async function throughWindow<T>(
  conn: SourceConnection,
  run: (sessionCall: string | undefined) => Promise<T>
): Promise<T> {
  const grant = sessionGrantFor(conn.id)
  const call = grant ? openSessionCall(grant) : undefined
  try {
    const value = await run(call?.key)
    if (call) closeSessionCall(call)
    return value
  } catch (err) {
    if (!call) throw err
    const outcome = await sessionOutcome(conn, call, failureOf(err))
    throw new Error(outcome.error ?? messageOf(err), { cause: err })
  }
}

function toEvent(item: TriggerPollItem): TriggerEvent {
  return {
    id: item.externalId,
    type: MCP_POLL_EVENT,
    timestamp: item.updatedAt || new Date().toISOString(),
    data: { ...item }
  }
}

export async function pollSdkConnection(
  conn: SourceConnection,
  trigger: string,
  cursor?: string
): Promise<PollResult> {
  const client = await getOrStartSdkClient(conn)
  const page = await throughWindow(conn, (sessionCall) =>
    client.poll({
      trigger,
      ...(cursor !== undefined && { cursor }),
      ...(sessionCall && { sessionCall })
    })
  )
  return {
    events: page.items.map(toEvent),
    ...(page.nextCursor !== undefined && { nextCursor: page.nextCursor }),
    hasMore: page.hasMore
  }
}

/** Runaway guard: a connector that keeps saying there is more stops here. */
const MAX_BACKFILL_PAGES = 1_000

function toExternalItem(event: TriggerEvent): ExternalItem {
  const data = event.data
  const str = (value: unknown): string => (value == null ? '' : String(value))
  return {
    externalId: str(data.externalId ?? event.id),
    url: str(data.url),
    title: str(data.title),
    description: str(data.description),
    status: str(data.status),
    updatedAt: event.timestamp,
    metadata: data
  }
}

/** Everything a connection's trigger returns from the start, page by page while it says there is more. */
export async function backfillSdkConnection(
  conn: SourceConnection,
  visit: (item: ExternalItem) => void
): Promise<void> {
  const trigger = sdkTriggerOf(conn)
  if (!trigger) {
    throw new Error(`Connection "${conn.name}" has no trigger, so there is nothing to import.`)
  }
  let cursor: string | undefined
  for (let page = 0; page < MAX_BACKFILL_PAGES; page++) {
    const result = await pollSdkConnection(conn, trigger, cursor)
    for (const event of result.events) visit(toExternalItem(event))
    if (!result.hasMore) return
    if (result.nextCursor === undefined || result.nextCursor === cursor) {
      throw new Error(`Connection "${conn.name}" reported more items without advancing its cursor`)
    }
    cursor = result.nextCursor
  }
  throw new Error(`Connection "${conn.name}" exceeded ${MAX_BACKFILL_PAGES} backfill pages`)
}

export async function preflightSdkConnection(conn: SourceConnection): Promise<PreflightReport> {
  return (await getOrStartSdkClient(conn)).preflight()
}

export const sdkConnector: VornConnector = {
  id: SDK_CONNECTOR_ID,
  name: 'Connector package',
  icon: 'mcp',
  capabilities: ['actions', 'triggers', 'tasks'],

  describe(): ConnectorManifest {
    return {
      auth: [
        {
          key: 'command',
          label: 'Command',
          type: 'text',
          description: 'Executable that starts the connector, when it is not an installed pack.'
        },
        {
          key: 'args',
          label: 'Arguments (JSON array)',
          type: 'textarea',
          description: 'JSON array of args passed to the command.'
        },
        {
          key: 'env',
          label: 'Environment (JSON object)',
          type: 'textarea',
          description: 'Non-secret env vars. JSON object of string values.'
        },
        {
          key: 'secretEnv',
          label: 'Secret env (JSON object)',
          type: 'password',
          description: 'Secret env vars encrypted via OS keychain. JSON object of string values.'
        }
      ],
      triggers: [
        {
          type: MCP_POLL_EVENT,
          label: 'Poll',
          description: "Polls the connection's trigger on a schedule and fires once per new item.",
          configFields: [],
          defaultIntervalMs: 300_000
        }
      ],
      // Per connection, from the connector's own manifest; see `connection:listActions`.
      actions: []
    }
  },

  async execute(actionType: string): Promise<ActionResult> {
    return {
      success: false,
      error: `Connector package actions run through connection:executeAction (tried ${actionType}).`
    }
  }
}
