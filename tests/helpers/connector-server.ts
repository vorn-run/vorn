import {
  createConnectorServer,
  type ConnectorServerOptions,
  type ProtocolError
} from '../../packages/connector-sdk/src/index'
import type { Connector } from '../../packages/connector-sdk/src/types'

export interface Greeted {
  /** Send one request and return the whole reply. */
  send(method: string, params?: unknown): Promise<unknown>
  /** Send one request and return its result, failing the test on an error reply. */
  call<T = Record<string, unknown>>(method: string, params?: unknown): Promise<T>
  /** Send one request and return its error, failing the test on a result. */
  fail(method: string, params?: unknown): Promise<ProtocolError>
}

/** A connector's server after the hello, driven one request at a time as Vorn drives it over stdio. */
export async function greeted(
  connector: Connector,
  options: ConnectorServerOptions = { config: {} }
): Promise<Greeted> {
  const server = createConnectorServer(connector, options)
  let id = 0
  const send = (method: string, params: unknown = {}) =>
    server.handle({ jsonrpc: '2.0', id: ++id, method, params })
  await send('vorn/hello', { protocols: [1], host: { name: 'test', version: '1' } })
  return {
    send,
    async call<T>(method: string, params?: unknown) {
      const reply = (await send(method, params)) as { result?: T; error?: ProtocolError }
      if (reply.error) throw new Error(`${method} failed: ${reply.error.message}`)
      return reply.result as T
    },
    async fail(method: string, params?: unknown) {
      const reply = (await send(method, params)) as { result?: unknown; error?: ProtocolError }
      if (!reply.error) throw new Error(`${method} answered ${JSON.stringify(reply.result)}`)
      return reply.error
    }
  }
}
