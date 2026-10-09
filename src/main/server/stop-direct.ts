import WebSocket from 'ws'
import { createRequest } from '@vornrun/shared/protocol'
import log from '../logger'

/**
 * Asking a server to stop over a socket of its own, without adopting it.
 *
 * A server this app cannot speak to, because a release changed the wire
 * format, still understands a bearer credential and one `server:shutdown`
 * frame. Its terminals live in the session holder, so stopping it ends none.
 */

const TIMEOUT_MS = 15_000

/** Resolves once the server answered or hung up; rejects when it refused or could not be asked. */
export function askToStop(target: string, credential: string): Promise<void> {
  const socket = new WebSocket(target, { headers: { authorization: `Bearer ${credential}` } })
  return new Promise<void>((resolve, reject) => {
    const id = 1
    let opened = false
    const timer = setTimeout(() => finish(new Error('the server never answered')), TIMEOUT_MS)
    const finish = (err?: Error): void => {
      clearTimeout(timer)
      socket.removeAllListeners()
      socket.on('error', () => {})
      try {
        socket.close()
      } catch {
        // Already gone.
      }
      if (err) reject(err)
      else resolve()
    }
    socket.once('open', () => {
      opened = true
      socket.send(JSON.stringify(createRequest(id, 'server:shutdown', undefined)))
      log.info('[launcher] asked an older server to stop, without adopting it')
    })
    socket.on('message', (raw) => {
      let frame: { id?: unknown; error?: { message?: string } }
      try {
        frame = JSON.parse(String(raw))
      } catch {
        return
      }
      if (frame.id !== id) return
      finish(frame.error ? new Error(frame.error.message ?? 'refused') : undefined)
    })
    socket.once('close', (code) =>
      finish(
        opened && code !== 4001 && code !== 4002
          ? undefined
          : new Error('the server refused this app')
      )
    )
    socket.once('error', (err) => finish(err))
  })
}
