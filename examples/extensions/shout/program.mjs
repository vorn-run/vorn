// Shout: a workflow step. Vorn calls `workflow.node.shout` with the step's
// config (templates already expanded) and its run's context; the answer is
// the step's state. One JSON-RPC message per line.
import process from 'node:process'
import readline from 'node:readline'

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)

function handle(method, args) {
  if (method !== 'workflow.node.shout') return undefined
  const text = String(args.config?.text ?? '')
  if (!text) return { status: 'error', error: 'Shout has nothing to shout' }
  const times = Math.min(Math.max(Number(args.config?.times ?? 1), 1), 3)
  const loud = Array(times).fill(text.toUpperCase()).join(' ')
  return { status: 'success', output: loud, logs: `${args.node.label}: ${loud}` }
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, method, params } = JSON.parse(line)
  if (method === 'guest.init') return send({ id, result: null })
  if (method === 'guest.handle') {
    const result = handle(params.method, params.args ?? {})
    if (result === undefined) {
      return send({
        id,
        error: {
          code: -32000,
          message: `Shout has no action ${params.method}`,
          data: { code: 'unknown-action' }
        }
      })
    }
    return send({ id, result })
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: `no ${method}` } })
})
