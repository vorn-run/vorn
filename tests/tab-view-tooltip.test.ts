import { describe, it, expect } from 'vitest'
import { buildTooltip } from '../packages/web/src/renderer/lib/tab-tooltip'

describe('buildTooltip (TabView tab hover)', () => {
  it('contains display name and status header', () => {
    const text = buildTooltip('fix-auth', 'running')
    expect(text).toMatch(/fix-auth/)
    expect(text).toMatch(/Running/)
  })

  it('adds branch line when provided', () => {
    const text = buildTooltip('fix-auth', 'waiting', 'feat/x', false, undefined)
    expect(text).toMatch(/Branch: feat\/x/)
  })

  it('marks branch as worktree when isWorktree is true', () => {
    const text = buildTooltip('fix-auth', 'running', 'feat/x', true)
    expect(text).toMatch(/Branch: feat\/x \(worktree\)/)
  })

  it('adds task line when taskTitle is set', () => {
    const text = buildTooltip('fix-auth', 'running', undefined, undefined, 'Add OAuth login')
    expect(text).toMatch(/Task: Add OAuth login/)
  })

  // Unified sessions panel: shells fold cwd/exit into the tab tooltip instead
  // of rendering a status bar under the terminal.
  it('adds shell Cwd line when shellCwd is provided', () => {
    const text = buildTooltip(
      'Shell 1',
      'running',
      undefined,
      undefined,
      undefined,
      '/home/user/vorn'
    )
    expect(text).toMatch(/Cwd: \/home\/user\/vorn/)
  })

  it('adds shell Exit line when shellExitCode is set (including 0)', () => {
    const exited = buildTooltip('Shell 1', 'idle', undefined, undefined, undefined, '/home/user', 0)
    expect(exited).toMatch(/Exit: 0/)

    const failed = buildTooltip(
      'Shell 1',
      'idle',
      undefined,
      undefined,
      undefined,
      '/home/user',
      127
    )
    expect(failed).toMatch(/Exit: 127/)
  })

  it('omits Cwd/Exit lines for agent tabs (no shell fields)', () => {
    const text = buildTooltip('fix-auth', 'running', 'feat/x')
    expect(text).not.toMatch(/Cwd:/)
    expect(text).not.toMatch(/Exit:/)
  })

  it('prefixes the heading with the project when projectName is provided', () => {
    const text = buildTooltip(
      'main',
      'running',
      'main',
      false,
      undefined,
      undefined,
      undefined,
      'vorn'
    )
    const firstLine = text.split('\n')[0]
    expect(firstLine).toBe('vorn / main — Running')
    expect(text).toMatch(/Branch: main/)
  })

  it('falls back to displayName-only heading when projectName is undefined', () => {
    const text = buildTooltip('fix-auth', 'running')
    const firstLine = text.split('\n')[0]
    expect(firstLine).toBe('fix-auth — Running')
    expect(firstLine).not.toMatch(/\//)
  })
})
