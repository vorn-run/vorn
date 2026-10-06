/**
 * What may differ between the TypeScript MCP server's answer and vornd's
 * answer at `/mcp` to the same message, and nothing else.
 *
 * Both answers are compared as the JSON-RPC responses a client receives,
 * serialized, so key order counts. The configuration a call leaves behind is
 * compared the same way. Each normalizer below names one accepted difference:
 *
 * - {@link madeUpIds}: tasks, workflows, nodes, workspaces and a page fence's
 *   nonce get a random UUID on each side. The nth distinct UUID becomes
 *   `<uuid n>` on both sides, so an id has to recur where the other side's
 *   recurs.
 * - {@link clockTimes}: `createdAt`, `updatedAt` and the like are the moment
 *   each side ran.
 * - {@link jsonParseWording}: `import_workflow` passes on `JSON.parse`'s
 *   message for a workflow that is not JSON. V8 and serde_json word the same
 *   failure differently; both say it is not valid JSON, and only the parser's
 *   own wording after that is dropped.
 */

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi
const ISO_TIME = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?Z/g
const JSON_PARSE = /(workflow is not valid JSON — )SyntaxError: [^"\\]*/g

/** Applies `f` to every string in `value`, keys excluded. */
function mapStrings<T>(value: T, f: (s: string) => string): T {
  if (typeof value === 'string') return f(value) as T
  if (Array.isArray(value)) return value.map((v) => mapStrings(v, f)) as T
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, mapStrings(v, f)])) as T
  }
  return value
}

/** Every UUID, numbered in order of first appearance. */
export function madeUpIds<T>(value: T, seen: Map<string, number> = new Map()): T {
  return mapStrings(value, (s) =>
    s.replace(UUID, (id) => {
      const key = id.toLowerCase()
      if (!seen.has(key)) seen.set(key, seen.size + 1)
      return `<uuid ${seen.get(key)}>`
    })
  )
}

/** Every ISO timestamp is `<time>`. */
export function clockTimes<T>(value: T): T {
  return mapStrings(value, (s) => s.replace(ISO_TIME, '<time>'))
}

/** `JSON.parse`'s own wording after "not valid JSON" is dropped. */
export function jsonParseWording<T>(value: T): T {
  return mapStrings(value, (s) => s.replace(JSON_PARSE, '$1SyntaxError: <parser message>'))
}

/**
 * One side's answer and the configuration it left, normalized and
 * serialized, ready to compare with the other side's. Ids that came from
 * the fixture are kept as they are, so only ids a side made up are numbered.
 */
export function comparable(answer: unknown, config: unknown, fixtureIds: Set<string>): string {
  const seen = new Map<string, number>()
  const keep = (value: unknown): unknown =>
    mapStrings(value, (s) =>
      s.replace(UUID, (id) => (fixtureIds.has(id.toLowerCase()) ? `<fixture ${id}>` : id))
    )
  const normalized = [answer, config].map((v) =>
    jsonParseWording(clockTimes(madeUpIds(keep(v), seen)))
  )
  return JSON.stringify(normalized, null, 2)
}

/** Every UUID in `value`, for {@link comparable}'s `fixtureIds`. */
export function uuidsIn(value: unknown): Set<string> {
  return new Set((JSON.stringify(value).match(UUID) ?? []).map((id) => id.toLowerCase()))
}

/** A seeded generator (mulberry32), so a failing case can be run again. */
export function seeded(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

type Schema = {
  type?: string | string[]
  properties?: Record<string, Schema>
  required?: string[]
  items?: Schema
  enum?: unknown[]
  const?: unknown
  anyOf?: Schema[]
  oneOf?: Schema[]
  additionalProperties?: Schema | boolean
  minimum?: number
  maximum?: number
}

/**
 * Arguments for a tool, made from its input schema: mostly well formed, with
 * values drawn from `pool` (names and ids the fixture has, and some it does
 * not), and now and then a value of the wrong type, so both servers refuse
 * the same arguments the same way.
 */
export function randomArgs(
  schema: Schema,
  random: () => number,
  pool: { strings: string[]; numbers: number[] }
): Record<string, unknown> {
  const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]
  const wrong = (): unknown => pick([42, 'x', true, null, [], {}])
  const value = (s: Schema, depth: number): unknown => {
    if (random() < 0.06) return wrong()
    if (s.const !== undefined) return s.const
    if (s.enum) return random() < 0.9 ? pick(s.enum) : 'not-a-choice'
    const options = s.anyOf ?? s.oneOf
    if (options) return value(pick(options), depth)
    const type = Array.isArray(s.type) ? pick(s.type) : s.type
    switch (type) {
      case 'string':
        return pick(pool.strings)
      case 'number':
        return pick(pool.numbers)
      case 'integer':
        return Math.trunc(pick(pool.numbers))
      case 'boolean':
        return random() < 0.5
      case 'array': {
        if (depth > 3) return []
        const n = Math.floor(random() * 3)
        return Array.from({ length: n }, () => value(s.items ?? {}, depth + 1))
      }
      case 'object':
        return depth > 3 ? {} : object(s, depth + 1)
      default:
        return pick(pool.strings)
    }
  }
  const object = (s: Schema, depth: number): Record<string, unknown> => {
    const out: Record<string, unknown> = {}
    const required = new Set(s.required ?? [])
    for (const [key, prop] of Object.entries(s.properties ?? {})) {
      const include = required.has(key) ? random() < 0.92 : random() < 0.5
      if (include) out[key] = value(prop, depth)
    }
    if (!s.properties && typeof s.additionalProperties === 'object' && random() < 0.7) {
      out[pick(pool.strings) || 'key'] = value(s.additionalProperties, depth)
    }
    return out
  }
  return object(schema, 0)
}

/**
 * Arguments the schema refuses: its first property given a value of the
 * wrong type. For a tool whose handler would start something real (a
 * process, a download, an agent), so only the refusal is compared. Null for
 * a tool that takes no arguments.
 */
export function refusedArgs(schema: Schema): Record<string, unknown> | null {
  const [key, prop] = Object.entries(schema.properties ?? {})[0] ?? []
  if (!key || !prop) return null
  const type = Array.isArray(prop.type) ? prop.type[0] : prop.type
  return { [key]: type === 'string' || prop.enum ? 12345 : 'not the right type' }
}
