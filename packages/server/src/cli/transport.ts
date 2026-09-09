import type { RequestMethods } from '@vornrun/shared/protocol'
import { rpcCall, rpcNotify, isServerRunning } from '../rpc-client'

/**
 * How a client command reaches the server.
 *
 * An interface rather than the module itself, so a test drives every command
 * without a socket, a port file or a credential — the same reason `runCli`
 * takes its output sinks as arguments.
 */
export interface RpcTransport {
  call<M extends keyof RequestMethods>(
    method: M,
    params?: RequestMethods[M]['params'],
    timeoutMs?: number
  ): Promise<RequestMethods[M]['result']>
  notify(method: string, params?: unknown): Promise<void>
  /** Whether a server is reachable right now, without opening a socket. */
  isRunning(): boolean
}

export const socketTransport: RpcTransport = {
  call(method, params, timeoutMs) {
    return rpcCall(method, params, timeoutMs)
  },
  notify(method, params) {
    return rpcNotify(method, params)
  },
  isRunning: isServerRunning
}
