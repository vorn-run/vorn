// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowNode } from '../src/shared/types'

vi.mock('../src/renderer/stores', () => {
  const state = {
    config: { projects: [], tasks: [], defaults: { defaultAgent: 'claude' } },
    activeWorkspace: 'personal'
  }
  const useAppStore = (selector?: (s: unknown) => unknown) => {
    return selector ? selector(state) : state
  }
  useAppStore.getState = () => state
  return { useAppStore }
})

vi.mock('../src/renderer/components/AgentIcon', () => ({
  AgentIcon: () => <div />
}))

vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({ status: {} })
}))

vi.mock('../src/renderer/components/rich-editor/RichMarkdownEditor', () => ({
  RichMarkdownEditor: () => <div />
}))

const { NodeConfigPanel } =
  await import('../src/renderer/components/workflow-editor/panels/NodeConfigPanel')

function makeNode(type: WorkflowNode['type']): WorkflowNode {
  return {
    id: 'n1',
    type,
    label: 'My node',
    config:
      type === 'trigger'
        ? { triggerType: 'manual' }
        : type === 'script'
          ? { scriptType: 'bash' }
          : type === 'condition'
            ? { variable: '', operator: 'equals', value: '' }
            : type === 'approval'
              ? { message: '' }
              : { agentType: 'claude', projectName: '', projectPath: '' },
    position: { x: 0, y: 0 }
  }
}

describe('NodeConfigPanel', () => {
  it('renders the node label in the editable header input', () => {
    render(
      <NodeConfigPanel
        node={makeNode('script')}
        onChange={vi.fn()}
        onLabelChange={vi.fn()}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    const input = screen.getByPlaceholderText('Label') as HTMLInputElement
    expect(input.value).toBe('My node')
  })

  it('calls onLabelChange when the header input changes', () => {
    const onLabelChange = vi.fn()
    render(
      <NodeConfigPanel
        node={makeNode('script')}
        onChange={vi.fn()}
        onLabelChange={onLabelChange}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    fireEvent.change(screen.getByPlaceholderText('Label'), { target: { value: 'Updated' } })
    expect(onLabelChange).toHaveBeenCalledWith('n1', 'Updated')
  })

  it('does not show the More menu button for trigger nodes', () => {
    render(
      <NodeConfigPanel
        node={makeNode('trigger')}
        onChange={vi.fn()}
        onLabelChange={vi.fn()}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    expect(screen.queryByText('Remove action')).not.toBeInTheDocument()
  })

  it('reveals Remove action in the More menu for non-trigger nodes', () => {
    render(
      <NodeConfigPanel
        node={makeNode('script')}
        onChange={vi.fn()}
        onLabelChange={vi.fn()}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    const buttons = screen.getAllByRole('button')
    const moreButton = buttons.find((b) => b.querySelector('svg.lucide-ellipsis'))
    if (moreButton) fireEvent.click(moreButton)
    expect(screen.getByText('Remove action')).toBeInTheDocument()
  })

  it('calls onDelete when Remove action is clicked', () => {
    const onDelete = vi.fn()
    render(
      <NodeConfigPanel
        node={makeNode('script')}
        onChange={vi.fn()}
        onLabelChange={vi.fn()}
        onDelete={onDelete}
        onClose={vi.fn()}
      />
    )
    const buttons = screen.getAllByRole('button')
    const moreButton = buttons.find((b) => b.querySelector('svg.lucide-ellipsis'))
    if (moreButton) fireEvent.click(moreButton)
    fireEvent.click(screen.getByText('Remove action'))
    expect(onDelete).toHaveBeenCalledWith('n1')
  })

  it('renders a step ref hint for non-trigger nodes with a slug', () => {
    const node = { ...makeNode('script'), slug: 'my_step' }
    render(
      <NodeConfigPanel
        node={node}
        onChange={vi.fn()}
        onLabelChange={vi.fn()}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    expect(screen.getByText(/steps\.my_step\.output/)).toBeInTheDocument()
  })

  it('renders the approval form for approval nodes', () => {
    const onChange = vi.fn()
    render(
      <NodeConfigPanel
        node={makeNode('approval')}
        onChange={onChange}
        onLabelChange={vi.fn()}
        onDelete={vi.fn()}
        onClose={vi.fn()}
      />
    )
    const textarea = document.querySelector('textarea') as HTMLTextAreaElement
    expect(textarea).toBeTruthy()
    fireEvent.change(textarea, { target: { value: 'ok?' } })
    expect(onChange).toHaveBeenCalledWith('n1', expect.objectContaining({ message: 'ok?' }))
  })

  describe('the error policy control', () => {
    const policySelect = () => screen.getByLabelText('If this step fails') as HTMLSelectElement

    it('shows a node that says nothing as stopping the run', () => {
      render(
        <NodeConfigPanel
          node={makeNode('script')}
          onChange={vi.fn()}
          onLabelChange={vi.fn()}
          onErrorChange={vi.fn()}
          onDelete={vi.fn()}
          onClose={vi.fn()}
        />
      )
      expect(policySelect().value).toBe('stop')
      expect(screen.getByText(/Everything downstream is skipped/)).toBeInTheDocument()
    })

    it('reflects a node that opted out', () => {
      render(
        <NodeConfigPanel
          node={{ ...makeNode('script'), onError: 'continue' }}
          onChange={vi.fn()}
          onLabelChange={vi.fn()}
          onErrorChange={vi.fn()}
          onDelete={vi.fn()}
          onClose={vi.fn()}
        />
      )
      expect(policySelect().value).toBe('continue')
      expect(screen.getByText(/Later steps run as if this one had succeeded/)).toBeInTheDocument()
    })

    it('reports a change to the policy', () => {
      const onErrorChange = vi.fn()
      render(
        <NodeConfigPanel
          node={makeNode('script')}
          onChange={vi.fn()}
          onLabelChange={vi.fn()}
          onErrorChange={onErrorChange}
          onDelete={vi.fn()}
          onClose={vi.fn()}
        />
      )
      fireEvent.change(policySelect(), { target: { value: 'continue' } })
      expect(onErrorChange).toHaveBeenCalledWith('n1', 'continue')
    })

    // A trigger cannot fail in a way that has anything downstream to govern.
    it('is absent for a trigger', () => {
      render(
        <NodeConfigPanel
          node={makeNode('trigger')}
          onChange={vi.fn()}
          onLabelChange={vi.fn()}
          onErrorChange={vi.fn()}
          onDelete={vi.fn()}
          onClose={vi.fn()}
        />
      )
      expect(screen.queryByLabelText('If this step fails')).not.toBeInTheDocument()
    })
  })
})
