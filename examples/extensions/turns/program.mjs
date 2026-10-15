// Turns: counts each agent's turns from the session events Vorn sends and
// publishes the rows its templates draw. One JSON-RPC message per line.
import process from 'node:process'
import readline from 'node:readline'

const KEEP = 8
const sessions = new Map()
let finished = []
let plan = 'free'
const plans = () =>
  ['free', 'team'].map((id) => ({
    id,
    name: id === 'free' ? 'Free plan' : 'Team plan',
    selected: id === plan
  }))
const account = { id: 'turns-account', name: 'Turns account', signed_in: false }
const prefs = new Map([['shells', { id: 'shells', label: 'Count shells too', on: true }]])
let asked = 0
const waiting = new Map()

const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)
const publish = (collection, ops) => send({ method: 'host.publish', params: { collection, ops } })
const upsert = (collection, rows) =>
  publish(
    collection,
    rows.map((row) => ({ upsert: row }))
  )

function call(method, args) {
  asked += 1
  const id = `h${asked}`
  send({ id, method: 'host.call', params: { method, args } })
  return new Promise((resolve) => waiting.set(id, resolve))
}

const clock = () => new Date().toTimeString().slice(0, 5)

const row = (s) => ({
  id: s.id,
  name: s.name,
  status: s.status,
  turns: s.turns,
  last: s.last
})

async function names() {
  const { result } = await call('sessions.list', {})
  for (const s of Array.isArray(result) ? result : []) {
    const known = sessions.get(s.id)
    if (known && s.name) known.name = s.name
  }
}

async function onEvents(batch) {
  const touched = new Set()
  for (const { name, data } of batch) {
    const id = data.sessionId
    if (name === 'session.created') {
      if (data.kind === 'shell' && !prefs.get('shells').on) continue
      sessions.set(id, { id, name: data.agent || 'shell', status: 'running', turns: 0, last: '' })
      touched.add(id)
    } else if (name === 'session.status') {
      const s = sessions.get(id)
      if (!s) continue
      if (s.status === 'running' && (data.status === 'idle' || data.status === 'waiting')) {
        s.turns += 1
        s.last = clock()
        finished = [
          { id: `${id}-${s.turns}`, session: id, name: s.name, turn: s.turns, at: s.last },
          ...finished
        ].slice(0, KEEP)
      }
      s.status = data.status
      touched.add(id)
    } else if (name === 'session.exited') {
      sessions.delete(id)
      publish('sessions', [{ delete: id }])
    }
  }
  if (touched.size === 0) return
  await names()
  upsert(
    'sessions',
    [...touched].filter((id) => sessions.has(id)).map((id) => row(sessions.get(id)))
  )
  for (const f of finished) f.name = sessions.get(f.session)?.name ?? f.name
  publish('finished', ['clear', ...finished.map((f) => ({ upsert: f }))])
}

function handle(method, args) {
  switch (method) {
    case 'reset': {
      const s = sessions.get(args.id ?? args.sessionId)
      if (!s) return { toast: 'That session is gone' }
      s.turns = 0
      s.last = ''
      upsert('sessions', [row(s)])
      return null
    }
    case 'ask-clear':
      return { open: 'clear' }
    case 'clear':
      finished = []
      publish('finished', ['clear'])
      return null
    case 'plan':
      plan = args.id
      upsert('plans', plans())
      return null
    case 'key':
      return null
    case 'signin':
      // The window's target; Vorn opens it and tells the connection when it is signed in.
      return {
        name: account.name,
        browser: { signInUrl: 'https://example.com/sign-in', origins: ['https://example.com'] }
      }
    case 'pref': {
      const p = prefs.get(args.id)
      if (!p) return null
      p.on = args.value === true
      upsert('prefs', [p])
      return null
    }
    default:
      return undefined
  }
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const message = JSON.parse(line)
  const { id, method, params } = message
  if (method === undefined) {
    const reply = waiting.get(id)
    waiting.delete(id)
    return reply?.(message)
  }
  if (method === 'guest.init') {
    upsert('prefs', [...prefs.values()])
    upsert('plans', plans())
    upsert('account', [account])
    publish('finished', ['clear'])
    return send({ id, result: null })
  }
  if (method === 'guest.on-event') return void onEvents(params.batch)
  if (method === 'guest.handle') {
    const result = handle(params.method, params.args ?? {})
    if (result === undefined) {
      return send({
        id,
        error: {
          code: -32000,
          message: `Turns has no action ${params.method}`,
          data: { code: 'unknown-action' }
        }
      })
    }
    return send({ id, result })
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: `no ${method}` } })
})
