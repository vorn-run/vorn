/**
 * Pure transforms for the completion index generator.
 *
 * Reads completion specs from the third-party corpus and reduces them to the
 * Outline shape the intent bar's completion engine already uses. Everything
 * dynamic is dropped: the corpus resolves branches, paths and package
 * scripts by shelling out, and Vorn already has live sources for those.
 *
 * Split out from the generator so the limits below can be unit-tested
 * without touching the filesystem or the corpus.
 */

/** Nesting levels of subcommands kept below the top-level command. */
export const MAX_DEPTH = 2
/** Flags kept per node. Beyond this the menu stops being scannable. */
export const MAX_FLAGS = 12
/** Descriptions are one-line hints, not documentation. */
export const MAX_DETAIL = 48
/**
 * Subcommands kept per node. Cloud CLIs carry hundreds; the menu shows at
 * most a dozen, and the generator reports anything it drops.
 */
export const MAX_SUBCOMMANDS = 60

/** Names dropped by the subcommand cap, reported by the generator. */
export const truncations = []

export function resetTruncations() {
  truncations.length = 0
}

/** First name when a spec uses aliases, e.g. ["-m", "--message"]. */
function firstName(name) {
  if (Array.isArray(name)) return name[0]
  return typeof name === 'string' ? name : undefined
}

function allNames(name) {
  if (Array.isArray(name)) return name.filter((n) => typeof n === 'string')
  return typeof name === 'string' ? [name] : []
}

/**
 * Trim a description to a hint. Returns undefined when it would only repeat
 * the name it sits next to, which is noise in a narrow menu.
 */
export function compactDetail(detail, name) {
  if (typeof detail !== 'string') return undefined
  const trimmed = detail.trim().replace(/\s+/g, ' ')
  if (!trimmed) return undefined
  if (name && trimmed.toLowerCase() === String(name).toLowerCase()) return undefined
  if (trimmed.length <= MAX_DETAIL) return trimmed
  return `${trimmed.slice(0, MAX_DETAIL - 1).trimEnd()}…`
}

/**
 * Map a spec's `args` onto the engine's ArgKind. Only the filesystem
 * templates survive — branch and script arguments come from the live
 * sources, which know the actual repository.
 */
export function argKind(args) {
  if (!args) return 'none'
  const list = Array.isArray(args) ? args : [args]
  const templates = list.flatMap((a) => {
    if (!a || !a.template) return []
    return Array.isArray(a.template) ? a.template : [a.template]
  })
  if (templates.includes('folders')) return 'dir'
  if (templates.includes('filepaths')) return 'path'
  return 'none'
}

function extractFlags(options) {
  if (!Array.isArray(options)) return undefined
  const flags = []
  for (const option of options) {
    if (!option || option.hidden || option.deprecated) continue
    for (const name of allNames(option.name)) {
      if (!name.startsWith('-')) continue
      flags.push({ flag: name, detail: compactDetail(option.description, name) })
      if (flags.length >= MAX_FLAGS) break
    }
    if (flags.length >= MAX_FLAGS) break
  }
  return flags.length ? flags : undefined
}

function extractSubcommands(subcommands, depth, trail) {
  if (!Array.isArray(subcommands) || depth >= MAX_DEPTH) return undefined
  // Annotated because it is filled by name at runtime: without this it infers as
  // `{}` and every reader of a subcommand is untyped.
  /** @type {Record<string, ReturnType<typeof extractNode>>} */
  const sub = {}
  let dropped = 0
  for (const entry of subcommands) {
    if (!entry || entry.hidden || entry.deprecated) continue
    const name = firstName(entry.name)
    if (!name || sub[name]) continue
    if (Object.keys(sub).length >= MAX_SUBCOMMANDS) {
      dropped++
      continue
    }
    sub[name] = extractNode(entry, depth + 1, `${trail} ${name}`.trim())
  }
  if (dropped) truncations.push(`${trail}: dropped ${dropped} subcommands`)
  return Object.keys(sub).length ? sub : undefined
}

function extractNode(node, depth, trail = '') {
  const outline = {}
  const detail = compactDetail(node.description, firstName(node.name))
  if (detail) outline.detail = detail

  const arg = argKind(node.args)
  if (arg !== 'none') outline.arg = arg

  const flags = extractFlags(node.options)
  if (flags) outline.flags = flags

  const sub = extractSubcommands(node.subcommands, depth, trail)
  if (sub) {
    outline.sub = sub
  } else if (arg === 'none') {
    // No subcommands and no filesystem argument: say so explicitly, so the
    // engine offers nothing rather than falling back to path completion.
    outline.arg = 'none'
  }

  return outline
}

/**
 * Turn a spec module's default export into an Outline.
 * Returns null for specs with nothing static worth indexing.
 */
export function extractSpec(spec) {
  if (!spec || typeof spec !== 'object') return null
  const name = firstName(spec.name)
  if (!name) return null
  const outline = extractNode(spec, 0, name)
  // A bare name with no detail, flags or subcommands adds nothing the PATH
  // scan does not already provide.
  if (!outline.detail && !outline.flags && !outline.sub) return null
  return { name, outline }
}

/** Throw rather than silently shipping a bundle nobody meant to ship. */
export function assertBudget(label, bytes, limitBytes) {
  if (bytes > limitBytes) {
    throw new Error(
      `${label} is ${(bytes / 1024).toFixed(1)} KB, over the ${(limitBytes / 1024).toFixed(0)} KB budget`
    )
  }
}
