// A connector in the connector protocol: polls a feed, reads one in an action.
import process from 'node:process'
import readline from 'node:readline'

if (process.argv.includes('--check')) {
  process.stdout.write('manifest ok\npoll ok\n2 checks passed\n')
  process.exit(0)
}

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, method, params = {} } = JSON.parse(line)
  const answer = (result) => send({ id, result })
  switch (method) {
    case 'vorn/hello':
      return answer({ protocol: 1 })
    case 'connector/manifest':
      return answer({ id: 'test.feeds', name: 'Feeds', protocol: 1, actions: [{ type: 'read' }] })
    case 'trigger/poll':
      return answer({
        items: [{ externalId: '1', title: 'First post', url: 'https://feeds.example.com/1' }],
        nextCursor: 'c1',
        hasMore: false,
        pid: process.pid
      })
    case 'action/run':
      if (params.action === 'crash') {
        process.stderr.write('Error: the feed went away\n')
        return process.exit(4)
      }
      if (params.action === 'hang') return undefined
      return answer({ read: params.args?.url ?? null, pid: process.pid })
    default:
      return send({ id, error: { code: -32601, message: `no ${method}` } })
  }
})
