// A child that speaks the native connector protocol; argv[2] picks how it behaves.
import { Buffer } from 'node:buffer'
import process from 'node:process'
import { setInterval, setTimeout } from 'node:timers'

const mode = process.argv[2] ?? 'normal'

const HELLO = {
  protocol: 1,
  sdk: { name: '@vornrun/connector-sdk', version: '0.0.0-test' },
  connector: { id: 'native-fixture', version: '1.0.0', kind: 'connector' }
}

function write(line) {
  if (mode === 'garbage') {
    process.stdout.write('not json\n[1,2]\n{"jsonrpc":"2.0","id":999,"result":{}}\n')
    process.stdout.write('{"jsonrpc":"2.0","method":"log","params":{}}\n')
  }
  if (mode !== 'split') {
    process.stdout.write(`${line}\n`)
    return
  }
  const bytes = Buffer.from(`${line}\r\n\n`, 'utf8')
  let at = 0
  const next = () => {
    if (at >= bytes.length) return
    process.stdout.write(bytes.subarray(at, at + 1))
    at += 1
    setTimeout(next, 0)
  }
  next()
}

const answer = (id, result) => write(JSON.stringify({ jsonrpc: '2.0', id, result }))
const fail = (id, code, message, data) =>
  write(JSON.stringify({ jsonrpc: '2.0', id, error: { code, message, ...(data && { data }) } }))

function handle({ id, method, params = {} }) {
  if (mode === 'silent') return
  if (mode === 'mcp-only') return fail(id, -32601, 'Method not found')
  if (method === 'vorn/hello') {
    if (mode === 'hello-error') {
      return fail(id, -32000, 'the fixture refused to start', {
        kind: 'internal',
        retryable: false
      })
    }
    if (mode === 'hello-unsupported') return fail(id, -32001, 'this connector speaks protocol 2')
    return answer(id, HELLO)
  }
  if (mode === 'oversized') {
    process.stdout.write(Buffer.alloc(Number(process.argv[3] ?? 4097), 'x'))
    return
  }
  if (mode === 'exit-mid-call' && method === 'action/run') {
    process.stderr.write('Error: lost the upstream\n')
    process.exit(7)
  }
  switch (method) {
    case 'connector/manifest':
      return answer(id, {
        protocol: 1,
        id: 'native-fixture',
        name: 'Native fixture ✓ é',
        version: '1.0.0'
      })
    case 'connector/preflight':
      return answer(id, { ok: true, message: 'ready' })
    case 'connector/options':
      return answer(id, { options: [{ value: 'a', label: 'A' }] })
    case 'trigger/poll':
      if (params.trigger === 'malformed') return answer(id, { items: 'none' })
      return answer(id, {
        items: [
          {
            externalId: '1',
            title: 'One',
            url: '',
            description: '',
            status: 'open',
            labels: [],
            updatedAt: '2026-09-13T00:00:00.000Z',
            cursor: params.cursor ?? null
          }
        ],
        hasMore: false
      })
    case 'action/run': {
      const args = params.args ?? {}
      if (args.fail) {
        return fail(id, -32000, 'the upstream said no', {
          kind: 'upstream',
          retryable: true,
          field: 'title'
        })
      }
      if (typeof args.env === 'string') return answer(id, { value: process.env[args.env] ?? null })
      const reply = () => answer(id, { echo: args, sessionCall: params.sessionCall ?? null })
      const delay = mode === 'slow' ? 400 : args.delayMs
      if (typeof delay === 'number') setTimeout(reply, delay)
      else reply()
      return
    }
    case 'extension/footer':
      return answer(id, { items: [{ label: 'Branch', value: 'main' }] })
    case 'extension/handler':
      return answer(id, { openPane: 'details' })
    default:
      return fail(id, -32601, 'Method not found')
  }
}

if (mode === 'crash-on-start') {
  process.stderr.write('starting\nError: the fixture could not start\n')
  process.exit(3)
}
if (mode === 'stderr-flood') {
  for (let line = 1; line <= 100; line += 1) process.stderr.write(`line ${line}\n`)
}
if (mode === 'stubborn') {
  process.on('SIGTERM', () => {})
  setInterval(() => {}, 1000)
}

let buffered = ''
process.stdin.setEncoding('utf8')
process.stdin.on('data', (chunk) => {
  buffered += chunk
  let at
  while ((at = buffered.indexOf('\n')) !== -1) {
    const line = buffered.slice(0, at)
    buffered = buffered.slice(at + 1)
    if (line.trim() !== '') handle(JSON.parse(line))
  }
})
process.stdin.on('end', () => {
  if (mode !== 'stubborn') process.exit(0)
})
