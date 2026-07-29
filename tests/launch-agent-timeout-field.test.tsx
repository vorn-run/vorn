// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, cleanup } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'api', {
    value: {
      detectIDEs: () => Promise.resolve([]),
      listShellExecutables: () => Promise.resolve([])
    },
    writable: true,
    configurable: true
  })
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({
    claude: true,
    copilot: true,
    codex: true,
    opencode: true,
    gemini: true
  })
}))

import { useAppStore } from '../src/renderer/stores'
import { LaunchAgentConfigForm } from '../src/renderer/components/workflow-editor/panels/LaunchAgentConfigForm'
import type { LaunchAgentConfig } from '../src/shared/types'

/**
 * The per-step timeout. Without a bound, a step whose agent never exits holds
 * its run open forever — which is what let one stuck agent block a workflow.
 */

const BASE = { headless: true, agentType: 'claude', prompt: 'do the thing' } as LaunchAgentConfig

function renderForm(config: Partial<LaunchAgentConfig> = {}) {
  const onChange = vi.fn()
  render(<LaunchAgentConfigForm config={{ ...BASE, ...config }} onChange={onChange} />)
  return onChange
}

/** The timeout input is the only number field on the form. */
function timeoutInput(): HTMLInputElement {
  return screen.getByPlaceholderText(/^\d+$/) as HTMLInputElement
}

beforeEach(() => {
  useAppStore.setState({ config: { defaults: {}, projects: [], tasks: [] } as never })
})
afterEach(() => cleanup())

describe('per-step timeout', () => {
  it('stores minutes as milliseconds', () => {
    const onChange = renderForm()
    fireEvent.change(timeoutInput(), { target: { value: '20' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ timeoutMs: 20 * 60_000 }))
  })

  it('shows an existing timeout in minutes', () => {
    renderForm({ timeoutMs: 45 * 60_000 })
    expect(timeoutInput().value).toBe('45')
  })

  it('clears back to the default when emptied', () => {
    // Blank means "use the setting", which is not the same as zero.
    const onChange = renderForm({ timeoutMs: 30 * 60_000 })
    fireEvent.change(timeoutInput(), { target: { value: '' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ timeoutMs: undefined }))
  })

  it('keeps zero, which means no limit', () => {
    const onChange = renderForm()
    fireEvent.change(timeoutInput(), { target: { value: '0' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ timeoutMs: 0 }))
  })

  it('ignores a negative timeout rather than storing one', () => {
    const onChange = renderForm()
    fireEvent.change(timeoutInput(), { target: { value: '-5' } })
    expect(onChange).not.toHaveBeenCalled()
  })

  it('warns that no limit means a stuck agent holds the run open', () => {
    renderForm({ timeoutMs: 0 })
    expect(screen.getByText(/No limit/)).toBeInTheDocument()
  })

  it('offers no timeout for a step that is not headless', () => {
    // Only headless steps are waited on by the engine.
    renderForm({ headless: false })
    expect(screen.queryByText('Timeout')).toBeNull()
  })
})
