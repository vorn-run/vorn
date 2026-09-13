import { EXTENSION_AGENTS, resolveConfig } from './define'
import { UnknownNameError, messageOf, protocolError } from './errors'
import { createExtensionHost } from './host'
import { lineReader } from './lines'
import { isRecord } from './post-receive'
import {
  MAX_FRAME_BYTES,
  PROTOCOL_ERROR_CODES,
  PROTOCOL_METHODS,
  PROTOCOL_VERSION,
  type ProtocolError,
  type ProtocolResponse,
  type VornHelloResult
} from './protocol'
import { runAction, runOptions, runPoll } from './runtime'
import { connectorManifest } from './setup'
import type {
  Connector,
  ConnectorConfig,
  ExtensionAgent,
  ExtensionContext,
  ExtensionHost
} from './types'

declare const __VORN_SDK_VERSION__: string | undefined

// Stamped by the build; a run from source has none.
const SDK_VERSION = typeof __VORN_SDK_VERSION__ === 'string' ? __VORN_SDK_VERSION__ : '0.0.0-source'

export interface ConnectorServerOptions {
  /** Resolved connector configuration. Defaults to reading `process.env`. */
  config?: ConnectorConfig
  now?: () => string
  /** The host an extension's contributions talk to; defaults to the bridge Vorn served. */
  host?(sessionId: string): ExtensionHost
}

export interface ConnectorServer {
  /** Answer one decoded message; a notification, or anything that is not a request, gets no answer. */
  handle(message: unknown): Promise<ProtocolResponse | undefined>
}

type Params = Record<string, unknown>

/** A request whose params do not fit the method. */
class InvalidParams extends Error {}

function text(params: Params, key: string): string {
  const value = params[key]
  if (typeof value !== 'string' || value === '') {
    throw new InvalidParams(`"${key}" must be a non-empty string`)
  }
  return value
}

function optionalText(params: Params, key: string): string | undefined {
  const value = params[key]
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'string') throw new InvalidParams(`"${key}" must be a string`)
  return value
}

function optionalNumber(params: Params, key: string): number | undefined {
  const value = params[key]
  if (value === undefined || value === null) return undefined
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new InvalidParams(`"${key}" must be a number`)
  }
  return value
}

function failure(id: number, error: ProtocolError): ProtocolResponse {
  return { jsonrpc: '2.0', id, error }
}

function refusal(id: number, code: number, message: string): ProtocolResponse {
  return failure(id, { code, message })
}

/** Answer Vorn's connector protocol for one connector, one decoded message at a time. */
export function createConnectorServer(
  connector: Connector,
  options: ConnectorServerOptions = {}
): ConnectorServer {
  // Resolved on first use, so a missing variable is an answer the user can read, not a dead child.
  let cached: ConnectorConfig | undefined = options.config
  const config = (): ConnectorConfig => (cached ??= resolveConfig(connector))
  const now = options.now
  let greeted = false

  const runtime = (params: Params) => ({
    config: config(),
    now,
    sessionCall: optionalText(params, 'sessionCall')
  })

  const hostFor = (sessionId: string): ExtensionHost =>
    options.host?.(sessionId) ?? createExtensionHost({ sessionId })

  const sessionContext = (params: Params): ExtensionContext => {
    const agent = text(params, 'agent')
    if (!(EXTENSION_AGENTS as string[]).includes(agent)) {
      throw new InvalidParams(`"agent" must be one of ${EXTENSION_AGENTS.join(', ')}`)
    }
    const sessionId = text(params, 'sessionId')
    return {
      sessionId,
      worktreePath: text(params, 'worktreePath'),
      agent: agent as ExtensionAgent,
      host: hostFor(sessionId),
      now: now ?? (() => new Date().toISOString())
    }
  }

  const methods: Record<string, (params: Params) => unknown> = {
    [PROTOCOL_METHODS.manifest]: () => connectorManifest(connector),

    // A throw is the connector saying "broken", which must not read as a passing check.
    [PROTOCOL_METHODS.preflight]: async () => {
      if (!connector.preflight) return { ok: null }
      try {
        return { ...(await connector.preflight()) }
      } catch (error) {
        return { ok: false, message: messageOf(error) }
      }
    },

    [PROTOCOL_METHODS.options]: async (params) => ({
      options: await runOptions(connector, text(params, 'name'), runtime(params))
    }),

    [PROTOCOL_METHODS.poll]: (params) =>
      runPoll(connector, text(params, 'trigger'), {
        ...runtime(params),
        cursor: optionalText(params, 'cursor'),
        since: optionalText(params, 'since'),
        limit: optionalNumber(params, 'limit')
      }),

    [PROTOCOL_METHODS.action]: (params) => {
      const action = text(params, 'action')
      const args = params.args ?? {}
      if (!isRecord(args)) throw new InvalidParams('"args" must be an object')
      return runAction(connector, action, args, runtime(params))
    },

    [PROTOCOL_METHODS.footer]: async (params) => {
      const id = text(params, 'footer')
      const footer = connector.contributes?.footers?.find((entry) => entry.id === id)
      if (!footer) throw new InvalidParams(`${connector.id} contributes no footer "${id}"`)
      return { items: await footer.run(sessionContext(params)) }
    },

    [PROTOCOL_METHODS.handler]: async (params) => {
      const id = text(params, 'handler')
      const handler = connector.contributes?.linkHandlers?.find((entry) => entry.id === id)
      if (!handler) throw new InvalidParams(`${connector.id} contributes no link handler "${id}"`)
      const context = { ...sessionContext(params), url: text(params, 'url') }
      return { ...((await handler.run(context)) ?? {}) }
    }
  }

  const hello = (id: number, params: Params): ProtocolResponse => {
    const offered = params.protocols
    if (!Array.isArray(offered)) {
      return refusal(id, PROTOCOL_ERROR_CODES.invalidParams, '"protocols" must be a list')
    }
    if (!offered.includes(PROTOCOL_VERSION)) {
      return refusal(
        id,
        PROTOCOL_ERROR_CODES.unsupportedProtocol,
        `Vorn offers connector protocol ${offered.join(', ') || 'none'}; ${connector.id} speaks ${PROTOCOL_VERSION}`
      )
    }
    greeted = true
    const result: VornHelloResult = {
      protocol: PROTOCOL_VERSION,
      sdk: { name: '@vornrun/connector-sdk', version: SDK_VERSION },
      connector: { id: connector.id, version: connector.version, kind: connector.kind }
    }
    return { jsonrpc: '2.0', id, result }
  }

  return {
    async handle(message) {
      if (!isRecord(message) || typeof message.id !== 'number') return undefined
      const { id, method } = message
      if (typeof method !== 'string') {
        return refusal(id, PROTOCOL_ERROR_CODES.invalidParams, 'A request names its method')
      }
      const params = message.params ?? {}
      if (!isRecord(params)) {
        return refusal(id, PROTOCOL_ERROR_CODES.invalidParams, '"params" must be an object')
      }
      if (method === PROTOCOL_METHODS.hello) return hello(id, params)
      const run = Object.hasOwn(methods, method) ? methods[method] : undefined
      if (!run) return refusal(id, PROTOCOL_ERROR_CODES.methodNotFound, 'Method not found')
      if (!greeted) {
        return refusal(
          id,
          PROTOCOL_ERROR_CODES.beforeHello,
          `Call ${PROTOCOL_METHODS.hello} before ${method}`
        )
      }
      try {
        return { jsonrpc: '2.0', id, result: await run(params) } as ProtocolResponse
      } catch (error) {
        if (error instanceof InvalidParams || error instanceof UnknownNameError) {
          return refusal(id, PROTOCOL_ERROR_CODES.invalidParams, error.message)
        }
        return failure(id, protocolError(error))
      }
    }
  }
}

/** One reply as one line, or an error in its place when the result cannot travel. */
function frame(response: ProtocolResponse): string {
  try {
    const line = JSON.stringify(response)
    if (Buffer.byteLength(line) > MAX_FRAME_BYTES) {
      throw new Error(`The answer is over ${MAX_FRAME_BYTES} bytes`)
    }
    return line
  } catch (error) {
    return JSON.stringify(failure(response.id, protocolError(error)))
  }
}

// A pack's generated entry and a connector's own entry both call this in one process; only the first serves.
const SERVING = Symbol.for('@vornrun/connector-sdk/serving')

/** Serve a connector on stdio. This is the one line a connector's bin needs. */
export async function serveConnector(
  connector: Connector,
  options: ConnectorServerOptions = {}
): Promise<void> {
  const shared = globalThis as { [SERVING]?: boolean }
  if (shared[SERVING]) return
  shared[SERVING] = true
  // stdout carries only replies, so anything else written there goes to stderr.
  const reply = process.stdout.write.bind(process.stdout) as typeof process.stdout.write
  process.stdout.write = process.stderr.write.bind(process.stderr) as typeof process.stdout.write
  const server = createConnectorServer(connector, options)
  let inFlight = 0
  let ended = false
  const finish = (): void => {
    if (ended && inFlight === 0) reply('', () => process.exit(0))
  }

  const onLine = (line: string): void => {
    if (line.trim() === '') return
    let message: unknown
    try {
      message = JSON.parse(line)
    } catch {
      process.stderr.write(`${connector.id}: skipped a line that is not JSON\n`)
      return
    }
    inFlight++
    void server
      .handle(message)
      .then((response) => {
        if (response) reply(`${frame(response)}\n`)
      })
      .finally(() => {
        inFlight--
        finish()
      })
  }

  process.stdin.on(
    'data',
    lineReader(onLine, () => {
      process.stderr.write(`${connector.id}: a message was over ${MAX_FRAME_BYTES} bytes\n`)
      process.exit(1)
    })
  )
  process.stdin.on('end', () => {
    ended = true
    finish()
  })
}
