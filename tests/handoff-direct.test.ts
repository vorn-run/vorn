import { describe, it, expect, afterEach } from 'vitest'
import { WebSocketServer, type WebSocket as ServerSocket } from 'ws'
import { HANDOFF_PROTOCOL_VERSION, type HandoffRequest } from '@vornrun/shared/protocol'
import { askForHandoff } from '../src/main/server/handoff-direct'

/**
 * The channel that has to work when the runtime protocol does not.
 *
 * A release that changes the wire format makes the incumbent unadoptable, and an
 * app that cannot adopt it never gets a bridge to ask for a handoff with. This
 * socket speaks only the frozen part: a bearer credential, one JSON-RPC frame,
 * one reply -- no greeting read, no capabilities, no version compared.
 */
const servers: WebSocketServer[] = []
afterEach(() => {
  for (const server of servers.splice(0)) server.close()
})

const request: HandoffRequest = {
  handoffVersion: HANDOFF_PROTOCOL_VERSION,
  exec: '/bin/true',
  args: [],
  env: {},
  cwd: '/tmp',
  appVersion: '9.9.9'
}

/** A server that greets in a protocol nobody here understands, then answers. */
function stub(behave: (socket: ServerSocket, frame: { id: number }) => void): Promise<string> {
  const server = new WebSocketServer({ port: 0 })
  servers.push(server)
  server.on('connection', (socket, req) => {
    if (req.headers.authorization !== 'Bearer secret') return socket.close()
    // Deliberately unreadable to this client: nothing here reads the greeting.
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'server:hello', params: { v: 999 } }))
    socket.on('message', (raw) => behave(socket, JSON.parse(String(raw))))
  })
  return new Promise((resolve) =>
    server.on('listening', () => {
      const address = server.address()
      resolve(`ws://127.0.0.1:${typeof address === 'object' && address ? address.port : 0}/ws`)
    })
  )
}

describe('asking for a handoff without adopting', () => {
  it('sends the request and returns the answer, ignoring the greeting', async () => {
    let sent: unknown = null
    const target = await stub((socket, frame) => {
      sent = frame
      socket.send(
        JSON.stringify({
          jsonrpc: '2.0',
          id: frame.id,
          result: { kind: 'handed-over', sessions: 3, pid: 77 }
        })
      )
    })

    const result = await askForHandoff(target, 'secret', request)
    expect(result).toEqual({ kind: 'handed-over', sessions: 3, pid: 77 })
    expect(sent).toMatchObject({ method: 'server:handoff', params: { appVersion: '9.9.9' } })
  })

  it('carries a refusal back rather than throwing', async () => {
    const target = await stub((socket, frame) => {
      socket.send(
        JSON.stringify({
          jsonrpc: '2.0',
          id: frame.id,
          result: { kind: 'declined', because: 'a terminal could not be described' }
        })
      )
    })
    await expect(askForHandoff(target, 'secret', request)).resolves.toEqual({
      kind: 'declined',
      because: 'a terminal could not be described'
    })
  })

  it('is not confused by frames that are not its reply', async () => {
    const target = await stub((socket, frame) => {
      socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'terminal:data', params: { id: 'x' } }))
      socket.send(JSON.stringify({ jsonrpc: '2.0', id: frame.id + 41, result: 'someone else' }))
      socket.send('not json at all')
      socket.send(
        JSON.stringify({
          jsonrpc: '2.0',
          id: frame.id,
          result: { kind: 'declined', because: 'ok' }
        })
      )
    })
    await expect(askForHandoff(target, 'secret', request)).resolves.toMatchObject({
      kind: 'declined'
    })
  })

  it('fails rather than hanging when the server refuses the credential', async () => {
    const target = await stub(() => {})
    await expect(askForHandoff(target, 'wrong', request)).rejects.toThrow()
  })

  it('treats a socket that closes before answering as a failure', async () => {
    // A successful handoff replies first and exits after; a close with no reply is
    // the far side having gone wrong, and must never read as success.
    const target = await stub((socket) => socket.close())
    await expect(askForHandoff(target, 'secret', request)).rejects.toThrow(/closed first/)
  })

  it('surfaces an error reply as an error', async () => {
    const target = await stub((socket, frame) => {
      socket.send(
        JSON.stringify({
          jsonrpc: '2.0',
          id: frame.id,
          error: { code: -32000, message: 'may only be called over the local endpoint' }
        })
      )
    })
    await expect(askForHandoff(target, 'secret', request)).rejects.toThrow(/local endpoint/)
  })
})
