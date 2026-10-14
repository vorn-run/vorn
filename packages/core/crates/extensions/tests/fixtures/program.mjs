// A program in the extension API, for the bridge's tests: guest.handle's
// method picks what it does, and `relay` asks Vorn whatever the test names.
import process from 'node:process'
import readline from 'node:readline'

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)
const waiting = new Map()
const events = []
let inited = null
let relayed = 0

function handle(id, method, args, ctx) {
  const answer = (result) => send({ id, result })
  switch (method) {
    case 'echo':
      return answer({ args, ctx, inited, pid: process.pid })
    case 'stall':
      return undefined
    case 'crash':
      process.stderr.write('Error: asked to crash\n')
      return process.exit(3)
    case 'events':
      return answer(events)
    case 'relay': {
      const notification = args.notification === true
      if (notification) {
        send({ method: args.method, params: args.params })
        return answer({ sent: true })
      }
      relayed += 1
      const rid = `r${relayed}`
      waiting.set(rid, (reply) => answer(reply))
      return send({ id: rid, method: args.method, params: args.params })
    }
    case 'publish':
      send({ method: 'host.publish', params: { collection: 'rows', ops: args.ops } })
      return answer({ ok: true })
    default:
      return send({
        id,
        error: { code: -32000, message: `no action ${method}`, data: { code: 'unknown-action' } }
      })
  }
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const message = JSON.parse(line)
  if (message.method === undefined) {
    const reply = waiting.get(message.id)
    waiting.delete(message.id)
    const { result, error } = message
    return reply?.(error ? { error } : { result: result ?? null })
  }
  const { id, method, params } = message
  if (method === 'guest.init') {
    inited = params.ctx
    return send({ id, result: null })
  }
  if (method === 'guest.on-event') return events.push(...params.batch)
  if (method === 'guest.handle') return handle(id, params.method, params.args, params.ctx)
  if (id !== undefined) send({ id, error: { code: -32601, message: `no ${method}` } })
})
