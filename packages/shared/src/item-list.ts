/**
 * Lists of records, as a for-each loop walks them and a review gate draws them.
 *
 * Both take whatever a step produced: a list, the JSON text of one, or an
 * object wrapping one (`{ "findings": [...] }`, the shape agents and APIs
 * reach for). They agree on what counts, so a gate that shows a table hands a
 * loop exactly the rows it showed.
 */

export type ItemList = { items: unknown[]; wrapperKey?: string } | { error: string }

export function toItemList(value: unknown): ItemList {
  let data = value
  if (typeof data === 'string') {
    const text = data.trim()
    if (text === '') return { items: [] }
    try {
      data = JSON.parse(text)
    } catch (err) {
      const where = jsonErrorLocation(text, err)
      return {
        error: `Not a list: the JSON is invalid at line ${where.line}, column ${where.column}.`
      }
    }
  }
  if (Array.isArray(data)) return { items: data }
  if (data && typeof data === 'object') {
    const lists = Object.entries(data as Record<string, unknown>).filter(([, v]) =>
      Array.isArray(v)
    )
    if (lists.length === 1) return { items: lists[0][1] as unknown[], wrapperKey: lists[0][0] }
    if (lists.length > 1) {
      return {
        error: `Not a list: the object holds several (${lists.map(([k]) => k).join(', ')}).`
      }
    }
  }
  return { error: 'Not a list: expected a JSON array, or an object holding one.' }
}

/** Whether every item is a plain object, which is what a table can draw. */
export function isRecordList(items: unknown[]): items is Record<string, unknown>[] {
  return (
    items.length > 0 && items.every((i) => i !== null && typeof i === 'object' && !Array.isArray(i))
  )
}

/** Where a JSON.parse failure happened, as a person counts: from line 1, column 1. */
export function jsonErrorLocation(
  text: string,
  err: unknown
): { line: number; column: number; message: string } {
  const message = err instanceof Error ? err.message : String(err)
  const explicit = /line (\d+) column (\d+)/.exec(message)
  if (explicit) return { line: Number(explicit[1]), column: Number(explicit[2]), message }
  const position = /position (\d+)/.exec(message)
  const offset = position ? Math.min(Number(position[1]), text.length) : text.length
  const before = text.slice(0, offset).split('\n')
  return { line: before.length, column: before[before.length - 1].length + 1, message }
}
