/**
 * A scratch repository with a working-tree diff of about 500 KB.
 *
 * A size that should add no measurable lag during a PTY burst, and the cap `getGitDiffText` truncates at, so the largest
 * diff the UI ever asks for.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const FILES = 200
const LINES = 120

function line(file: number, n: number, edited: boolean): string {
  const tag = edited ? 'next' : 'base'
  return `export const value_${file}_${n} = computeSomething('${tag}', ${n * file}, ${'x'.repeat(16)})\n`
}

export function makeRepo(): { dir: string; diffBytes: number; cleanup(): void } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-bench-git-'))
  const git = (...args: string[]): string =>
    execFileSync('git', args, {
      cwd: dir,
      encoding: 'utf-8',
      stdio: ['ignore', 'pipe', 'pipe'],
      maxBuffer: 64 * 1024 * 1024
    })
  git('init', '-q', '-b', 'main')
  git('config', 'user.email', 'bench@vorn.invalid')
  git('config', 'user.name', 'bench')
  git('config', 'commit.gpgsign', 'false')
  for (let f = 0; f < FILES; f++) {
    let body = ''
    for (let n = 0; n < LINES; n++) body += line(f, n, false)
    fs.mkdirSync(path.join(dir, 'src', `m${f % 10}`), { recursive: true })
    fs.writeFileSync(path.join(dir, 'src', `m${f % 10}`, `file-${f}.ts`), body)
  }
  git('add', '-A')
  git('commit', '-q', '-m', 'base')
  // Every fourth line of a fifth of the files, which lands near 500 KB of unified diff.
  for (let f = 0; f < FILES / 5; f++) {
    let body = ''
    for (let n = 0; n < LINES; n++) body += line(f, n, n % 4 === 0)
    fs.writeFileSync(path.join(dir, 'src', `m${f % 10}`, `file-${f}.ts`), body)
  }
  const diffBytes = Buffer.byteLength(git('diff', 'HEAD'))
  return { dir, diffBytes, cleanup: () => fs.rmSync(dir, { recursive: true, force: true }) }
}
