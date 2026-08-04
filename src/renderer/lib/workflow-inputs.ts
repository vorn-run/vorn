import type { WorkflowInputDef } from '../../shared/types'

/**
 * Value helpers for manual-run inputs. Kept out of WorkflowInputFields.tsx so
 * that file exports only its component — mixing the two breaks Fast Refresh.
 */

/**
 * Starting value for one declared input. `defaultValue` wins when present;
 * otherwise the empty value for the field's type — booleans need `false`
 * rather than `''` so a toggle renders unchecked instead of indeterminate.
 */
function defaultInputValue(def: WorkflowInputDef): unknown {
  if (def.defaultValue !== undefined) return def.defaultValue
  if (def.type === 'boolean') return false
  return ''
}

export function initialInputValues(defs: WorkflowInputDef[]): Record<string, unknown> {
  return Object.fromEntries(defs.map((d) => [d.key, defaultInputValue(d)]))
}

/**
 * Whether every required input has a value. Booleans are always satisfied —
 * `false` is a real answer, not a missing one.
 */
export function areInputsValid(defs: WorkflowInputDef[], values: Record<string, unknown>): boolean {
  return defs.every((def) => {
    if (!def.required || def.type === 'boolean') return true
    const value = values[def.key]
    if (value === undefined || value === null) return false
    // A number field that failed to parse is missing, not answered —
    // String(NaN) is 'NaN', which would otherwise sail through as non-empty
    // and reach templates, persistence and the dedupe fingerprint.
    if (typeof value === 'number' && Number.isNaN(value)) return false
    return String(value).trim() !== ''
  })
}
