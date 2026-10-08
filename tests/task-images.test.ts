import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { getTaskImagePath } from '../packages/server/src/task-images'

let dataDir: string
const TASK = 'task-1'

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-images-'))
  initDatabase(dataDir)
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('path traversal', () => {
  it.each([
    ['a traversing task id', '../escape'],
    ['an absolute task id', '/etc'],
    ['a task id with a separator', 'a/b']
  ])('refuses %s', (_label, taskId) => {
    expect(() => getTaskImagePath(taskId, 'x.png')).toThrow(/Invalid taskId/)
  })

  it.each([
    ['a traversing filename', '../../vorn.db'],
    ['a dotfile', '.env'],
    ['a filename with a separator', 'a/b.png']
  ])('refuses %s', (_label, filename) => {
    expect(() => getTaskImagePath(TASK, filename)).toThrow(/Invalid filename/)
  })

  it('resolves an accepted name inside the images directory', () => {
    const resolved = getTaskImagePath(TASK, 'ok.png')
    expect(resolved).toBe(path.join(dataDir, 'task-images', TASK, 'ok.png'))
  })
})
