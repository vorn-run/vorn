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
  type ProtocolMethod,
  type ProtocolMethods,
  type TriggerPollParams,
  type TriggerPollResult,
  type VornHelloResult
} from '@vornrun/shared/connector-protocol'
import type { LaunchSpec } from './mcp-clients'
import {
  SdkCallError,
  isRecord,
  startNativeClient,
  type ChildExit,
  type NativeClient
} from './native-client'
import { startLegacyMcpSdkClient } from './sdk-legacy-mcp'
import log from '../logger'

/** One connector child, whichever protocol it speaks. */
export interface SdkClient {
  /** The native protocol agreed in `vorn/hello`, or `mcp` for a child that only speaks MCP. */
  readonly protocol: number | 'mcp'
  readonly hello?: VornHelloResult
  readonly exited: boolean
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

  get exited(): boolean {
    return this.client.exited
  }

  private async call<M extends ProtocolMethod>(
    method: M,
    params: ProtocolMethods[M]['params'],
    timeoutMs: number,
    valid: (result: Record<string, unknown>) => boolean,
    what: string
  ): Promise<ProtocolMethods[M]['result'] & Record<string, unknown>> {
    const result: unknown = await this.client.request(method, params, timeoutMs)
    if (!isRecord(result) || !valid(result)) {
      throw new SdkCallError(
        method,
        PROTOCOL_ERROR_CODES.connectorError,
        `${this.key} answered ${method} without ${what}`
      )
    }
    return result as ProtocolMethods[M]['result'] & Record<string, unknown>
  }

  private callTimeout(sessionCall?: string): number {
    return sessionCall ? this.timeouts.sessionCall : this.timeouts.call
  }

  manifest(): Promise<Record<string, unknown>> {
    return this.call(
      PROTOCOL_METHODS.manifest,
      {},
      this.timeouts.manifest,
      (r) => typeof r.id === 'string',
      'a manifest'
    )
  }

  async preflight(): Promise<ConnectorPreflightResult> {
    const result = await this.call(
      PROTOCOL_METHODS.preflight,
      {},
      this.timeouts.call,
      (r) => typeof r.ok === 'boolean' || r.ok === null,
      'an ok'
    )
    const message = typeof result.message === 'string' ? result.message : undefined
    return { ok: result.ok, ...(message && { message }) }
  }

  options(params: ConnectorOptionsParams): Promise<ConnectorOptionsResult> {
    return this.call(
      PROTOCOL_METHODS.options,
      params,
      this.callTimeout(params.sessionCall),
      (r) => Array.isArray(r.options),
      'options'
    )
  }

  poll(params: TriggerPollParams): Promise<TriggerPollResult> {
    return this.call(
      PROTOCOL_METHODS.poll,
      params,
      this.callTimeout(params.sessionCall),
      (r) => Array.isArray(r.items) && typeof r.hasMore === 'boolean',
      'a page of items'
    )
  }

  action(params: ActionRunParams): Promise<ActionRunResult> {
    return this.call(
      PROTOCOL_METHODS.action,
      params,
      this.callTimeout(params.sessionCall),
      () => true,
      'an output object'
    )
  }

  footer(params: ExtensionFooterParams): Promise<ExtensionFooterResult> {
    return this.call(
      PROTOCOL_METHODS.footer,
      params,
      this.timeouts.call,
      (r) => Array.isArray(r.items),
      'items'
    )
  }

  async handler(params: ExtensionHandlerParams): Promise<ExtensionHandlerResult> {
    const result = await this.call(
      PROTOCOL_METHODS.handler,
      params,
      this.timeouts.call,
      () => true,
      'an object'
    )
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
