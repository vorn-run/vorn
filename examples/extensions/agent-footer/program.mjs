// Agent footer: keeps one row per agent session from the events Vorn sends
// (the model it started with, how many tools its hooks reported, the last
// one) and publishes them for the card footer template. One JSON-RPC
// message per line.
import process from 'node:process'
import readline from 'node:readline'

const rows = new Map()

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)
const publish = (ops) => send({ method: 'host.publish', params: { collection: 'sessions', ops } })

function onEvents(batch) {
  const ops = []
  for (const { name, data } of batch) {
    const id = data.sessionId
    if (name === 'session.created' && data.agent && data.agent !== 'shell') {
      const row = { id, model: data.model ?? '', tool: '', tools: 0 }
      rows.set(id, row)
      ops.push({ upsert: row })
    } else if (name === 'session.exited' && rows.delete(id)) {
      ops.push({ delete: id })
    } else if (name === 'agent.hook' && data.event === 'PreToolUse') {
      const row = rows.get(id)
      if (!row) continue
      row.tools += 1
      if (data.tool) row.tool = data.tool
      ops.push({ upsert: row })
    }
  }
  if (ops.length) publish(ops)
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, method, params } = JSON.parse(line)
  if (method === 'guest.init') return send({ id, result: null })
  if (method === 'guest.on-event') return onEvents(params.batch)
  // It has no actions; a call only starts it, so it hears the next session start.
  if (method === 'guest.handle') return send({ id, result: null })
  if (id !== undefined) {
    send({
      id,
      error: { code: -32000, message: `no ${method}`, data: { code: 'unknown-action' } }
    })
  }
})
