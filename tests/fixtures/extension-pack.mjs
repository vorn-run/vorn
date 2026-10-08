// An extension pack's child: its footer reports who it is and what its own bridge answered.
import process from 'node:process'
import readline from 'node:readline'

const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`)
const host = process.env.VORN_EXTENSION_HOST
const token = process.env.VORN_EXTENSION_TOKEN

async function bridged(sessionId) {
  const res = await globalThis.fetch(`${host}/status`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ sessionId })
  })
  return String(res.status)
}

readline.createInterface({ input: process.stdin }).on('line', async (line) => {
  const { id, method, params } = JSON.parse(line)
  if (id === undefined) return
  if (method === 'vorn/hello') return send({ jsonrpc: '2.0', id, result: { protocol: 1 } })
  if (method === 'extension/footer') {
    const items = [
      { label: 'pid', value: String(process.pid) },
      { label: 'token', value: token },
      { label: 'host', value: host },
      { label: 'bridge', value: await bridged(params.sessionId) }
    ]
    return send({ jsonrpc: '2.0', id, result: { items } })
  }
  if (method === 'extension/handler') {
    return send({
      jsonrpc: '2.0',
      id,
      result: { openPane: params.url.includes('12') ? 'page' : '' }
    })
  }
  send({ jsonrpc: '2.0', id, error: { code: -32601, message: `no ${method}` } })
})
