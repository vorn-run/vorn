// An extension child in the connector protocol, for the host's tests.
import path from 'node:path'
import process from 'node:process'
import readline from 'node:readline'

const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`)
const token = process.env.VORN_EXTENSION_TOKEN

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, method, params } = JSON.parse(line)
  if (method === 'vorn/hello') return send({ jsonrpc: '2.0', id, result: { protocol: 1 } })
  if (method === 'extension/footer') {
    if (params.footer === 'crash') {
      process.stderr.write('Error: asked to crash\n')
      process.exit(3)
    }
    if (params.footer === 'fail') {
      return send({ jsonrpc: '2.0', id, error: { code: -32000, message: 'no reading' } })
    }
    return send({
      jsonrpc: '2.0',
      id,
      result: {
        items: [
          { label: 'token', value: token },
          { label: 'host', value: process.env.VORN_EXTENSION_HOST },
          { label: 'cwd', value: path.basename(process.cwd()) },
          { label: 'session', value: params.sessionId }
        ]
      }
    })
  }
  if (method === 'extension/handler') {
    return send({
      jsonrpc: '2.0',
      id,
      result: { openPane: params.url.includes('pane') ? 'p' : '' }
    })
  }
  send({ jsonrpc: '2.0', id, error: { code: -32601, message: `no ${method}` } })
})
