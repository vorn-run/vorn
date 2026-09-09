import { describe, it, expect, beforeAll, afterAll } from 'vitest'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { getRepoRoot } from '../packages/server/src/git-utils'

let repo: string
let outside: string

beforeAll(() => {
  // realpath: /tmp is a symlink on macOS, and git answers with the resolved path.
  repo = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-repo-')))
  outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-plain-')))
  execFileSync('git', ['init', '-q'], { cwd: repo })
  fs.mkdirSync(path.join(repo, 'packages', 'server'), { recursive: true })
})

afterAll(() => {
  fs.rmSync(repo, { recursive: true, force: true })
  fs.rmSync(outside, { recursive: true, force: true })
})

describe('the repository a directory sits in', () => {
  it('is the root, asked from anywhere inside it', () => {
    expect(getRepoRoot(repo)).toBe(repo)
    expect(getRepoRoot(path.join(repo, 'packages', 'server'))).toBe(repo)
  })

  it('is nothing at all outside one, rather than an error', () => {
    expect(getRepoRoot(outside)).toBeNull()
  })
})
