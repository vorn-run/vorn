// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { useState } from 'react'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { CodeEditor } from '../src/renderer/components/code-editor/CodeEditor'

// Stub shiki so highlightCode resolves without WASM in jsdom.
vi.mock('shiki', () => ({
  createHighlighter: async () => ({
    loadLanguage: async () => undefined,
    codeToTokens: () => ({ tokens: [] })
  }),
  createJavaScriptRegexEngine: () => ({})
}))

describe('CodeEditor', () => {
  it('hands each edit to onChange and draws the new text', () => {
    const onChange = vi.fn()
    function Harness(): React.JSX.Element {
      const [value, setValue] = useState('one')
      return (
        <CodeEditor
          value={value}
          onChange={(next) => {
            onChange(next)
            setValue(next)
          }}
          language="json"
          ariaLabel="Code"
        />
      )
    }
    render(<Harness />)
    fireEvent.change(screen.getByLabelText('Code'), { target: { value: 'one\ntwo' } })
    expect(onChange).toHaveBeenCalledWith('one\ntwo')
    expect(screen.getByTestId('editor-highlight')).toHaveTextContent('onetwo')
  })

  it('marks the error line in the gutter and across its row', () => {
    render(<CodeEditor value={'[\n  1,\n]'} onChange={() => {}} ariaLabel="Code" errorLine={3} />)
    expect(screen.getByTestId('editor-error-gutter')).toHaveTextContent('3')
    const rows = screen.getByTestId('editor-highlight').children
    expect(rows[2]).toHaveAttribute('data-error-line', 'true')
    expect(rows[0]).not.toHaveAttribute('data-error-line')
    expect(screen.getByLabelText('Code')).toHaveAttribute('aria-invalid', 'true')
  })

  it('leaves every line unmarked without an error', () => {
    render(<CodeEditor value={'a\nb'} onChange={() => {}} ariaLabel="Code" />)
    expect(screen.queryByTestId('editor-error-gutter')).not.toBeInTheDocument()
    expect(screen.getByLabelText('Code')).not.toHaveAttribute('aria-invalid')
  })

  it('saves on Cmd+S and passes other keys through', () => {
    const onSaveShortcut = vi.fn()
    const onKeyDown = vi.fn()
    render(
      <CodeEditor
        value="x"
        onChange={() => {}}
        ariaLabel="Code"
        onSaveShortcut={onSaveShortcut}
        onKeyDown={onKeyDown}
      />
    )
    const box = screen.getByLabelText('Code')
    fireEvent.keyDown(box, { key: 's', metaKey: true })
    expect(onSaveShortcut).toHaveBeenCalledTimes(1)
    expect(onKeyDown).not.toHaveBeenCalled()
    fireEvent.keyDown(box, { key: 'Escape' })
    expect(onKeyDown).toHaveBeenCalledTimes(1)
  })
})
