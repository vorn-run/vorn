import { describe, it, expect } from 'vitest'
import {
  initialInputValues,
  areInputsValid,
  parseNumberInput
} from '../src/renderer/lib/workflow-inputs'
import type { WorkflowInputDef } from '../src/shared/types'

const def = (over: Partial<WorkflowInputDef> = {}): WorkflowInputDef => ({
  key: 'k',
  label: 'K',
  type: 'text',
  ...over
})

describe('initialInputValues', () => {
  it('seeds a declared default', () => {
    expect(initialInputValues([def({ defaultValue: 'x' })])).toEqual({ k: 'x' })
  })

  it('seeds a toggle as false so it renders unchecked, not indeterminate', () => {
    expect(initialInputValues([def({ type: 'boolean' })])).toEqual({ k: false })
  })

  it('seeds everything else as empty string', () => {
    expect(initialInputValues([def({ type: 'number' })])).toEqual({ k: '' })
  })

  it('parses a number default so an untouched field submits a number', () => {
    // Defaults are authored as text. Left as a string, submitting without
    // touching the field would persist '42' while touching it persists 42,
    // and the two would dedupe as different runs.
    expect(initialInputValues([def({ type: 'number', defaultValue: '42' })])).toEqual({ k: 42 })
  })

  it('drops a number default that does not parse', () => {
    expect(initialInputValues([def({ type: 'number', defaultValue: 'abc' })])).toEqual({ k: '' })
  })
})

describe('parseNumberInput', () => {
  it('parses a number', () => {
    expect(parseNumberInput('42')).toBe(42)
    expect(parseNumberInput('-1.5')).toBe(-1.5)
  })

  it('treats blank as no value', () => {
    expect(parseNumberInput('')).toBe('')
    expect(parseNumberInput('  ')).toBe('')
  })

  it('treats unparseable and overflowing text as no value', () => {
    // Neither NaN nor Infinity survives JSON, so they must never be stored.
    expect(parseNumberInput('abc')).toBe('')
    expect(parseNumberInput('1e999')).toBe('')
  })
})

describe('areInputsValid', () => {
  it('accepts an optional input left blank', () => {
    expect(areInputsValid([def()], { k: '' })).toBe(true)
  })

  it('rejects a required input left blank or whitespace', () => {
    expect(areInputsValid([def({ required: true })], { k: '' })).toBe(false)
    expect(areInputsValid([def({ required: true })], { k: '   ' })).toBe(false)
  })

  it('rejects a required input that is missing entirely', () => {
    expect(areInputsValid([def({ required: true })], {})).toBe(false)
  })

  it('treats a required toggle as answered even when false', () => {
    // `false` is a real answer, not a missing one.
    expect(areInputsValid([def({ type: 'boolean', required: true })], { k: false })).toBe(true)
  })

  it('accepts a required number of zero', () => {
    expect(areInputsValid([def({ type: 'number', required: true })], { k: 0 })).toBe(true)
  })

  it('rejects a required number that failed to parse', () => {
    // String(NaN) is 'NaN', which would otherwise pass the non-empty check and
    // leak into templates, persistence and the dedupe fingerprint.
    expect(areInputsValid([def({ type: 'number', required: true })], { k: NaN })).toBe(false)
  })

  it('rejects a non-finite number even on an optional field', () => {
    // Optional fields still reach persistence and the dedupe fingerprint.
    expect(areInputsValid([def({ type: 'number' })], { k: Infinity })).toBe(false)
    expect(areInputsValid([def({ type: 'number' })], { k: NaN })).toBe(false)
  })

  it('accepts a required input holding a resolved object', () => {
    expect(areInputsValid([def({ required: true })], { k: { number: 7 } })).toBe(true)
  })
})
