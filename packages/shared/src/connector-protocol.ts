// Mirrors @vornrun/connector-sdk's protocol.ts, which the published SDK cannot import from here.

// ---- protocol 1 ----

/** The protocol this build speaks when it opens with `vorn/hello`. */
export const PROTOCOL_VERSION = 1

/** Every protocol this build can speak, offered in `vorn/hello`. */
export const SUPPORTED_PROTOCOLS: readonly number[] = [PROTOCOL_VERSION]

/** A line longer than this ends the child rather than being buffered. */
export const MAX_FRAME_BYTES = 16 * 1024 * 1024

export const PROTOCOL_METHODS = {
  hello: 'vorn/hello',
  manifest: 'connector/manifest',
  preflight: 'connector/preflight',
  options: 'connector/options',
  poll: 'trigger/poll',
  action: 'action/run',
  footer: 'extension/footer',
  handler: 'extension/handler'
} as const

export const PROTOCOL_ERROR_CODES = {
  methodNotFound: -32601,
  invalidParams: -32602,
  connectorError: -32000,
  unsupportedProtocol: -32001,
  beforeHello: -32002
} as const

export const PROTOCOL_ERROR_KINDS = [
  'validation',
  'app-offline',
  'signed-out',
  'upstream',
  'internal'
] as const

export type ProtocolErrorKind = (typeof PROTOCOL_ERROR_KINDS)[number]

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue }

export interface VornHelloParams {
  protocols: number[]
  host: { name: string; version: string }
}

export interface VornHelloResult {
  protocol: number
  sdk: { name: string; version: string }
  connector: { id: string; version: string; kind: 'connector' | 'extension' }
}

export type ConnectorManifestParams = Record<string, never>

/** The manifest the SDK writes to `manifest.json`, with the protocol it speaks; the host validates the rest. */
export interface ConnectorManifestResult {
  protocol: number
  id: string
  name: string
  version: string
  kind?: 'connector' | 'extension'
}

export type ConnectorPreflightParams = Record<string, never>

/** `ok: null` when the connector declares no preflight, which is not the same as passing one. */
export interface ConnectorPreflightResult {
  ok: boolean | null
  message?: string
}

export interface ConnectorOptionsParams {
  name: string
  sessionCall?: string
}

export interface ConnectorOptionsResult {
  options: Array<{ value: string; label?: string }>
}

export interface TriggerPollParams {
  trigger: string
  cursor?: string
  since?: string
  limit?: number
  sessionCall?: string
}

export interface TriggerPollItem {
  externalId: string
  title: string
  url: string
  description: string
  status: string
  labels: string[]
  updatedAt: string
  assignee?: string
  [key: string]: unknown
}

export interface TriggerPollResult {
  items: TriggerPollItem[]
  nextCursor?: string
  hasMore: boolean
}

export interface ActionRunParams {
  action: string
  args: Record<string, JsonValue>
  sessionCall?: string
}

export type ActionRunResult = Record<string, unknown>

export interface ExtensionFooterParams {
  footer: string
  sessionId: string
  worktreePath: string
  agent: string
}

export interface ExtensionFooterResult {
  items: Array<{ label: string; value: string; tone?: 'default' | 'ok' | 'danger'; href?: string }>
}

export interface ExtensionHandlerParams {
  handler: string
  sessionId: string
  worktreePath: string
  agent: string
  url: string
}

export interface ExtensionHandlerResult {
  openPane?: string
}

export interface ProtocolMethods {
  'vorn/hello': { params: VornHelloParams; result: VornHelloResult }
  'connector/manifest': { params: ConnectorManifestParams; result: ConnectorManifestResult }
  'connector/preflight': { params: ConnectorPreflightParams; result: ConnectorPreflightResult }
  'connector/options': { params: ConnectorOptionsParams; result: ConnectorOptionsResult }
  'trigger/poll': { params: TriggerPollParams; result: TriggerPollResult }
  'action/run': { params: ActionRunParams; result: ActionRunResult }
  'extension/footer': { params: ExtensionFooterParams; result: ExtensionFooterResult }
  'extension/handler': { params: ExtensionHandlerParams; result: ExtensionHandlerResult }
}

export type ProtocolMethod = keyof ProtocolMethods

export interface ProtocolErrorData {
  kind: ProtocolErrorKind
  retryable?: boolean
  field?: string
}

export interface ProtocolError {
  code: number
  message: string
  data?: ProtocolErrorData
}

export interface ProtocolRequest<M extends ProtocolMethod = ProtocolMethod> {
  jsonrpc: '2.0'
  id: number
  method: M
  params: ProtocolMethods[M]['params']
}

export type ProtocolResponse<M extends ProtocolMethod = ProtocolMethod> =
  | { jsonrpc: '2.0'; id: number; result: ProtocolMethods[M]['result'] }
  | { jsonrpc: '2.0'; id: number; error: ProtocolError }
