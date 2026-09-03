// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.mock('react-dom', async () => {
  const actual = await vi.importActual<typeof import('react-dom')>('react-dom')
  return { ...actual, createPortal: (n: React.ReactNode) => n }
})
vi.mock('framer-motion', () => ({
  motion: {
    div: ({ children, ...p }: React.PropsWithChildren<Record<string, unknown>>) => (
      <div {...p}>{children}</div>
    )
  },
  AnimatePresence: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

const projectPickerProps: Array<Record<string, unknown>> = []
vi.mock('../src/renderer/components/ProjectPicker', () => ({
  ProjectPicker: (props: Record<string, unknown>) => {
    projectPickerProps.push(props)
    return <div data-testid="project-picker" />
  }
}))
vi.mock('../src/renderer/components/workflow-editor/panels/VariableAutocomplete', () => ({
  VariableAutocomplete: () => <div data-testid="variable-autocomplete" />
}))

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (s: unknown) => unknown) =>
    selector ? selector({ config: { projects: [] } }) : { config: { projects: [] } }
}))

const selectPickerProps: Array<Record<string, unknown>> = []
vi.mock('../src/renderer/components/SelectPicker', () => ({
  SelectPicker: (props: Record<string, unknown>) => {
    selectPickerProps.push(props)
    return <div data-testid="select-picker" />
  }
}))

const held = { keys: [] as Array<{ connectionId: string; name: string }> }
vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnectorKeys: () => held.keys
}))

import { ScriptConfigForm } from '../src/renderer/components/workflow-editor/panels/ScriptConfigForm'
import type { ScriptConfig } from '../src/shared/types'

function base(o: Partial<ScriptConfig> = {}): ScriptConfig {
  return { scriptType: 'bash', scriptContent: '', ...o }
}

describe('the key a script borrows', () => {
  beforeEach(() => {
    selectPickerProps.length = 0
    held.keys = [{ connectionId: 'conn-slack', name: 'Sandbox Slack' }]
  })

  const secretsPicker = () =>
    selectPickerProps.find((props) => {
      const options = props.options as Array<{ value: string }> | undefined
      return options?.some((option) => option.value === 'conn-slack')
    })

  it('offers the connections this machine holds a secret for', async () => {
    render(<ScriptConfigForm config={base()} onChange={vi.fn()} />)
    await vi.waitFor(() => expect(secretsPicker()).toBeDefined())
    expect(secretsPicker()!.options).toEqual([
      { value: '', label: 'None' },
      { value: 'conn-slack', label: 'Sandbox Slack' }
    ])
  })

  it('names the connection on the step, and unsets it rather than storing an empty one', async () => {
    const onChange = vi.fn<(config: ScriptConfig) => void>()
    render(<ScriptConfigForm config={base()} onChange={onChange} />)
    await vi.waitFor(() => expect(secretsPicker()).toBeDefined())

    const pick = secretsPicker()!.onChange as (value: string) => void
    pick('conn-slack')
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ secretsFrom: 'conn-slack' }))

    pick('')
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ secretsFrom: undefined }))
  })

  it('asks for nothing when this machine holds no keys', () => {
    held.keys = []
    const { queryByText } = render(<ScriptConfigForm config={base()} onChange={vi.fn()} />)
    expect(queryByText('Secrets from')).toBeNull()
    expect(
      selectPickerProps.some((props) =>
        (props.options as Array<{ label: string }>).some((o) => o.label === 'None')
      )
    ).toBe(false)
  })
})

describe('ScriptConfigForm — contextual surface', () => {
  beforeEach(() => {
    projectPickerProps.length = 0
  })

  it('passes allowFromContext=true when contextual', () => {
    render(<ScriptConfigForm config={base()} onChange={vi.fn()} isContextualTrigger />)
    expect(projectPickerProps.at(-1)!.allowFromContext).toBe(true)
  })

  it('flags isFromContext when cwd holds the sentinel', () => {
    render(
      <ScriptConfigForm
        config={base({ cwd: '{{context.cwd}}' })}
        onChange={vi.fn()}
        isContextualTrigger
      />
    )
    expect(projectPickerProps.at(-1)!.isFromContext).toBe(true)
  })

  it('writes context sentinels when onSelectFromContext is invoked', () => {
    const onChange = vi.fn<(config: ScriptConfig) => void>()
    render(<ScriptConfigForm config={base()} onChange={onChange} isContextualTrigger />)
    const onSelect = projectPickerProps.at(-1)!.onSelectFromContext as () => void
    onSelect()
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        projectName: '{{context.projectName}}',
        projectPath: '{{context.projectPath}}',
        cwd: '{{context.cwd}}'
      })
    )
  })

  it('clears From Context fields when the trigger flips off contextual', () => {
    const onChange = vi.fn<(config: ScriptConfig) => void>()
    const { rerender } = render(
      <ScriptConfigForm
        config={base({
          cwd: '{{context.cwd}}',
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}'
        })}
        onChange={onChange}
        isContextualTrigger
      />
    )
    rerender(
      <ScriptConfigForm
        config={base({
          cwd: '{{context.cwd}}',
          projectName: '{{context.projectName}}',
          projectPath: '{{context.projectPath}}'
        })}
        onChange={onChange}
        isContextualTrigger={false}
      />
    )
    const reset = onChange.mock.calls.find(
      ([c]: [ScriptConfig]) =>
        c.cwd === undefined && c.projectName === undefined && c.projectPath === undefined
    )
    expect(reset).toBeDefined()
  })
})
