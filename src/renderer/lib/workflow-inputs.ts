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
    return value !== undefined && value !== null && String(value).trim() !== ''
  })
}
