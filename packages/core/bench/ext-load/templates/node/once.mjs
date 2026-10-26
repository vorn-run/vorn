// A bench one-shot: one request in, one answer out.
import process from 'node:process'
import readline from 'node:readline'

const input = readline.createInterface({ input: process.stdin })
input.once('line', (line) => {
  const { id, params } = JSON.parse(line)
  const answer = { jsonrpc: '2.0', id, result: { method: params.method } }
  process.stdout.write(`${JSON.stringify(answer)}\n`)
  input.close()
})
