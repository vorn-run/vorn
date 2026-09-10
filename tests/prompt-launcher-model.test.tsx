// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const state = vi.hoisted(() => ({
  config: { projects: [{ name: 'Project', path: '/project' }] },
  addTerminal: vi.fn(),
  isNewAgentDialogOpen: false
}))
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (select: (value: typeof state) => unknown) => select(state)
}))
vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({ status: { codex: true } })
}))
vi.mock('../src/renderer/hooks/useLaunchSettings', () => ({
  useLaunchSettings: () => ({
    selectedProject: 'Project',
    selectedAgent: 'codex',
    worktreeMode: 'project-root',
    selectedBranch: '',
    activeProjectPath: '',
    filteredProjects: [],
    persist: vi.fn(),
    reset: vi.fn()
  })
}))
vi.mock('../src/renderer/hooks/usePreferredModel', () => ({
  usePreferredModel: () => ({ model: 'chosen', setModel: vi.fn() })
}))
import { PromptLauncher } from '../src/renderer/components/PromptLauncher'

beforeEach(() => {
  Object.defineProperty(window, 'api', {
    configurable: true,
    writable: true,
    value: { createTerminal: vi.fn().mockResolvedValue({ id: 'new-session' }) }
  })
})
afterEach(cleanup)

it('launches the selected model without overriding advanced arguments', async () => {
  render(<PromptLauncher mode="inline" />)
  const input = screen.getByPlaceholderText('Describe your task...')
  fireEvent.change(input, { target: { value: 'hello' } })
  fireEvent.keyDown(input, { key: 'Enter' })
  await waitFor(() =>
    expect(window.api.createTerminal).toHaveBeenCalledWith(
      expect.objectContaining({ model: 'chosen', agentType: 'codex', initialPrompt: 'hello' })
    )
  )
  expect(vi.mocked(window.api.createTerminal).mock.calls[0][0].args).toBeUndefined()
})

it('shows an actionable model launch error', async () => {
  vi.mocked(window.api.createTerminal).mockRejectedValue(
    new Error('Move flags out of the agent command')
  )
  render(<PromptLauncher mode="inline" />)
  fireEvent.keyDown(screen.getByPlaceholderText('Describe your task...'), { key: 'Enter' })
  expect(await screen.findByRole('alert')).toHaveTextContent('Move flags out of the agent command')
})
