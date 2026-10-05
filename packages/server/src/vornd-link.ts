import { EventEmitter } from 'node:events'
import type { WebSocket } from 'ws'
import type { RecordCursor } from '@vornrun/shared/types'
import log from './logger'

/**
 * vornd as this server's process backend: the link.
 *
 * With the Native daemon switch on, the desktop starts vornd with this server's
 * credential, and vornd opens a socket to this server and claims it with
 * `vornd:identify`. From then on the server starts its terminals and piped
 * agents through vornd (and so through the session holder, which outlives the
 * app and this process) instead of spawning them itself, and hears back every
 * record they produce and every effect vornd's analysis reports.
 *
 * One link at a time. A newer identify replaces the older one: that is a vornd
 * restarted by the app, and the old socket is already on its way out.
 *
 * Nothing here knows about terminals. `vornd-process.ts` is a process over the
 * link; `pty-manager` and `headless-manager` choose it when `linked()` is true.
 */

/** The link protocol this server speaks. vornd sends it with its identify call. */
export const VORND_LINK_PROTOCOL = 1

/** How long a call over the link may take before it is given up. */
const CALL_TIMEOUT_MS = 15_000

/** One record as vornd sends it, in its session's log. */
export interface LinkRecord {
  epoch: number
  rseq: number
  offset: number
  /** Output bytes, base64. */
  data?: string
  resize?: [number, number]
  /** Bytes lost before this point. */
  gap?: number
  exit?: { code: number | null; signal: number | null }
}

/** One effect, named the same way by every replay of the same records. */
export interface LinkEffect {
  id: string
  /** `session:epoch:rseq:index`: the effect_id receivers deduplicate by. */
  effect: string
  epoch: number
  rseq: number
  index: number
  kind: 'bell' | 'clipboard' | 'notify' | 'cwd' | 'status' | 'exit'
  title?: string
  body?: string
  cwd?: string
  status?: number
  code?: number | null
  signal?: number | null
}

/** A session vornd's session holder has, as `vornd:list` reports it. */
export interface ListedSession {
  id: string
  kind: 'pty' | 'piped'
  pid: number
  epoch: number
  state: string | null
  /** Where vornd's own terminal for it stands. */
  cursor: RecordCursor | null
  cols: number | null
  rows: number | null
  exited: { code: number | null; signal: number | null } | null
}

export interface Listing {
  connected: boolean
  sessions: ListedSession[]
  /** The last few sessions that ended, with how. */
  ended: Array<{ id: string; exited: { code: number | null; signal: number | null } }>
}

/** What a session spawned over the link needs. */
export interface LinkSpawn {
  id: string
  argv: string[]
  cwd: string
  env: Record<string, string>
  cols?: number
  rows?: number
  piped?: boolean
  /**
   * Run `argv` joined into one command line by the platform's shell, as
   * child_process's `shell: true` does: the arguments are already quoted for it.
   */
  shell?: boolean
  /** For a piped agent: written to its stdin, which is then closed. */
  stdin?: string
}

interface Pending {
  resolve(value: unknown): void
  reject(err: Error): void
  timer: ReturnType<typeof setTimeout>
}

/** The receiving end of one session's records and effects. */
export interface LinkReceiver {
  records(records: LinkRecord[]): void
  effect(effect: LinkEffect): void
}

class VorndLink extends EventEmitter {
  private socket: WebSocket | null = null
  private nextId = 0
  private pending = new Map<number, Pending>()
  private receivers = new Map<string, LinkReceiver>()

  /** Whether a vornd is linked now. */
  linked(): boolean {
    return this.socket !== null && this.socket.readyState === this.socket.OPEN
  }

  /** Whether `ws` is the linked vornd's socket, whose frames are the link's and no one else's. */
  owns(ws: WebSocket): boolean {
    return this.socket === ws
  }

  /**
   * Take `ws` as the link. Emits `up` once it is in place; whoever drives the
   * backend lists vornd's sessions then and asks it to follow.
   */
  attach(ws: WebSocket): void {
    const old = this.socket
    if (old && old !== ws) {
      this.detach(old, 'replaced by a newer vornd')
      try {
        old.close(1000, 'replaced')
      } catch {
        // Already gone.
      }
    }
    this.socket = ws
    ws.once('close', () => this.detach(ws, 'the link closed'))
    log.info('[vornd-link] vornd linked as the process backend')
    this.emit('up')
  }

  private detach(ws: WebSocket, why: string): void {
    if (this.socket !== ws) return
    this.socket = null
    for (const [, p] of this.pending) {
      clearTimeout(p.timer)
      p.reject(new Error(`vornd is not linked: ${why}`))
    }
    this.pending.clear()
    log.warn(`[vornd-link] ${why}`)
    this.emit('down', why)
  }

  /** A frame from the linked socket: an answer to a call, or a record or effect. */
  receive(frame: {
    id?: unknown
    method?: unknown
    params?: unknown
    result?: unknown
    error?: unknown
  }): void {
    if (frame.method === undefined) {
      if (typeof frame.id !== 'number') return
      const p = this.pending.get(frame.id)
      if (!p) return
      this.pending.delete(frame.id)
      clearTimeout(p.timer)
      const error = frame.error as { message?: unknown } | undefined
      if (error) p.reject(new Error(String(error.message ?? 'vornd refused')))
      else p.resolve(frame.result)
      return
    }
    // vornd connected to a session holder: what it holds is to be listed again.
    if (frame.method === 'vornd:held') {
      this.emit('held')
      return
    }
    const params = frame.params as { id?: unknown } | undefined
    const id = typeof params?.id === 'string' ? params.id : null
    if (!id) return
    if (frame.method === 'vornd:records') {
      const records = (params as { records?: unknown }).records
      if (Array.isArray(records)) this.receivers.get(id)?.records(records as LinkRecord[])
    } else if (frame.method === 'vornd:effect') {
      const effect = params as LinkEffect
      this.receivers.get(id)?.effect(effect)
      // Notifications belong to whoever shows them, not to one process.
      this.emit('effect', effect)
    }
  }

  /** Records and effects for session `id` go to `r` from now on. */
  register(id: string, r: LinkReceiver): void {
    this.receivers.set(id, r)
  }

  unregister(id: string, r: LinkReceiver): void {
    if (this.receivers.get(id) === r) this.receivers.delete(id)
  }

  call<T = unknown>(method: string, params: unknown): Promise<T> {
    const ws = this.socket
    if (!ws || !this.linked()) return Promise.reject(new Error('vornd is not linked'))
    const id = ++this.nextId
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`vornd did not answer ${method}`))
      }, CALL_TIMEOUT_MS)
      timer.unref?.()
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer })
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  /** A call nobody waits on. Dropped while no vornd is linked: input is at most once. */
  notify(method: string, params: unknown): boolean {
    const ws = this.socket
    if (!ws || !this.linked()) return false
    ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
    return true
  }

  list(): Promise<Listing> {
    return this.call<Listing>('vornd:list', {})
  }

  /** Records and effects, the ones held while nobody was linked first, from now on. */
  follow(): Promise<unknown> {
    return this.call('vornd:follow', {})
  }

  spawn(spec: LinkSpawn): Promise<{ id: string; pid: number; epoch: number }> {
    return this.call('vornd:spawn', spec)
  }

  /** Test seam: forget every link and receiver. */
  reset(): void {
    if (this.socket) this.detach(this.socket, 'reset')
    this.receivers.clear()
    this.removeAllListeners()
  }
}

export const vorndLink = new VorndLink()
export type { VorndLink }
