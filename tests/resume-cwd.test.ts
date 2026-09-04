import { describe, it, expect } from 'vitest'
import { resumeCwdFor } from '../packages/server/src/resume-cwd'

/** A filesystem with exactly these directories in it. */
const has =
  (...dirs: string[]) =>
  (at: string) =>
    dirs.includes(at)

const agent = { projectPath: '/repo', worktreePath: '/repo/.wt/feature' }
const shell = { ...agent, shellCwd: '/repo/.wt/feature/src' }

describe('where a session resumes into', () => {
  it('lands where it was when that is still there', () => {
    expect(resumeCwdFor(agent, has('/repo', '/repo/.wt/feature'))).toEqual({
      cwd: '/repo/.wt/feature'
    })
  })

  it('prefers the directory a shell reported over its worktree', () => {
    expect(resumeCwdFor(shell, has('/repo', '/repo/.wt/feature', '/repo/.wt/feature/src'))).toEqual(
      { cwd: '/repo/.wt/feature/src' }
    )
  })

  it('falls back to the project when the worktree is gone, and says so', () => {
    // The live bug: an agent's worktree was handed to the spawn unchecked, so a
    // worktree cleaned up after its branch merged was spawned into anyway.
    expect(resumeCwdFor(agent, has('/repo'))).toEqual({
      cwd: '/repo',
      fellBackFrom: '/repo/.wt/feature'
    })
  })

  it('does not report a fallback when there was nothing more specific to fall from', () => {
    expect(resumeCwdFor({ projectPath: '/repo' }, has('/repo'))).toEqual({ cwd: '/repo' })
  })

  it('refuses rather than spawning into nothing when the project is gone too', () => {
    expect(resumeCwdFor(agent, has())).toBeNull()
  })

  it('treats a path that is a file as gone', () => {
    // `isDirectory` answers false for a file, a broken link and a missing path
    // alike -- this only asks one question, so all three are the same answer.
    expect(resumeCwdFor(agent, (at) => at === '/repo')).toEqual({
      cwd: '/repo',
      fellBackFrom: '/repo/.wt/feature'
    })
  })
})
