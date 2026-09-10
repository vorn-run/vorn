// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { AgentModelCatalog } from '@vornrun/shared/agent-models'
import { ModelPicker } from '../src/renderer/components/ModelPicker'

afterEach(cleanup)

const listed: AgentModelCatalog = {
  choices: [
    { id: 'claude-sonnet-5', label: 'Sonnet 5' },
    { id: 'claude-opus-5', label: 'Opus 5' }
  ],
  status: 'ready',
  source: 'agent',
  fetchedAt: Date.now()
}

function api(list = vi.fn().mockResolvedValue(listed)) {
  Object.defineProperty(window, 'api', {
    configurable: true,
    writable: true,
    value: { listAgentModels: list }
  })
  return list
}

const trigger = () => screen.getByRole('button', { name: /^Model:/ })
const open = () => fireEvent.click(trigger())
const choose = (name: string) =>
  fireEvent.mouseDown(screen.getByRole('option', { name: new RegExp(name) }))

describe('ModelPicker', () => {
  it('renders nothing for an agent that takes no model', () => {
    api()
    const { container } = render(<ModelPicker agentType="gemini" onChange={vi.fn()} />)
    expect(container).toBeEmptyDOMElement()
  })

  it('asks for models only when opened, and lists them with the default first', async () => {
    const list = api()
    const onChange = vi.fn()
    render(<ModelPicker agentType="claude" projectPath="/p" onChange={onChange} />)
    expect(list).not.toHaveBeenCalled()
    expect(trigger()).toHaveTextContent('Default')

    open()
    await screen.findByRole('option', { name: /Sonnet 5/ })
    const options = screen.getAllByRole('option')
    expect(options[0]).toHaveTextContent('Agent default')
    expect(options[1]).toHaveTextContent('claude-sonnet-5')
    expect(list).toHaveBeenCalledWith(
      expect.objectContaining({ agentType: 'claude', projectPath: '/p', refresh: false })
    )
    expect(screen.getByText(/From claude/)).toBeInTheDocument()

    choose('Opus 5')
    expect(onChange).toHaveBeenCalledWith('claude-opus-5')
    await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument())
  })

  it('shows the chosen label on the trigger, and the default row clears it', async () => {
    api()
    const onChange = vi.fn()
    render(
      <ModelPicker agentType="claude" projectPath="/p" value="claude-opus-5" onChange={onChange} />
    )
    open()
    await screen.findByRole('option', { name: /Opus 5/ })
    expect(trigger()).toHaveTextContent('Opus 5')
    choose('Agent default')
    expect(onChange).toHaveBeenLastCalledWith(undefined)
  })

  it('takes a typed id on Enter, and refuses one that cannot be an argument', async () => {
    api()
    const onChange = vi.fn()
    render(<ModelPicker agentType="claude" projectPath="/p" onChange={onChange} />)
    open()
    await screen.findByRole('option', { name: /Sonnet 5/ })
    const field = screen.getByRole('textbox', { name: 'Model id' })

    fireEvent.change(field, { target: { value: '--bad' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.getByText(/leading dash/)).toBeInTheDocument()

    fireEvent.change(field, { target: { value: 'claude-sonnet-5[1m]' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(onChange).toHaveBeenLastCalledWith('claude-sonnet-5[1m]')
  })

  it('filters the rows as an id is typed, and picks the exact match on Enter', async () => {
    api()
    const onChange = vi.fn()
    render(<ModelPicker agentType="claude" projectPath="/p" onChange={onChange} />)
    open()
    await screen.findByRole('option', { name: /Sonnet 5/ })
    const field = screen.getByRole('textbox', { name: 'Model id' })
    fireEvent.change(field, { target: { value: 'opus' } })
    expect(screen.queryByRole('option', { name: /Sonnet 5/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('option', { name: /Agent default/ })).not.toBeInTheDocument()
    fireEvent.change(field, { target: { value: 'claude-opus-5' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(onChange).toHaveBeenLastCalledWith('claude-opus-5')
  })

  it('keeps a saved id that the list does not know, and says when the list is unavailable', async () => {
    api(vi.fn().mockRejectedValue(new Error('offline')))
    render(<ModelPicker agentType="claude" projectPath="/p" value="saved-id" onChange={vi.fn()} />)
    open()
    await screen.findByText('Could not list models.')
    expect(screen.getByRole('option', { name: /saved-id/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Refresh models' })).not.toBeInTheDocument()
  })

  it('asks again on refresh', async () => {
    const list = api()
    render(<ModelPicker agentType="claude" projectPath="/p" onChange={vi.fn()} />)
    open()
    await screen.findByRole('button', { name: 'Refresh models' })
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Refresh models' }))
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2))
    expect(list).toHaveBeenLastCalledWith(expect.objectContaining({ refresh: true }))
  })

  it('groups OpenCode models by provider', async () => {
    api(
      vi.fn().mockResolvedValue({
        choices: [
          { id: 'anthropic/claude-sonnet-5', label: 'anthropic/claude-sonnet-5' },
          { id: 'openai/gpt-5.4', label: 'openai/gpt-5.4' }
        ],
        status: 'ready',
        source: 'agent',
        fetchedAt: Date.now()
      })
    )
    render(<ModelPicker agentType="opencode" projectPath="/p" onChange={vi.fn()} />)
    open()
    await screen.findByRole('option', { name: /gpt-5.4/ })
    expect(screen.getByText('anthropic')).toBeInTheDocument()
    expect(screen.getByText('openai')).toBeInTheDocument()
  })

  it('says where a built-in list came from, without a refresh', async () => {
    api(
      vi.fn().mockResolvedValue({
        choices: [{ id: 'auto', label: 'Auto' }],
        status: 'ready',
        source: 'built-in'
      })
    )
    render(<ModelPicker agentType="copilot" projectPath="/p" onChange={vi.fn()} />)
    open()
    await screen.findByRole('option', { name: /Auto/ })
    expect(screen.getByText(/Known ids/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Refresh models' })).not.toBeInTheDocument()
  })

  it('is settled, not hidden, when the step follows the task', () => {
    api()
    render(<ModelPicker variant="form" agentType="fromTask" onChange={vi.fn()} />)
    expect(trigger()).toBeDisabled()
    expect(trigger()).toHaveTextContent("Follows the task's agent")
  })

  it('shows only the icon in the bordered variant until a model is chosen', () => {
    api()
    const { rerender } = render(
      <ModelPicker variant="bordered" agentType="claude" onChange={vi.fn()} />
    )
    expect(trigger()).not.toHaveTextContent('Default')
    rerender(<ModelPicker variant="bordered" agentType="claude" value="haiku" onChange={vi.fn()} />)
    expect(trigger()).toHaveTextContent('haiku')
  })
})
