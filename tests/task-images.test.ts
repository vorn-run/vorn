import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import {
  saveTaskImage,
  saveTaskImageFromBase64,
  getTaskImagePath,
  deleteTaskImage,
  cleanupTaskImages
} from '../packages/server/src/task-images'

let dataDir: string
const TASK = 'task-1'

/** A one-pixel PNG, so the bytes are a real image rather than a placeholder. */
const PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8AAAwAB/AL+iwAAAABJRU5ErkJggg=='

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-images-'))
  initDatabase(dataDir)
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

/** Write a file outside the images tree, as a real upload source would be. */
function sourceFile(name: string): string {
  const p = path.join(dataDir, name)
  fs.writeFileSync(p, Buffer.from(PNG_BASE64, 'base64'))
  return p
}

describe('accepted types', () => {
  it.each(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp'])('accepts %s', (ext) => {
    expect(() => saveTaskImage(TASK, sourceFile(`shot${ext}`))).not.toThrow()
  })

  /**
   * An SVG is a document, not a bitmap. Served as `image/svg+xml` from the app's
   * own origin, inline script inside it would run against that origin — where the
   * web client keeps its device token. No format Vorn needs is a document.
   */
  it('refuses .svg on the file path, which used to check nothing', () => {
    expect(() => saveTaskImage(TASK, sourceFile('payload.svg'))).toThrow(/Unsupported image type/)
  })

  it('refuses .svg on the base64 path too', () => {
    expect(() => saveTaskImageFromBase64(TASK, PNG_BASE64, 'payload.svg')).toThrow(
      /Unsupported image type/
    )
  })

  it.each(['.html', '.js', '.sh', '.exe'])('refuses %s', (ext) => {
    expect(() => saveTaskImage(TASK, sourceFile(`payload${ext}`))).toThrow(/Unsupported image type/)
  })

  it('judges the extension case-insensitively, so .SVG is not a way through', () => {
    expect(() => saveTaskImage(TASK, sourceFile('payload.SVG'))).toThrow(/Unsupported image type/)
  })
})

describe('stored names', () => {
  it('renames to an unguessable filename, keeping the extension', () => {
    const filename = saveTaskImage(TASK, sourceFile('holiday.png'))

    // The URL is the capability: both the task id and this name have to be known
    // to read the file back, and only the extension survives from the original.
    expect(filename).not.toContain('holiday')
    expect(filename.endsWith('.png')).toBe(true)
    expect(filename.replace('.png', '')).toHaveLength(36) // uuid v4
  })

  it('gives two uploads of the same file different names', () => {
    const a = saveTaskImage(TASK, sourceFile('same.png'))
    const b = saveTaskImage(TASK, sourceFile('same.png'))
    expect(a).not.toBe(b)
  })
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
    const filename = saveTaskImage(TASK, sourceFile('ok.png'))
    const resolved = getTaskImagePath(TASK, filename)

    expect(resolved.startsWith(path.join(dataDir, 'task-images'))).toBe(true)
    expect(fs.existsSync(resolved)).toBe(true)
  })
})

describe('removal', () => {
  it('deletes one image and leaves its siblings', () => {
    const keep = saveTaskImage(TASK, sourceFile('keep.png'))
    const drop = saveTaskImage(TASK, sourceFile('drop.png'))

    deleteTaskImage(TASK, drop)

    expect(fs.existsSync(getTaskImagePath(TASK, drop))).toBe(false)
    expect(fs.existsSync(getTaskImagePath(TASK, keep))).toBe(true)
  })

  it('clears a task’s whole directory', () => {
    saveTaskImage(TASK, sourceFile('a.png'))
    saveTaskImage(TASK, sourceFile('b.png'))

    cleanupTaskImages(TASK)

    expect(fs.existsSync(path.join(dataDir, 'task-images', TASK))).toBe(false)
  })
})
