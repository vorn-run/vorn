import type { WorkflowInputDef } from '../../shared/types'

/**
 * Value helpers for manual-run inputs. Kept out of WorkflowInputFields.tsx so
 * that file exports only its component — mixing the two breaks Fast Refresh.
 */

/**
 * Starting value for one declared input: the authored default where it makes
 * sense, otherwise the empty value for the field's type.
 *
 * Type is checked before `defaultValue` because a default left behind by an
 * earlier type must not leak through. A toggle seeds `false` so it renders
 * unchecked rather than indeterminate, and a number's default is parsed —
 * defaults are authored as text, and left as a string an untouched field
 * would submit `'42'` where a touched one submits `42`, deduping as two runs.
 */
function defaultInputValue(def: WorkflowInputDef): unknown {
  if (def.type === 'boolean') return def.defaultValue === 'true'
  if (def.type === 'number') {
    return def.defaultValue !== undefined ? parseNumberInput(def.defaultValue) : ''
  }
  return def.defaultValue ?? ''
}

/**
 * Parse text from a number field. Blank and unparseable text both mean "no
 * value": `NaN` and `Infinity` don't survive JSON, so letting them through
 * would corrupt the persisted run, the template expansion and the dedupe
 * fingerprint alike.
 */
export function parseNumberInput(raw: string): number | '' {
  if (raw.trim() === '') return ''
  const parsed = Number(raw)
  return Number.isFinite(parsed) ? parsed : ''
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
    const value = values[def.key]
    // Checked ahead of the `required` shortcut: a non-finite number breaks
    // persistence and dedupe whether or not the field had to be answered.
    if (typeof value === 'number' && !Number.isFinite(value)) return false
    if (!def.required || def.type === 'boolean') return true
    if (value === undefined || value === null) return false
    return String(value).trim() !== ''
  })
}
