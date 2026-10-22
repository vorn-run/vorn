// A bench service: each call asks Vorn for its sessions, as a sidebar would.
import process from 'node:process'
import readline from 'node:readline'

const send = (m) => process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...m })}\n`)
const waiting = new Map()
let asked = 0
let heard = 0
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const m = JSON.parse(line)
  if (m.method === undefined) {
    const reply = waiting.get(m.id)
    waiting.delete(m.id)
    return reply?.(m)
  }
  if (m.method === 'guest.init') return send({ id: m.id, result: null })
  if (m.method === 'guest.on-event') {
    heard += m.params.batch.length
    return
  }
  if (m.method === 'guest.handle') {
    const rid = `r${++asked}`
    waiting.set(rid, (r) => {
      if (r.error) return send({ id: m.id, error: r.error })
      const bytes = JSON.stringify(r.result ?? null).length
      send({ id: m.id, result: { bytes, heard } })
    })
    return send({ id: rid, method: 'host.call', params: { method: 'sessions.list', args: {} } })
  }
  if (m.id !== undefined) send({ id: m.id, error: { code: -32601, message: `no ${m.method}` } })
})
