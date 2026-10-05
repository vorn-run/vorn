/**
 * What may differ between the TypeScript store and the native one for the
 * same calls, and nothing else.
 *
 * - Ids and tokens the store makes up (`randomUUID()`), and timestamps it
 *   takes from the clock, differ run to run on either store. Each is replaced
 *   by a placeholder numbered by first appearance, so the same value in two
 *   places still has to match.
 * - Values cross as JSON on the native side, so both sides are compared as
 *   JSON: a key set to `undefined` is the same as a missing one.
 * - Key order is not compared: keys are visited sorted, so placeholders are
 *   numbered the same whatever order either side wrote them in.
 */

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
const TOKEN = /^[0-9a-f]{32}$/
const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/
const HOUR = 60 * 60 * 1000

/** A copy of `value` with what the store made up replaced by placeholders. */
export function normalizeStoreOutput(value: unknown, now = Date.now()): unknown {
  const seen = new Map<string, string>()
  const name = (kind: string, raw: string): string => {
    let placeholder = seen.get(raw)
    if (!placeholder) {
      placeholder = `<${kind} ${seen.size + 1}>`
      seen.set(raw, placeholder)
    }
    return placeholder
  }
  const recent = (ms: number): boolean => Math.abs(ms - now) < HOUR
  const walk = (v: unknown): unknown => {
    if (typeof v === 'string') {
      if (UUID.test(v)) return name('uuid', v)
      if (TOKEN.test(v)) return name('token', v)
      if (ISO.test(v) && recent(Date.parse(v))) return '<now>'
      // A batch id or run key embedding one.
      return v.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, (m) =>
        name('uuid', m)
      )
    }
    if (typeof v === 'number') return Number.isInteger(v) && recent(v) ? '<now ms>' : v
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      return Object.fromEntries(
        Object.entries(v)
          .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
          .map(([k, inner]) => [k, walk(inner)])
      )
    }
    return v
  }
  return walk(JSON.parse(JSON.stringify(value ?? null)))
}
