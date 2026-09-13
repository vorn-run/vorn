// Opens a connector's child in the protocol it speaks: native after a `vorn/hello`, MCP when it has never heard of one.
import {
  PROTOCOL_ERROR_CODES,
  PROTOCOL_METHODS,
  SUPPORTED_PROTOCOLS,
  type ActionRunParams,
  type ActionRunResult,
  type ConnectorOptionsParams,
  type ConnectorOptionsResult,
  type ConnectorPreflightResult,
  type ExtensionFooterParams,
  type ExtensionFooterResult,
  type ExtensionHandlerParams,
  type ExtensionHandlerResult,
  type TriggerPollParams,
  type TriggerPollResult,
  type VornHelloResult
} from '@vornrun/shared/connector-protocol'
import type { LaunchSpec } from './mcp-clients'
import { SdkCallError, startNativeClient, type ChildExit, type NativeClient } from './native-client'
import { startLegacyMcpSdkClient } from './sdk-legacy-mcp'
import log from '../logger'

/** One connector child, whichever protocol it speaks. */
export interface SdkClient {
  /** The native protocol agreed in `vorn/hello`, or `mcp` for a child that only speaks MCP. */
  readonly protocol: number | 'mcp'
  readonly hello?: VornHelloResult
  manifest(): Promise<Record<string, unknown>>
  preflight(): Promise<ConnectorPreflightResult>
  options(params: ConnectorOptionsParams): Promise<ConnectorOptionsResult>
  poll(params: TriggerPollParams): Promise<TriggerPollResult>
  action(params: ActionRunParams): Promise<ActionRunResult>
  footer(params: ExtensionFooterParams): Promise<ExtensionFooterResult>
  handler(params: ExtensionHandlerParams): Promise<ExtensionHandlerResult>
  close(): Promise<void>
  onExit(listener: (exit: ChildExit) => void): void
}

export const SDK_TIMEOUTS_MS = {
  hello: 15_000,
  npxHello: 90_000,
  manifest: 15_000,
  call: 60_000,
  sessionCall: 120_000
} as const

export type SdkTimeouts = { -readonly [K in keyof typeof SDK_TIMEOUTS_MS]: number }

const NPX_NAMES = ['npx', 'npx.cmd', 'npx.exe']

/** npx may download the package before the child can say anything. */
export function helloTimeoutFor(command: string, timeouts: SdkTimeouts = SDK_TIMEOUTS_MS): number {
  const name = command.split(/[\\/]/).pop()?.toLowerCase() ?? ''
  return NPX_NAMES.includes(name) ? timeouts.npxHello : timeouts.hello
}

export type Detection = 'native' | 'mcp' | 'probe' | 'unsupported'

/** An installed pack names its protocol; a checkout or a stored command has to be asked. */
export function detectionFor(launch: Pick<LaunchSpec, 'source' | 'protocol'>): Detection {
  if (launch.source !== 'pack') return 'probe'
  if (launch.protocol === undefined) return 'mcp'
  return SUPPORTED_PROTOCOLS.includes(launch.protocol) ? 'native' : 'unsupported'
}

/** A child that could not be opened in any protocol this build speaks. */
export class SdkDetectionError extends Error {
  readonly reason: 'unsupported' | 'failed'

  constructor(reason: 'unsupported' | 'failed', message: string) {
    super(message)
    this.name = 'SdkDetectionError'
    this.reason = reason
  }
}

export interface SdkLaunch extends LaunchSpec {
  env: Record<string, string>
  cwd?: string
}

export interface ConnectSdkOptions {
  label: string
  key: string
  hostVersion?: string
  timeouts?: Partial<SdkTimeouts>
  startLegacy?: (launch: SdkLaunch) => Promise<SdkClient>
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value)

const messageOf = (err: unknown): string => (err instanceof Error ? err.message : String(err))

function needsNewerVorn(key: string, protocol?: number): SdkDetectionError {
  const spoken =
    protocol === undefined ? 'a newer connector protocol' : `connector protocol ${protocol}`
  return new SdkDetectionError('unsupported', `${key} speaks ${spoken}, which needs a newer Vorn`)
}

export async function connectSdkClient(
  launch: SdkLaunch,
  options: ConnectSdkOptions
): Promise<SdkClient> {
  const timeouts: SdkTimeouts = { ...SDK_TIMEOUTS_MS, ...options.timeouts }
  const detection = detectionFor(launch)
  if (detection === 'unsupported') throw needsNewerVorn(options.key, launch.protocol)
  const legacy = (): Promise<SdkClient> => {
    log.info(`[connectors] ${options.key}: legacy MCP protocol (${launch.source})`)
    return options.startLegacy
      ? options.startLegacy(launch)
      : startLegacyMcpSdkClient(launch, { label: options.label, key: options.key })
  }
  if (detection === 'mcp') return legacy()

  const native = await startNativeClient(launch, { label: options.label, key: options.key })
  let hello: unknown
  try {
    hello = await native.request(
      PROTOCOL_METHODS.hello,
      {
        protocols: [...SUPPORTED_PROTOCOLS],
        host: { name: 'vorn', version: options.hostVersion ?? 'unknown' }
      },
      helloTimeoutFor(launch.command, timeouts)
    )
  } catch (err) {
    await native.close()
    const code = err instanceof SdkCallError ? err.code : undefined
    // Only a child nobody vouched for may turn out to be MCP; a pack that says native and is not is broken.
    if (code === PROTOCOL_ERROR_CODES.methodNotFound && detection === 'probe') return legacy()
    if (code === PROTOCOL_ERROR_CODES.unsupportedProtocol) throw needsNewerVorn(options.key)
    throw new SdkDetectionError(
      'failed',
      `${options.key} did not answer vorn/hello: ${messageOf(err)}`
    )
  }

  const agreed = isRecord(hello) ? hello.protocol : undefined
  if (typeof agreed !== 'number' || !SUPPORTED_PROTOCOLS.includes(agreed)) {
    await native.close()
    if (typeof agreed === 'number') throw needsNewerVorn(options.key, agreed)
    throw new SdkDetectionError('failed', `${options.key} answered vorn/hello without a protocol`)
  }
  return new NativeSdkClient(native, hello as VornHelloResult, timeouts, options.key)
}

class NativeSdkClient implements SdkClient {
  readonly protocol: number
  readonly hello: VornHelloResult
  private readonly client: NativeClient
  private readonly timeouts: SdkTimeouts
  private readonly key: string

  constructor(client: NativeClient, hello: VornHelloResult, timeouts: SdkTimeouts, key: string) {
    this.client = client
    this.hello = hello
    this.protocol = hello.protocol
    this.timeouts = timeouts
    this.key = key
  }

  private malformed(method: string, what: string): SdkCallError {
    return new SdkCallError(
      method,
      PROTOCOL_ERROR_CODES.connectorError,
      `${this.key} answered ${method} without ${what}`
    )
  }

  private callTimeout(sessionCall?: string): number {
    return sessionCall ? this.timeouts.sessionCall : this.timeouts.call
  }

  async manifest(): Promise<Record<string, unknown>> {
    const method = PROTOCOL_METHODS.manifest
    const result: unknown = await this.client.request(method, {}, this.timeouts.manifest)
    if (!isRecord(result) || typeof result.id !== 'string') {
      throw this.malformed(method, 'a manifest')
    }
    return result
  }

  async preflight(): Promise<ConnectorPreflightResult> {
    const method = PROTOCOL_METHODS.preflight
    const result: unknown = await this.client.request(method, {}, this.timeouts.call)
    if (!isRecord(result) || (typeof result.ok !== 'boolean' && result.ok !== null)) {
      throw this.malformed(method, 'an ok')
    }
    const message = typeof result.message === 'string' ? result.message : undefined
    return { ok: result.ok, ...(message && { message }) }
  }

  async options(params: ConnectorOptionsParams): Promise<ConnectorOptionsResult> {
    const method = PROTOCOL_METHODS.options
    const result: unknown = await this.client.request(
      method,
      params,
      this.callTimeout(params.sessionCall)
    )
    if (!isRecord(result) || !Array.isArray(result.options)) throw this.malformed(method, 'options')
    return result as unknown as ConnectorOptionsResult
  }

  async poll(params: TriggerPollParams): Promise<TriggerPollResult> {
    const method = PROTOCOL_METHODS.poll
    const result: unknown = await this.client.request(
      method,
      params,
      this.callTimeout(params.sessionCall)
    )
    if (!isRecord(result) || !Array.isArray(result.items) || typeof result.hasMore !== 'boolean') {
      throw this.malformed(method, 'a page of items')
    }
    return result as unknown as TriggerPollResult
  }

  async action(params: ActionRunParams): Promise<ActionRunResult> {
    const method = PROTOCOL_METHODS.action
    const result: unknown = await this.client.request(
      method,
      params,
      this.callTimeout(params.sessionCall)
    )
    if (!isRecord(result)) throw this.malformed(method, 'an output object')
    return result
  }

  async footer(params: ExtensionFooterParams): Promise<ExtensionFooterResult> {
    const method = PROTOCOL_METHODS.footer
    const result: unknown = await this.client.request(method, params, this.timeouts.call)
    if (!isRecord(result) || !Array.isArray(result.items)) throw this.malformed(method, 'items')
    return result as unknown as ExtensionFooterResult
  }

  async handler(params: ExtensionHandlerParams): Promise<ExtensionHandlerResult> {
    const method = PROTOCOL_METHODS.handler
    const result: unknown = await this.client.request(method, params, this.timeouts.call)
    if (!isRecord(result)) throw this.malformed(method, 'an object')
    return typeof result.openPane === 'string' && result.openPane !== ''
      ? { openPane: result.openPane }
      : {}
  }

  close(): Promise<void> {
    return this.client.close()
  }

  onExit(listener: (exit: ChildExit) => void): void {
    this.client.onExit(listener)
  }
}
