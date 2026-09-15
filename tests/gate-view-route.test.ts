import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Fastify, { type FastifyInstance } from 'fastify'
import { registerGateViewRoute } from '../packages/server/src/gate-view-route'
import {
  GATE_VIEW_MAX_BYTES,
  gateViewFile,
  publishGateView,
  sweepGateViews
} from '../packages/server/src/workflows/gate-views'

let dir: string
let app: FastifyInstance
let token = ''

beforeEach(async () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'gate-views-'))
  const page = publishGateView(dir, 'run-1', 'approve-1', 1, '<h1>Tuesday</h1><script>1</script>')
  token = 'token' in page ? page.token : ''
  app = Fastify()
  registerGateViewRoute(app, (runId, nodeId, t) =>
    t === token ? gateViewFile(dir, runId, nodeId, 1) : null
  )
  await app.ready()
})

afterEach(async () => {
  await app.close()
  fs.rmSync(dir, { recursive: true, force: true })
})

describe('serving a review page', () => {
  it('serves the page to the holder of its token, sealed off from everything', async () => {
    const res = await app.inject(`/gate-view/run-1/approve-1?t=${token}`)
    expect(res.statusCode).toBe(200)
    expect(res.body).toContain('<h1>Tuesday</h1>')
    expect(res.headers['content-type']).toBe('text/html; charset=utf-8')
    const csp = String(res.headers['content-security-policy'])
    expect(csp).toContain("default-src 'none'")
    expect(csp).toContain("script-src 'unsafe-inline'")
    expect(csp).toContain('img-src data: blob:')
    expect(csp).toContain('sandbox allow-scripts')
    expect(csp).not.toContain('allow-same-origin')
    expect(res.headers['x-content-type-options']).toBe('nosniff')
    expect(res.headers['referrer-policy']).toBe('no-referrer')
    expect(res.headers['cache-control']).toBe('no-store')
  })

  it('answers 404 without the token, or with another one', async () => {
    expect((await app.inject('/gate-view/run-1/approve-1')).statusCode).toBe(404)
    expect((await app.inject('/gate-view/run-1/approve-1?t=nope')).statusCode).toBe(404)
  })
})

describe('keeping a review page', () => {
  it('takes HTML text or a .html file, and a new token each time', () => {
    const file = path.join(dir, 'page.html')
    fs.writeFileSync(file, '<p>from a file</p>')
    const first = publishGateView(dir, 'run-2', 'gate', 1, `  ${file}  `)
    const second = publishGateView(dir, 'run-2', 'gate', 2, '<p>round two</p>')
    expect(fs.readFileSync(gateViewFile(dir, 'run-2', 'gate', 1), 'utf8')).toBe(
      '<p>from a file</p>'
    )
    expect(fs.readFileSync(gateViewFile(dir, 'run-2', 'gate', 2), 'utf8')).toBe('<p>round two</p>')
    expect('token' in first && 'token' in second && first.token !== second.token).toBe(true)
  })

  it('says why when there is no page to keep', () => {
    expect(publishGateView(dir, 'r', 'g', 1, '   ')).toEqual({
      error: 'The review page came out empty.'
    })
    expect(publishGateView(dir, 'r', 'g', 1, 'notes/today.md')).toMatchObject({
      error: expect.stringContaining('neither HTML nor a .html file')
    })
    expect(publishGateView(dir, 'r', 'g', 1, '/nowhere/page.html')).toMatchObject({
      error: expect.stringContaining('does not exist')
    })
    expect(
      publishGateView(dir, 'r', 'g', 1, `<p>${'x'.repeat(GATE_VIEW_MAX_BYTES)}</p>`)
    ).toMatchObject({ error: expect.stringContaining('the limit is 5 MB') })
  })

  it('drops the pages of runs no longer kept', () => {
    publishGateView(dir, 'run-gone', 'gate', 1, '<p>old</p>')
    sweepGateViews(dir, ['run-1'])
    expect(fs.existsSync(gateViewFile(dir, 'run-1', 'approve-1', 1))).toBe(true)
    expect(fs.existsSync(path.join(dir, 'gate-views', 'run-gone'))).toBe(false)
  })
})
