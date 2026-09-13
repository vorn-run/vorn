// Connections to packages built with @vornrun/connector-sdk: one child per connection, spoken to in Vorn's connector protocol.
import {
  CONNECTOR_POLL_EVENT,
  SDK_CONNECTOR_ID,
  SDK_FILTER_KEYS,
  actionInputField,
  type ActionResult,
  type ConnectorActionDef,
  type ConnectorManifest,
  type ExternalItem,
  type PollResult,
  type SdkAction,
  type SdkConnectorManifest,
  type SourceConnection,
  type TriggerEvent,
  type VornConnector
} from '@vornrun/shared/types'
import type {
  ConnectorPreflightResult,
  JsonValue,
  ProtocolErrorKind,
  TriggerPollItem,
  TriggerPollResult
} from '@vornrun/shared/connector-protocol'
import { localLaunchSpec } from './catalog'
import { getOrStartSdkClient, launchAuthFields, sdkIdOf, sessionGrantFor } from './mcp-clients'
import { SdkCallError } from './native-client'
import { installedPack } from './packs'
import { forEachPage } from './paging'
import { messageOf, type SdkClient } from './sdk-client'
import { toManifest } from './sdk-probe'
import {
  closeSessionCall,
  openSessionCall,
  sessionOutcome,
  type OpenSessionCall
} from './session-bridge'

/** The trigger a connection polls; absent on one made only for its actions. */
export function sdkTriggerOf(conn: SourceConnection): string {
  return String(conn.filters[SDK_FILTER_KEYS.trigger] ?? '').trim()
}

/** What a connection's connector declares: its installed pack, or the child a checkout or command runs. */
export async function sdkManifestFor(conn: SourceConnection): Promise<SdkConnectorManifest> {
  const sdkId = sdkIdOf(conn)
  const pack = sdkId && !localLaunchSpec(sdkId) ? installedPack(sdkId) : undefined
  if (pack) return pack
  return toManifest(await (await getOrStartSdkClient(conn)).manifest())
}

/** An action as the step editor draws it; its outputs become the fields a later step can name. */
export function sdkActionDef(action: SdkAction): ConnectorActionDef {
  const outputs = action.outputs ?? []
  return {
    type: action.type,
    label: action.label,
    ...(action.description && { description: action.description }),
    configFields: (action.inputs ?? []).map(actionInputField),
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

type Answered<T> = { value: T; sessionCalls?: ActionResult['sessionCalls'] }

/** Run a call on the connection's child, through its signed-in window when it has one; a failure reads the way an action's does. */
async function throughWindow<T>(
  conn: SourceConnection,
  run: (client: SdkClient, sessionCall: string | undefined) => Promise<T>
): Promise<Answered<T> | ActionResult> {
  let call: OpenSessionCall | undefined
  try {
    const client = await getOrStartSdkClient(conn)
    const grant = sessionGrantFor(conn.id)
    if (grant) call = openSessionCall(grant)
    const value = await run(client, call?.key)
    const sessionCalls = call ? closeSessionCall(call) : []
    return { value, ...(sessionCalls.length > 0 && { sessionCalls }) }
  } catch (err) {
    return call ? sessionOutcome(conn, call, failureOf(err)) : failureOf(err)
  }
}

export async function invokeSdkAction(
  conn: SourceConnection,
  action: string,
  args: Record<string, unknown>
): Promise<ActionResult> {
  const result = await throughWindow(conn, (client, sessionCall) =>
    client.action({
      action,
      args: args as Record<string, JsonValue>,
      ...(sessionCall && { sessionCall })
    })
  )
  if (!('value' in result)) return result
  return {
    success: true,
    output: result.value,
    ...(result.sessionCalls && { sessionCalls: result.sessionCalls })
  }
}

/** One page of a connection's trigger, read through its window. */
async function pollPage(
  conn: SourceConnection,
  trigger: string,
  cursor?: string
): Promise<TriggerPollResult> {
  const result = await throughWindow(conn, (client, sessionCall) =>
    client.poll({
      trigger,
      ...(cursor !== undefined && { cursor }),
      ...(sessionCall && { sessionCall })
    })
  )
  if ('value' in result) return result.value
  throw new Error(result.error ?? `Polling ${conn.name} failed`)
}

function toEvent(item: TriggerPollItem): TriggerEvent {
  return {
    id: item.externalId,
    type: CONNECTOR_POLL_EVENT,
    timestamp: item.updatedAt || new Date().toISOString(),
    data: { ...item }
  }
}

export async function pollSdkConnection(
  conn: SourceConnection,
  trigger: string,
  cursor?: string
): Promise<PollResult> {
  const page = await pollPage(conn, trigger, cursor)
  return {
    events: page.items.map(toEvent),
    ...(page.nextCursor !== undefined && { nextCursor: page.nextCursor }),
    hasMore: page.hasMore
  }
}

function toExternalItem(item: TriggerPollItem): ExternalItem {
  return {
    externalId: item.externalId,
    url: item.url ?? '',
    title: item.title ?? '',
    description: item.description ?? '',
    status: item.status ?? '',
    updatedAt: item.updatedAt || new Date().toISOString(),
    metadata: item
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
  await forEachPage(
    `Connection "${conn.name}"`,
    async (cursor) => {
      const page = await pollPage(conn, trigger, cursor)
      return { ...page, items: page.items.map(toExternalItem) }
    },
    visit
  )
}

export async function preflightSdkConnection(
  conn: SourceConnection
): Promise<ConnectorPreflightResult> {
  return (await getOrStartSdkClient(conn)).preflight()
}

export const sdkConnector: VornConnector = {
  id: SDK_CONNECTOR_ID,
  name: 'Connector package',
  icon: 'mcp',
  capabilities: ['actions', 'triggers', 'tasks'],
  // Made by installing a package, never added by hand.
  addable: false,

  runAction: invokeSdkAction,
  actionsFor: sdkConnectionActions,
  // A seeded workflow fires on the generic event and polls the connection's trigger; a template names the trigger itself.
  pollConnection(conn, event) {
    const trigger = event === CONNECTOR_POLL_EVENT ? sdkTriggerOf(conn) : event
    if (!trigger) return 'has no trigger to poll'
    return (cursor) => pollSdkConnection(conn, trigger, cursor)
  },
  backfill: backfillSdkConnection,
  preflight: preflightSdkConnection,

  describe(): ConnectorManifest {
    return {
      auth: launchAuthFields({
        command: {
          description: 'Executable that starts the connector, when it is not an installed pack.'
        }
      }),
      triggers: [
        {
          type: CONNECTOR_POLL_EVENT,
          label: 'Poll',
          description: "Polls the connection's trigger on a schedule and fires once per new item.",
          configFields: [],
          defaultIntervalMs: 300_000
        }
      ],
      // Per connection, from the connector's own manifest; see `actionsFor`.
      actions: []
    }
  }
}
