// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, cleanup, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { LoopNode } from '../src/renderer/components/workflow-editor/nodes/LoopNode'
import { LoopConfigForm } from '../src/renderer/components/workflow-editor/panels/LoopConfigForm'
import { ConnectorButton } from '../src/renderer/components/workflow-editor/nodes/AddStepNode'
import type { LoopConfig, WorkflowNode } from '../packages/shared/src/types'

afterEach(cleanup)

const step = (id: string, label: string): WorkflowNode => ({
  id,
  type: 'script',
  label,
  config: {} as WorkflowNode['config'],
  position: { x: 0, y: 0 }
})

const config = (over: Partial<LoopConfig> = {}): LoopConfig => ({
  nodeType: 'loop',
  bodyNodeIds: ['write', 'review'],
  maxIterations: 2,
  ...over
})

describe('LoopNode (the fallback card for an unreachable loop)', () => {
  const nodes = [step('write', 'Write the edition'), step('review', 'Review the draft')]

  it('names the steps it repeats, in run order', () => {
    render(<LoopNode label="Repeat" config={config()} nodes={nodes} onClick={() => {}} />)
    expect(screen.getByText('Write the edition')).toBeInTheDocument()
    expect(screen.getByText('Review the draft')).toBeInTheDocument()
  })

  it('says so when it has no body, rather than looking like a finished step', () => {
    render(
      <LoopNode
        label="Repeat"
        config={config({ bodyNodeIds: [] })}
        nodes={nodes}
        onClick={() => {}}
      />
    )
    expect(screen.getByText('No steps selected yet')).toBeInTheDocument()
  })

  it('ignores a body id whose step no longer exists', () => {
    // A step can be deleted while the loop still lists it; showing a blank row
    // would be worse than showing one fewer.
    render(
      <LoopNode
        label="Repeat"
        config={config({ bodyNodeIds: ['write', 'deleted'] })}
        nodes={nodes}
        onClick={() => {}}
      />
    )
    expect(screen.getByText(/Repeats 1 step /)).toBeInTheDocument()
  })

  it('shows the passes taken once a run has happened', () => {
    render(
      <LoopNode label="Repeat" config={config()} nodes={nodes} iteration={2} onClick={() => {}} />
    )
    expect(screen.getByText('2×')).toBeInTheDocument()
  })

  it('surfaces the exit condition', () => {
    render(
      <LoopNode
        label="Repeat"
        config={config({
          until: { variable: '{{steps.review.approved}}', operator: 'equals', value: 'true' }
        })}
        nodes={nodes}
        onClick={() => {}}
      />
    )
    expect(screen.getByText(/until \{\{steps.review.approved\}\} equals true/)).toBeInTheDocument()
  })

  it('selects itself when clicked', () => {
    const onClick = vi.fn()
    render(<LoopNode label="Repeat" config={config()} nodes={nodes} onClick={onClick} />)
    fireEvent.click(screen.getByText('Repeat'))
    expect(onClick).toHaveBeenCalledOnce()
  })
})

describe('LoopConfigForm', () => {
  it('offers only the budget and the condition — membership is spatial now', () => {
    render(<LoopConfigForm config={config()} onChange={() => {}} />)
    expect(screen.getByText('Maximum passes')).toBeInTheDocument()
    expect(screen.getByText(/Stop early when/)).toBeInTheDocument()
    // The checkbox list this replaced let the panel drift from the graph.
    expect(screen.queryByText('Steps to repeat')).not.toBeInTheDocument()
    expect(document.querySelectorAll('input[type=checkbox]')).toHaveLength(0)
  })

  it('clamps a runaway pass count', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={config()} onChange={onChange} />)
    fireEvent.change(screen.getByRole('spinbutton'), { target: { value: '999' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ maxIterations: 10 }))
  })

  it('never lets the count reach zero, which would run nothing', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={config()} onChange={onChange} />)
    fireEvent.change(screen.getByRole('spinbutton'), { target: { value: '0' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ maxIterations: 1 }))
  })

  it('builds the condition one field at a time without losing the others', () => {
    const onChange = vi.fn()
    render(
      <LoopConfigForm
        config={config({
          until: { variable: '{{steps.review.approved}}', operator: 'equals', value: '' }
        })}
        onChange={onChange}
      />
    )
    fireEvent.change(screen.getByPlaceholderText('true'), { target: { value: 'true' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        until: { variable: '{{steps.review.approved}}', operator: 'equals', value: 'true' }
      })
    )
  })

  it('keeps the variable when only the operator changes', () => {
    const onChange = vi.fn()
    render(
      <LoopConfigForm
        config={config({
          until: { variable: '{{steps.review.blocking}}', operator: 'equals', value: 'x' }
        })}
        onChange={onChange}
      />
    )
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'isEmpty' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        until: { variable: '{{steps.review.blocking}}', operator: 'isEmpty', value: 'x' }
      })
    )
  })

  it('explains that the cap is what ends the loop', () => {
    // The number looks like a safeguard and is actually the termination
    // condition; the form has to say so or it will be set to 1 and forgotten.
    render(<LoopConfigForm config={config()} onChange={() => {}} />)
    expect(screen.getByText(/not a safety net/)).toBeInTheDocument()
  })
})

describe('LoopConfigForm with no condition yet', () => {
  // Each field has to be able to start the condition on its own; requiring the
  // variable first would make the other two inputs dead until it was typed.
  const bare = config({ until: undefined })

  it('starts the condition from the variable', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={bare} onChange={onChange} />)
    fireEvent.change(screen.getByPlaceholderText('{{steps.review.approved}}'), {
      target: { value: '{{steps.review.approved}}' }
    })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        until: { variable: '{{steps.review.approved}}', operator: 'equals', value: '' }
      })
    )
  })

  it('starts the condition from the operator', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={bare} onChange={onChange} />)
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'isNotEmpty' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ until: { variable: '', operator: 'isNotEmpty', value: '' } })
    )
  })

  it('starts the condition from the value', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={bare} onChange={onChange} />)
    fireEvent.change(screen.getByPlaceholderText('true'), { target: { value: 'yes' } })
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ until: { variable: '', operator: 'equals', value: 'yes' } })
    )
  })

  it('falls back to one pass when the count is not a number', () => {
    const onChange = vi.fn()
    render(<LoopConfigForm config={bare} onChange={onChange} />)
    fireEvent.change(screen.getByRole('spinbutton'), { target: { value: '' } })
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ maxIterations: 1 }))
  })
})

describe('the + on the canvas', () => {
  it('opens the step library instead of carrying its own menu', () => {
    const onOpen = vi.fn()
    render(<ConnectorButton onOpen={onOpen} />)
    fireEvent.click(screen.getByRole('button', { name: 'Add a step' }))
    expect(onOpen).toHaveBeenCalledOnce()
    // Whether a loop is offered is the library's decision now.
    expect(screen.queryByText(/Repeat steps/)).toBeNull()
  })

  it('stays lit while it is the library anchor', () => {
    render(<ConnectorButton onOpen={() => {}} active />)
    expect(screen.getByRole('button', { name: 'Add a step' }).className).toContain(
      'border-white/40'
    )
  })
})
