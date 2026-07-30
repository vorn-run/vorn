// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const state = vi.hoisted(() => ({
  running: null as { command: string | null; since: number } | null,
  listeners: new Set<() => void>()
}))

vi.mock('../src/renderer/lib/command-blocks', () => ({
  getRunningBlock: () => state.running,
  onCommandBlocksChange: (_id: string, cb: () => void) => {
    state.listeners.add(cb)
    return () => state.listeners.delete(cb)
  },
  formatDuration: (ms: number) => `${Math.round(ms / 1000)}s`
}))

import { RunningCommand } from '../src/renderer/components/RunningCommand'

/**
 * A block only appears once its command has finished. While one runs there was
 * nothing to say so, and a command that never exits — `cat` with no arguments
 * is the easy way to get one — looked exactly like a terminal that had stopped
 * responding, because everything typed afterwards went to its stdin.
 */

function emit(): void {
  act(() => {
    for (const cb of state.listeners) cb()
  })
}

beforeEach(() => {
  state.running = null
  state.listeners.clear()
})
afterEach(() => cleanup())

describe('RunningCommand', () => {
  it('draws nothing while the shell waits at its prompt', () => {
    const { container } = render(<RunningCommand terminalId="t" />)
    expect(container).toBeEmptyDOMElement()
  })

  it('names the command that is running', () => {
    state.running = { command: 'cat', since: Date.now() }
    render(<RunningCommand terminalId="t" />)
    expect(screen.getByText('cat')).toBeInTheDocument()
  })

  it('says how to interrupt it, since that is the way out', () => {
    state.running = { command: 'cat', since: Date.now() }
    render(<RunningCommand terminalId="t" />)
    expect(screen.getByText(/interrupt/)).toBeInTheDocument()
  })

  it('appears as soon as a command starts', () => {
    render(<RunningCommand terminalId="t" />)
    state.running = { command: 'yarn test', since: Date.now() }
    emit()
    expect(screen.getByText('yarn test')).toBeInTheDocument()
  })

  it('disappears once the command finishes', () => {
    state.running = { command: 'yarn test', since: Date.now() }
    const { container } = render(<RunningCommand terminalId="t" />)
    state.running = null
    emit()
    expect(container).toBeEmptyDOMElement()
  })

  it('still reports a command whose text the shell could not name', () => {
    // cmd.exe cannot report command text, but "something is running" is the
    // fact that matters here.
    state.running = { command: null, since: Date.now() }
    render(<RunningCommand terminalId="t" />)
    expect(screen.getByText('Running')).toBeInTheDocument()
  })

  it('shows elapsed time, which is what tells you it is stuck', async () => {
    // Measured off the clock in an effect, not during render — a render that
    // reads the clock is not idempotent.
    state.running = { command: 'cat', since: Date.now() - 12_000 }
    render(<RunningCommand terminalId="t" />)
    await waitFor(() => expect(screen.getByText('12s')).toBeInTheDocument())
  })
})
