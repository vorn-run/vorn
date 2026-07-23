/**
 * Typed agent outputs for `launchAgent` workflow nodes.
 *
 * A headless agent emits only freeform text, so to let downstream `condition`
 * nodes branch on what the agent *decided* we ask it to end its run with a JSON
 * object matching a user-declared JSON Schema, wrapped in sentinel markers we
 * can find in the logs. `extractStructuredOutput` pulls that object back out and
 * validates it; the engine stores the result as the node's `structuredOutput`,
 * which surfaces as `{{steps.<slug>.<field>}}` step vars via the same path the
 * connector actions already use.
 */
import { schemaProperties, schemaTypeHint } from './json-schema-utils'

/** Markers the agent wraps its JSON answer in. Deliberately unlikely to appear
 *  in normal prose so the extractor can find the block even amid other output. */
export const STRUCTURED_OUTPUT_BEGIN = '<<<VORN_OUTPUT>>>'
export const STRUCTURED_OUTPUT_END = '<<<END_VORN_OUTPUT>>>'

export interface StructuredOutputResult {
  /** The parsed, validated object — present only when `error` is absent. */
  output?: Record<string, unknown>
  /** Human-readable reason extraction/validation failed, if it did. */
  error?: string
}

/**
 * The instruction block appended to a workflow prompt when a node declares an
 * `outputSchema`. Kept here so the marker contract lives in one place alongside
 * the extractor that relies on it.
 */
export function buildStructuredOutputInstructions(schema: Record<string, unknown>): string {
  const lines: string[] = []
  lines.push('## Required Output')
  lines.push('')
  lines.push(
    'When you have finished, output a single JSON object that conforms to the ' +
      'JSON Schema below. Wrap it exactly between the two marker lines — nothing ' +
      'else after the closing marker:'
  )
  lines.push('')
  lines.push(STRUCTURED_OUTPUT_BEGIN)
  lines.push('{ ...your JSON here... }')
  lines.push(STRUCTURED_OUTPUT_END)
  lines.push('')
  lines.push('Schema:')
  lines.push('')
  lines.push('```json')
  lines.push(JSON.stringify(schema, null, 2))
  lines.push('```')
  lines.push('')
  return lines.join('\n')
}

/** Pull the JSON text the agent produced out of a run's full log output.
 *  Precedence: the sentinel-delimited block (last occurrence, since an agent
 *  may echo the schema/example earlier), then a fenced ```json block, then the
 *  last balanced top-level `{...}`. Returns null when nothing usable is found. */
function extractJsonText(text: string): string | null {
  // 1. Sentinel-delimited block — the happy path.
  const beginIdx = text.lastIndexOf(STRUCTURED_OUTPUT_BEGIN)
  if (beginIdx !== -1) {
    const afterBegin = beginIdx + STRUCTURED_OUTPUT_BEGIN.length
    const endIdx = text.indexOf(STRUCTURED_OUTPUT_END, afterBegin)
    const block = text.slice(afterBegin, endIdx === -1 ? undefined : endIdx)
    const balanced = lastBalancedObject(block)
    if (balanced) return balanced
  }

  // 2. Fenced ```json block (last one).
  const fenceMatches = [...text.matchAll(/```(?:json)?\s*([\s\S]*?)```/gi)]
  for (let i = fenceMatches.length - 1; i >= 0; i--) {
    const balanced = lastBalancedObject(fenceMatches[i][1])
    if (balanced) return balanced
  }

  // 3. Last balanced top-level object anywhere in the text.
  return lastBalancedObject(text)
}

/** Find the last complete, brace-balanced top-level `{...}` in a string. Scans
 *  once from the start tracking string-literal state, so braces inside string
 *  values are ignored and nested objects don't split the match. Returns null if
 *  no complete top-level object is present. */
function lastBalancedObject(text: string): string | null {
  let depth = 0
  let inString = false
  let escaped = false
  let start = -1
  let last: string | null = null
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (inString) {
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
      continue
    }
    if (ch === '"') inString = true
    else if (ch === '{') {
      if (depth === 0) start = i
      depth++
    } else if (ch === '}' && depth > 0) {
      depth--
      if (depth === 0 && start !== -1) {
        last = text.slice(start, i + 1)
        start = -1
      }
    }
  }
  return last
}

/** Coerce stringified scalars to the type the schema declares. Models sometimes
 *  emit `"true"` / `"42"` for boolean/number fields; be lenient at the top level
 *  so a downstream numeric/boolean comparison sees the intended value. */
function coerceToSchema(
  obj: Record<string, unknown>,
  schema: Record<string, unknown>
): Record<string, unknown> {
  const props = schemaProperties(schema)
  const out: Record<string, unknown> = { ...obj }
  for (const [key, propSchema] of Object.entries(props)) {
    if (typeof out[key] !== 'string') continue
    const type = schemaTypeHint(propSchema)
    const raw = out[key] as string
    if (type === 'number' || type === 'integer') {
      const n = Number(raw)
      if (raw.trim() !== '' && !Number.isNaN(n)) out[key] = n
    } else if (type === 'boolean') {
      if (raw === 'true') out[key] = true
      else if (raw === 'false') out[key] = false
    }
  }
  return out
}

/**
 * Extract and validate the structured object an agent produced for a node with
 * the given `outputSchema`. Validation is intentionally lightweight: the value
 * must be a plain JSON object and must contain every `required` key the schema
 * declares. On any failure a descriptive `error` is returned so the engine can
 * mark the node `error` rather than let a downstream branch act on garbage.
 */
export function extractStructuredOutput(
  logs: string,
  schema: Record<string, unknown>
): StructuredOutputResult {
  const jsonText = extractJsonText(logs || '')
  if (!jsonText) {
    return { error: 'No JSON output block found in the agent output.' }
  }

  let parsed: unknown
  try {
    parsed = JSON.parse(jsonText)
  } catch {
    return { error: 'The agent output was not valid JSON.' }
  }

  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { error: 'The agent output was not a JSON object.' }
  }

  const coerced = coerceToSchema(parsed as Record<string, unknown>, schema)

  // Validate against the schema's `required` list directly, not just the keys
  // that happen to appear under `properties` — a schema may require a field it
  // doesn't otherwise describe, and that must still be enforced.
  const required = (schema as { required?: unknown }).required
  const missing = Array.isArray(required)
    ? required.filter((key): key is string => typeof key === 'string' && !(key in coerced))
    : []
  if (missing.length > 0) {
    return { error: `Agent output is missing required field(s): ${missing.join(', ')}.` }
  }

  return { output: coerced }
}
