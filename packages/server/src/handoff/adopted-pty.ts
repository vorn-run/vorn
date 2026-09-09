import fs from 'node:fs'
import tty from 'node:tty'
import log from '../logger'
import { resizeFd } from './native'

/** The part of a pty this server uses, so an inherited one can be the other implementation. */
export interface ManagedPty {
  readonly pid: number
  write(data: string): void
  resize(cols: number, rows: number): void
  kill(signal?: string): void
  onData(listener: (data: string) => void): { dispose(): void }
  onExit(listener: (event: { exitCode: number; signal?: number }) => void): { dispose(): void }
  /** A paused master is a paused reader: the kernel keeps the bytes for whoever reads next. */
  pause(): void
  resume(): void
}

/**
 * A pty master this process inherited rather than opened.
 * The program holds the slave end and never learns which process reads the master.
 */
export class AdoptedPty implements ManagedPty {
  readonly pid: number
  /** Public so a handoff can happen twice; node-pty exposes the same property. */
  readonly fd: number
  private readonly reader: tty.ReadStream
  private readonly dataListeners = new Set<(data: string) => void>()
  private readonly exitListeners = new Set<(event: { exitCode: number; signal?: number }) => void>()
  /** Output read before anything was listening. Delivered to the first listener. */
  private pending: string[] = []
  private readonly writeQueue: Array<{ buffer: Buffer; offset: number }> = []
  private writeImmediate: NodeJS.Immediate | undefined
  private ended = false

  constructor(fd: number, pid: number) {
    this.fd = fd
    this.pid = pid
    this.reader = new tty.ReadStream(fd)
    this.reader.setEncoding('utf8')
    this.reader.on('data', (chunk: string | Buffer) => {
      const data = typeof chunk === 'string' ? chunk : chunk.toString('utf8')
      // Attaching this handler starts the flow, and the pause is a turn of the loop away.
      if (this.dataListeners.size === 0) {
        this.pending.push(data)
        return
      }
      for (const listener of this.dataListeners) listener(data)
    })
    // A master reports the far side going away as EIO on some platforms and EOF on others.
    this.reader.on('end', () => this.finish())
    this.reader.on('error', (err: NodeJS.ErrnoException) => {
      // A non-blocking master says "nothing yet" as an error, and node-pty's own
      // reader notes it arrives twice on startup. Treating it as the far side
      // going away would end a healthy terminal the moment it was adopted --
      // and `onExit` removes the session's history on its way past.
      if (err.code === 'EAGAIN') return
      if (err.code === 'EIO') return this.finish()
      log.warn({ err, pid: this.pid }, '[handoff] read error on an adopted pty')
      this.finish()
    })
  }

  /** Exit code 0 is a stand-in: nothing can waitpid for a program that reparented to init. */
  private finish(): void {
    if (this.ended) return
    this.ended = true
    clearImmediate(this.writeImmediate)
    this.writeImmediate = undefined
    this.writeQueue.length = 0
    this.pending = []
    try {
      this.reader.destroy()
    } catch {
      // Already gone; the listeners below are what matter.
    }
    for (const listener of this.exitListeners) listener({ exitCode: 0 })
  }

  /** Queued, because a non-blocking master answers a full buffer with EAGAIN. */
  write(data: string): void {
    if (this.ended) return
    const buffer = Buffer.from(data, 'utf8')
    if (buffer.byteLength === 0) return
    this.writeQueue.push({ buffer, offset: 0 })
    if (this.writeQueue.length === 1) this.drain()
  }

  private drain(): void {
    this.writeImmediate = undefined
    const task = this.writeQueue[0]
    if (!task || this.ended) return
    fs.write(this.fd, task.buffer, task.offset, (err, written) => {
      if (err) {
        if ((err as NodeJS.ErrnoException).code === 'EAGAIN') {
          this.writeImmediate = setImmediate(() => this.drain())
          return
        }
        // EIO is the far side closing between the queue and the write.
        if ((err as NodeJS.ErrnoException).code !== 'EIO') {
          log.warn({ err, pid: this.pid }, '[handoff] write failed on an adopted pty')
        }
        this.writeQueue.length = 0
        return
      }
      task.offset += written
      if (task.offset >= task.buffer.byteLength) this.writeQueue.shift()
      if (this.writeQueue.length > 0) this.drain()
    })
  }

  resize(cols: number, rows: number): void {
    if (this.ended) return
    resizeFd(this.fd, cols, rows)
  }

  /** By pid: there is no child handle, but it is the same process and the same user. */
  kill(signal: string = 'SIGHUP'): void {
    try {
      process.kill(this.pid, signal as NodeJS.Signals)
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code
      if (code !== 'ESRCH') {
        log.warn({ err, pid: this.pid }, '[handoff] could not signal an adopted pty')
      }
    }
    this.finish()
  }

  onData(listener: (data: string) => void): { dispose(): void } {
    const first = this.dataListeners.size === 0
    this.dataListeners.add(listener)
    if (first && this.pending.length) {
      const held = this.pending
      this.pending = []
      // Synchronously: this output is older than anything live, and reordering a
      // terminal's own history is worse than delivering it late.
      for (const data of held) listener(data)
    }
    return { dispose: () => this.dataListeners.delete(listener) }
  }

  onExit(listener: (event: { exitCode: number; signal?: number }) => void): { dispose(): void } {
    this.exitListeners.add(listener)
    return { dispose: () => this.exitListeners.delete(listener) }
  }

  pause(): void {
    this.reader.pause()
  }

  resume(): void {
    this.reader.resume()
  }
}
