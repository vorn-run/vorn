import type { ClientArgs } from '../client-args'
import { ensureServer } from './autostart'
import { isPlain } from './output'
import { socketTransport, type RpcTransport } from './transport'

export interface CliDeps {
  /** Normal output. Log lines go to stderr (see `logger.ts`), so this is safe to pipe. */
  write(text: string): void
  /** Errors, notices and confirmations. Never data. */
  writeErr(text: string): void
  /** Defaults to the socket client. A test passes a fake and needs no server. */
  rpc?: RpcTransport
  /** Defaults to starting a server. A test passes a stub and spawns nothing. */
  ensureServer?: typeof ensureServer
  /** Defaults to whether stdout is a terminal. */
  isTty?: boolean
}

/** Everything a client command needs, with the production wiring already chosen. */
export interface ClientContext {
  write(text: string): void
  writeErr(text: string): void
  rpc: RpcTransport
  args: ClientArgs
  /** No colour: piped, redirected, or asked to go without. */
  plain: boolean
  /** A server to talk to, started if there is not one. False when it could not be. */
  server(): Promise<boolean>
}

/** `--timeout` applies to every call a command makes, so it is applied once, here. */
function withTimeout(rpc: RpcTransport, timeoutMs: number | undefined): RpcTransport {
  if (timeoutMs === undefined) return rpc
  return {
    call(method, params) {
      return rpc.call(method, params, timeoutMs)
    },
    notify(method, params) {
      return rpc.notify(method, params)
    },
    isRunning: () => rpc.isRunning()
  }
}

export function clientContext(deps: CliDeps, args: ClientArgs): ClientContext {
  const rpc = withTimeout(deps.rpc ?? socketTransport, args.timeoutMs)
  const start = deps.ensureServer ?? ensureServer
  return {
    write: deps.write,
    writeErr: deps.writeErr,
    rpc,
    args,
    plain: isPlain(deps.isTty ?? Boolean(process.stdout.isTTY)),
    server: () => start(rpc, deps.writeErr, args.dataDir)
  }
}
