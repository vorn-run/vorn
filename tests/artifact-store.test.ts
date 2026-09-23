import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  initTestDatabase,
  insertArtifact,
  getArtifact,
  listArtifacts,
  addArtifactVersion,
  listArtifactVersions,
  insertArtifactComment,
  updateArtifactComment,
  deleteArtifactComment,
  listArtifactComments,
  sendArtifactDrafts,
  unansweredBatchId,
  listArtifactIds
} from '../packages/server/src/database'
import {
  readVersionBody,
  versionFile,
  writeVersionBody
} from '../packages/server/src/artifacts/bodies'
import { sweepArtifacts } from '../packages/server/src/artifacts/service'

let teardown: () => void
let dataDir: string

beforeEach(() => {
  teardown = initTestDatabase()
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-artifacts-'))
})

afterEach(() => {
  teardown()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

const figures = () =>
  insertArtifact({
    kind: 'page',
    title: 'Figures for the triage article',
    sessionId: 's1',
    projectName: 'triage'
  }).artifact

const quote = { kind: 'quote' as const, quote: 'Opus 2/9', prefix: 'with ', suffix: '.' }

describe('artifact store', () => {
  it('numbers versions in order and moves the latest with them', () => {
    const a = figures()
    expect(a.latestVersion).toBe(0)
    addArtifactVersion(a.id, 'agent')
    addArtifactVersion(a.id, 'user')
    expect(getArtifact(a.id)!.latestVersion).toBe(2)
    expect(listArtifactVersions(a.id).map((v) => [v.version, v.author])).toEqual([
      [1, 'agent'],
      [2, 'user']
    ])
  })

  it('lists a session or project, newest first', () => {
    const a = figures()
    insertArtifact({ kind: 'doc', title: 'Other', sessionId: 's2', projectName: 'elsewhere' })
    expect(listArtifacts({ sessionId: 's1' }).map((x) => x.id)).toEqual([a.id])
    expect(listArtifacts({ projectName: 'triage' }).map((x) => x.id)).toEqual([a.id])
    expect(listArtifacts()).toHaveLength(2)
  })

  it('edits and deletes drafts but leaves sent comments alone', () => {
    const a = figures()
    addArtifactVersion(a.id, 'agent')
    const c = insertArtifactComment({ artifactId: a.id, version: 1, anchor: quote, body: 'Lead' })
    expect(updateArtifactComment(c.id, { body: 'This is the headline.' })!.body).toBe(
      'This is the headline.'
    )
    sendArtifactDrafts(a.id)
    expect(updateArtifactComment(c.id, { body: 'changed' })).toBeNull()
    expect(deleteArtifactComment(c.id)).toBe(false)
  })

  it('seals every draft into one batch that the next version answers', () => {
    const a = figures()
    addArtifactVersion(a.id, 'agent')
    insertArtifactComment({ artifactId: a.id, version: 1, anchor: quote, body: 'one' })
    insertArtifactComment({ artifactId: a.id, version: 1, anchor: null, body: 'two' })
    const batch = sendArtifactDrafts(a.id)!
    expect(batch.comments.map((c) => c.state)).toEqual(['sent', 'sent'])
    expect(sendArtifactDrafts(a.id)).toBeNull()
    expect(unansweredBatchId(a.id)).toBe(batch.batchId)
    addArtifactVersion(a.id, 'agent', batch.batchId)
    expect(unansweredBatchId(a.id)).toBeUndefined()
    expect(listArtifactComments(a.id, { batchId: batch.batchId })).toHaveLength(2)
  })

  it('keeps a doc as Markdown and a page as HTML', () => {
    writeVersionBody(dataDir, 'a1', 1, 'doc', '# Title')
    expect(versionFile(dataDir, 'a1', 1, 'doc').endsWith('1.md')).toBe(true)
    expect(readVersionBody(dataDir, 'a1', 1, 'doc')).toBe('# Title')
    expect(readVersionBody(dataDir, 'a1', 2, 'page')).toBeNull()
  })

  it('sweeps artifacts past retention and their files', () => {
    const a = figures()
    writeVersionBody(dataDir, a.id, 1, 'page', '<p>hi</p>')
    writeVersionBody(dataDir, 'orphan', 1, 'page', '<p>gone</p>')
    sweepArtifacts(dataDir)
    expect(listArtifactIds()).toEqual([a.id])
    expect(fs.existsSync(path.join(dataDir, 'artifacts', 'orphan'))).toBe(false)
    sweepArtifacts(dataDir, Date.now() + 91 * 24 * 60 * 60 * 1000)
    expect(listArtifactIds()).toEqual([])
    expect(fs.existsSync(path.join(dataDir, 'artifacts', a.id))).toBe(false)
  })
})
