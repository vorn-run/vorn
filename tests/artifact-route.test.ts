import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Fastify, { type FastifyInstance } from 'fastify'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { registerArtifactRoute } from '../packages/server/src/artifact-route'
import {
  addArtifactVersion,
  initTestDatabase,
  insertArtifact
} from '../packages/server/src/database'
import { writeVersionBody } from '../packages/server/src/artifacts/bodies'
import { artifactPage } from '../packages/server/src/artifacts/service'
import { markdownToHtml } from '../packages/shared/src/markdown'

let dir: string
let app: FastifyInstance
let teardown: () => void

beforeEach(async () => {
  teardown = initTestDatabase()
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'artifact-route-'))
  app = Fastify()
  registerArtifactRoute(app, (id, version, token) => artifactPage(dir, id, version, token))
  await app.ready()
})

afterEach(async () => {
  await app.close()
  teardown()
  fs.rmSync(dir, { recursive: true, force: true })
})

const publish = (kind: 'page' | 'doc', body: string) => {
  const { artifact, token } = insertArtifact({
    kind,
    title: 'Step 7: Evaluate',
    sessionId: 's1',
    projectName: 'triage'
  })
  const { version } = addArtifactVersion(artifact.id, 'agent')
  writeVersionBody(dir, artifact.id, version, kind, body)
  return { id: artifact.id, token }
}

describe('serving an artifact', () => {
  it('serves a page to the holder of its token, sealed off from the network', async () => {
    const { id, token } = publish('page', '<h1>Figures</h1>')
    const res = await app.inject(`/artifact/${id}/1?t=${token}`)
    expect(res.statusCode).toBe(200)
    expect(res.body).toBe('<h1>Figures</h1>')
    const csp = String(res.headers['content-security-policy'])
    expect(csp).toContain("default-src 'none'")
    expect(csp).toContain('sandbox allow-scripts')
    expect(res.headers['cache-control']).toBe('no-store')
  })

  it('refuses a wrong token, a missing version, and a malformed one', async () => {
    const { id, token } = publish('page', '<p>x</p>')
    expect((await app.inject(`/artifact/${id}/1?t=nope`)).statusCode).toBe(404)
    expect((await app.inject(`/artifact/${id}/1`)).statusCode).toBe(404)
    expect((await app.inject(`/artifact/${id}/2?t=${token}`)).statusCode).toBe(404)
    expect((await app.inject(`/artifact/${id}/0?t=${token}`)).statusCode).toBe(404)
    expect((await app.inject(`/artifact/${id}/one?t=${token}`)).statusCode).toBe(404)
  })

  it('renders a doc from its Markdown in a reading page', async () => {
    const { id, token } = publish('doc', '# Fine-tuning\n\nOn **124** alerts.')
    const res = await app.inject(`/artifact/${id}/1?t=${token}`)
    expect(res.body).toContain('<title>Step 7: Evaluate</title>')
    expect(res.body).toContain('<h1>Fine-tuning</h1>')
    expect(res.body).toContain('<p>On <strong>124</strong> alerts.</p>')
  })
})

describe('markdownToHtml', () => {
  it('escapes raw HTML and drops links that are not web, mail or in-page', () => {
    expect(markdownToHtml('<script>alert(1)</script>')).toBe(
      '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>'
    )
    expect(markdownToHtml('[go](javascript:void0)')).toBe('<p>go</p>')
    expect(markdownToHtml('[go](data:text/html,x)')).toBe('<p>go</p>')
    expect(markdownToHtml('[notes](docs/notes.md)')).toBe(
      '<p><a href="docs/notes.md">notes</a></p>'
    )
    expect(markdownToHtml('[site](https://vorn.run)')).toBe(
      '<p><a href="https://vorn.run">site</a></p>'
    )
  })
})
