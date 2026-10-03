/**
 * The server's real output path, driven without a real PTY.
 *
 * Everything below goes through `PtyManager`'s own handlers: a fake pty is wired
 * with the same `setupPtyEvents` a spawned one gets, so its `onData` is the real
 * one -- `bufferData` plus `appendOutput` per raw chunk, then `flushBuffer`
 * feeding the client, the scrollback, the headless xterm and history. Copying
 * that logic here would measure the copy, and WP2-4 replace exactly that code,
 * so the bench has to call it rather than resemble it.
 *
 * The private members are reached through a cast. That is deliberate: these are
 * the methods the plan names as hotspots, and widening their visibility for a
 * benchmark would change the product for the sake of a measurement.
 */
import type { EventEmitter } from 'node:events'
import type { WebSocket } from 'ws'
import { ptyManager } from '../../packages/server/src/pty-manager'
import { ClientRegistry } from '../../packages/server/src/broadcast'
import type { ManagedPty } from '../../packages/server/src/handoff/adopted-pty'
import type { TerminalSession } from '@vornrun/shared/types'

interface PtyInternals extends Pick<EventEmitter, 'on' | 'off'> {
  sessions: Map<string, TerminalSession>
  ptys: Map<string, ManagedPty>
  appendOutput(id: string, data: string): void
  bufferData(id: string, data: string): void
  flushBuffer(id: string): void
  drainBuffer(id: string): void
  clearBuffer(id: string): void
  clearSessionTracking(id: string): void
  setupPtyEvents(id: string, pty: ManagedPty, cols: number, rows: number): void
  dataBuffers: Map<string, string>
}

export const pm = ptyManager as unknown as PtyInternals

/** A pty that is only a place to push bytes from. */
export class FakePty implements ManagedPty {
  readonly pid = 0
  private listeners: Array<(data: string) => void> = []
  write(): void {}
  resize(): void {}
  kill(): void {}
  pause(): void {}
  resume(): void {}
  onData(listener: (data: string) => void): { dispose(): void } {
    this.listeners.push(listener)
    return { dispose: () => (this.listeners = this.listeners.filter((l) => l !== listener)) }
  }
  onExit(): { dispose(): void } {
    return { dispose() {} }
  }
  /** What node-pty does when the kernel hands it a read. */
  emit(data: string): void {
    for (const l of this.listeners) l(data)
  }
}

export function addSession(
  id: string,
  opts: { cols?: number; rows?: number; agentType?: TerminalSession['agentType'] } = {}
): FakePty {
  const cols = opts.cols ?? 200
  const rows = opts.rows ?? 50
  const fake = new FakePty()
  pm.setupPtyEvents(id, fake, cols, rows)
  pm.ptys.set(id, fake)
  pm.sessions.set(id, {
    id,
    agentType: opts.agentType ?? 'claude',
    projectName: 'bench',
    projectPath: '/tmp/bench',
    status: 'running',
    createdAt: Date.now(),
    cols,
    rows,
    pid: 0
  } as TerminalSession)
  return fake
}

/** Only what `appendOutput` reads: a registered, non-shell session. No screen, no history. */
export function addAnalysisSession(id: string): void {
  pm.sessions.set(id, {
    id,
    agentType: 'claude',
    projectName: 'bench',
    projectPath: '/tmp/bench',
    status: 'running',
    createdAt: Date.now(),
    cols: 200,
    rows: 50,
    pid: 0
  } as TerminalSession)
}

export function removeSession(id: string): void {
  pm.clearBuffer(id)
  pm.clearSessionTracking(id)
  pm.sessions.delete(id)
  pm.ptys.delete(id)
}

/** A socket that counts what it is sent, standing in for the desktop's connection. */
export class CountingSocket {
  readonly OPEN = 1
  readyState = 1
  bytes = 0
  messages = 0
  send(data: Uint8Array | string): void {
    this.bytes += typeof data === 'string' ? Buffer.byteLength(data) : data.byteLength
    this.messages++
  }
}

/**
 * Wire `client-message` to a registry the way `register-methods.ts` does, with
 * `count` desktop-style clients (terminal output as binary frames).
 */
export function connectClients(count: number): { sockets: CountingSocket[]; disconnect(): void } {
  const registry = new ClientRegistry()
  const sockets: CountingSocket[] = []
  for (let i = 0; i < count; i++) {
    const ws = new CountingSocket()
    registry.add(ws as unknown as WebSocket)
    registry.setTopics(ws as unknown as WebSocket, undefined, true)
    sockets.push(ws)
  }
  const listener = (channel: string, payload: unknown): void => {
    const id = (payload as { id?: unknown } | null)?.id
    registry.broadcast(channel, payload, typeof id === 'string' ? id : undefined)
  }
  pm.on('client-message', listener)
  return { sockets, disconnect: () => pm.off('client-message', listener) }
}
