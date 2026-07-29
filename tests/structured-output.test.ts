import { describe, it, expect } from 'vitest'
import {
  extractStructuredOutput,
  buildStructuredOutputInstructions,
  STRUCTURED_OUTPUT_BEGIN,
  STRUCTURED_OUTPUT_END
} from '@vornrun/shared/structured-output'

const schema = {
  type: 'object',
  properties: {
    verdict: { type: 'string' },
    tests_passed: { type: 'boolean' },
    coverage: { type: 'number' }
  },
  required: ['verdict']
}

function wrap(json: string): string {
  return `Some reasoning here.\n${STRUCTURED_OUTPUT_BEGIN}\n${json}\n${STRUCTURED_OUTPUT_END}\nDone.`
}

describe('extractStructuredOutput', () => {
  it('parses the object from the sentinel block', () => {
    const logs = wrap('{ "verdict": "APPROVE", "tests_passed": true }')
    const { output, error } = extractStructuredOutput(logs, schema)
    expect(error).toBeUndefined()
    expect(output).toEqual({ verdict: 'APPROVE', tests_passed: true })
  })

  it('takes the LAST sentinel block when the schema/example was echoed earlier', () => {
    const logs = wrap('{ "verdict": "EXAMPLE" }') + '\n' + wrap('{ "verdict": "REQUEST_CHANGES" }')
    const { output } = extractStructuredOutput(logs, schema)
    expect(output).toEqual({ verdict: 'REQUEST_CHANGES' })
  })

  it('falls back to a fenced ```json block when markers are absent', () => {
    const logs = 'Here is my answer:\n```json\n{ "verdict": "APPROVE" }\n```\n'
    const { output } = extractStructuredOutput(logs, schema)
    expect(output).toEqual({ verdict: 'APPROVE' })
  })

  it('falls back to the last balanced object in freeform text', () => {
    const logs = 'blah {not json} blah\nfinal: { "verdict": "APPROVE" } trailing words'
    const { output } = extractStructuredOutput(logs, schema)
    expect(output).toEqual({ verdict: 'APPROVE' })
  })

  it('ignores braces inside string literals when balancing', () => {
    const logs = wrap('{ "verdict": "use { and } carefully" }')
    const { output } = extractStructuredOutput(logs, schema)
    expect(output).toEqual({ verdict: 'use { and } carefully' })
  })

  it('coerces stringified scalars to the schema-declared type', () => {
    const logs = wrap('{ "verdict": "APPROVE", "tests_passed": "true", "coverage": "82" }')
    const { output } = extractStructuredOutput(logs, schema)
    expect(output).toEqual({ verdict: 'APPROVE', tests_passed: true, coverage: 82 })
  })

  it('coerces stringified "false" and leaves non-numeric strings alone', () => {
    const logs = wrap('{ "verdict": "APPROVE", "tests_passed": "false", "coverage": "n/a" }')
    const { output } = extractStructuredOutput(logs, schema)
    // "false" → false; "n/a" isn't a number so `coverage` stays a string.
    expect(output).toEqual({ verdict: 'APPROVE', tests_passed: false, coverage: 'n/a' })
  })

  it('errors when no JSON is present', () => {
    const { output, error } = extractStructuredOutput('the agent said nothing useful', schema)
    expect(output).toBeUndefined()
    expect(error).toMatch(/no json/i)
  })

  it('errors on invalid JSON', () => {
    const logs = wrap('{ "verdict": "APPROVE", }} broken')
    const { error } = extractStructuredOutput(logs, schema)
    expect(error).toMatch(/not valid json|not a json object/i)
  })

  it('errors when the value is a JSON array, not an object', () => {
    const logs = `${STRUCTURED_OUTPUT_BEGIN}\n[1, 2, 3]\n${STRUCTURED_OUTPUT_END}`
    const { error } = extractStructuredOutput(logs, schema)
    // An array has no top-level object, so extraction reports no object found.
    expect(error).toBeTruthy()
  })

  it('errors when a required field is missing', () => {
    const logs = wrap('{ "tests_passed": true }')
    const { output, error } = extractStructuredOutput(logs, schema)
    expect(output).toBeUndefined()
    expect(error).toMatch(/missing required field/i)
    expect(error).toMatch(/verdict/)
  })

  it('does not require optional fields', () => {
    const logs = wrap('{ "verdict": "APPROVE" }')
    const { output, error } = extractStructuredOutput(logs, schema)
    expect(error).toBeUndefined()
    expect(output).toEqual({ verdict: 'APPROVE' })
  })

  it('enforces required keys even when absent from properties', () => {
    // A schema can require a field it doesn't otherwise describe under
    // properties; that must still be validated.
    const sparse = { type: 'object', properties: {}, required: ['verdict'] }
    const { output, error } = extractStructuredOutput(wrap('{ "other": 1 }'), sparse)
    expect(output).toBeUndefined()
    expect(error).toMatch(/missing required field/i)
    expect(error).toMatch(/verdict/)
  })
})

describe('buildStructuredOutputInstructions', () => {
  it('includes both markers and the serialized schema', () => {
    const text = buildStructuredOutputInstructions(schema)
    expect(text).toContain(STRUCTURED_OUTPUT_BEGIN)
    expect(text).toContain(STRUCTURED_OUTPUT_END)
    expect(text).toContain('"verdict"')
    expect(text).toContain('Required Output')
  })
})
