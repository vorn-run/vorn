// Writes what vornd's MCP server needs to answer as the TypeScript one does,
// read from the TypeScript itself:
//
// - tools.json           the `tools` of a `tools/list` answer, exactly as the
//                        SDK's McpServer sends them over the wire
// - schemas.json         each tool's argument schema as the zod it is checked
//                        with, so vornd can check arguments the same way and
//                        word a refusal as zod does (JSON Schema keeps neither
//                        zod's own messages nor its refinements)
// - workflow-nodes.json  the reference describe_workflow_nodes answers with
//
//   node scripts/gen-mcp-tools.mjs           write the files
//   node scripts/gen-mcp-tools.mjs --check   fail if any is out of date
//
// A refinement is a function, which no file can carry. Each one the tools use
// is named here, and vornd implements it under that name; one this script
// does not know stops it, so a new rule in the TypeScript cannot go unported.
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { register } from 'tsx/esm/api'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const OUT = path.join(root, 'packages/core/crates/mcp/generated')

// One loader for every import, so a schema reached through two modules is one object.
register()
const load = (file) => import(pathToFileURL(path.join(root, file)).href)
const { createMcpServer } = await load('packages/mcp/src/server.ts')
const { V } = await load('packages/mcp/src/validation.ts')
const nodeConfig = await load('packages/mcp/src/tools/node-config-schemas.ts')
const workflows = await load('packages/mcp/src/tools/workflows.ts')
const { WORKFLOW_NODES_REFERENCE } = await load('packages/mcp/src/tools/describe-nodes.ts')
const { InMemoryTransport } = await import('@modelcontextprotocol/sdk/inMemory.js')

/** The `tools/list` answer as a client receives it: no client-side parsing in between. */
async function listTools(server) {
  const [ours, theirs] = InMemoryTransport.createLinkedPair()
  await server.connect(ours)
  const answers = new Map()
  theirs.onmessage = (message) => answers.set(message.id, message)
  await theirs.start()
  const ask = async (id, method, params) => {
    await theirs.send({ jsonrpc: '2.0', id, method, params })
    for (let i = 0; i < 1000 && !answers.has(id); i++) await new Promise((r) => setTimeout(r, 1))
    const answer = answers.get(id)
    if (!answer?.result) throw new Error(`${method} was not answered: ${JSON.stringify(answer)}`)
    return answer.result
  }
  await ask(1, 'initialize', {
    protocolVersion: '2025-06-18',
    capabilities: {},
    clientInfo: { name: 'gen-mcp-tools', version: '0' }
  })
  const { tools } = await ask(2, 'tools/list', {})
  await server.close()
  // Through JSON, as stdio would carry it: undefined fields drop out.
  return JSON.parse(JSON.stringify(tools))
}

// ── Refinements ──────────────────────────────────────────────────

/** Each refinement check object the tools use, by the name vornd knows it as. */
const REFINEMENTS = new Map()

function refinementsOf(schema, name) {
  const custom = (schema._zod.def.checks ?? []).filter((c) => c._zod.def.check === 'custom')
  if (custom.length !== 1) throw new Error(`${name}: expected one refinement, found ${custom.length}`)
  REFINEMENTS.set(custom[0], name)
}

refinementsOf(V.name, 'safeName')
refinementsOf(V.absolutePath, 'absolutePath')
refinementsOf(nodeConfig.workflowInputDefSchema, 'workflowInputDef')
refinementsOf(nodeConfig.workflowInputsSchema, 'uniqueInputKeys')
refinementsOf(nodeConfig.configSchemaByType.loop, 'loopConfig')
refinementsOf(nodeConfig.configSchemaByType.launchAgent, 'outputSchemaNeedsHeadless')
refinementsOf(workflows.nodeSchema, 'nodeConfig')

/**
 * Refinements written inline, which no export reaches: known by their message.
 * workflows.ts repeats node-config-schemas' rule for its convenience actions.
 */
const BY_MESSAGE = new Map([['outputSchema requires headless: true', 'outputSchemaNeedsHeadless']])

/** A check's own message, when it was given one. */
function messageOf(def) {
  if (typeof def.error !== 'function') return undefined
  const made = def.error()
  if (typeof made === 'string') return made
  if (made && typeof made.message === 'string') return made.message
  return undefined
}

function refinement(check) {
  const def = check._zod.def
  const name = REFINEMENTS.get(check) ?? BY_MESSAGE.get(messageOf(def))
  if (!name) {
    throw new Error(
      `a refinement vornd does not know (${messageOf(def) ?? 'no message'}): implement it in ` +
        'packages/core/crates/mcp/src/zod/refine.rs and name it in scripts/gen-mcp-tools.mjs'
    )
  }
  const out = { k: 'custom', name }
  const message = messageOf(def)
  if (message !== undefined) out.message = message
  if (Array.isArray(def.path)) out.path = def.path
  return out
}

// ── Schemas ──────────────────────────────────────────────────────

/** Schemas used in more than one place, written once under `defs`. */
const NAMED = new Map([
  [nodeConfig.triggerConfigSchema, 'triggerConfig'],
  [nodeConfig.edgeSchema, 'edge'],
  [nodeConfig.workflowInputDefSchema, 'workflowInputDef'],
  [nodeConfig.workflowInputsSchema, 'workflowInputs'],
  [workflows.nodeShapeSchema, 'nodeShape'],
  [workflows.nodeSchema, 'node'],
  ...nodeConfig.NODE_TYPES.map((type) => [
    nodeConfig.configSchemaByType[type],
    `nodeConfig.${type}`
  ])
])

const defs = {}

function check(c) {
  const def = c._zod.def
  const message = messageOf(def)
  const withMessage = (out) => (message === undefined ? out : { ...out, message })
  switch (def.check) {
    case 'min_length':
      return withMessage({ k: 'min', value: def.minimum })
    case 'max_length':
      return withMessage({ k: 'max', value: def.maximum })
    case 'greater_than':
      return withMessage({ k: def.inclusive ? 'gte' : 'gt', value: def.value })
    case 'less_than':
      return withMessage({ k: def.inclusive ? 'lte' : 'lt', value: def.value })
    case 'number_format':
      if (def.format !== 'safeint') throw new Error(`number format ${def.format}`)
      return withMessage({ k: 'int' })
    case 'string_format':
      if (def.format !== 'regex' || def.pattern.flags) throw new Error(`string format ${def.format}`)
      return withMessage({ k: 'regex', source: def.pattern.source, shown: String(def.pattern) })
    case 'custom':
      return refinement(c)
    default:
      throw new Error(`a check vornd does not know: ${def.check}`)
  }
}

function checksOf(def) {
  return (def.checks ?? []).map(check)
}

function ir(schema) {
  const name = NAMED.get(schema)
  if (name) {
    if (!(name in defs)) {
      defs[name] = null
      defs[name] = shape(schema)
    }
    return { t: 'ref', name }
  }
  return shape(schema)
}

function shape(schema) {
  const def = schema._zod.def
  const checks = checksOf(def)
  const withChecks = (out) => (checks.length ? { ...out, checks } : out)
  switch (def.type) {
    case 'string':
    case 'number':
    case 'boolean':
      return withChecks({ t: def.type })
    case 'unknown':
    case 'any':
      return { t: 'unknown' }
    case 'literal':
      return { t: 'literal', values: def.values }
    case 'enum':
      return { t: 'enum', values: Object.values(def.entries) }
    case 'optional':
      return { t: 'optional', inner: ir(def.innerType) }
    case 'array':
      return withChecks({ t: 'array', item: ir(def.element) })
    case 'record':
      if (def.keyType._zod.def.type !== 'string') throw new Error('record keys other than string')
      return withChecks({ t: 'record', value: ir(def.valueType) })
    case 'object': {
      const catchall = def.catchall?._zod.def.type
      const mode = catchall === undefined ? 'strip' : catchall === 'never' ? 'strict' : 'loose'
      if (mode === 'loose' && catchall !== 'unknown') throw new Error(`catchall ${catchall}`)
      return withChecks({
        t: 'object',
        mode,
        shape: Object.entries(def.shape).map(([key, value]) => [key, ir(value)])
      })
    }
    case 'union': {
      if (def.discriminator === undefined) {
        return withChecks({ t: 'union', options: def.options.map(ir) })
      }
      const key = def.discriminator
      return withChecks({
        t: 'discriminated',
        key,
        options: def.options.map((option) => {
          const tag = option._zod.def.shape?.[key]?._zod.def
          if (tag?.type !== 'literal') throw new Error(`${key} is not a literal on every option`)
          return { values: tag.values, schema: ir(option) }
        })
      })
    }
    default:
      throw new Error(`a schema type vornd does not know: ${def.type}`)
  }
}

async function generate() {
  const server = createMcpServer('0.0.0')
  const tools = await listTools(server)
  const registered = createMcpServer('0.0.0')._registeredTools
  const inputs = {}
  for (const tool of tools) {
    const schema = registered[tool.name].inputSchema
    // No schema: the SDK calls the handler without looking at the arguments.
    inputs[tool.name] = schema ? ir(schema) : null
  }
  for (const type of nodeConfig.NODE_TYPES) ir(nodeConfig.configSchemaByType[type])
  const sortedDefs = Object.fromEntries(Object.keys(defs).sort().map((k) => [k, defs[k]]))
  return {
    'tools.json': tools,
    'schemas.json': { tools: inputs, defs: sortedDefs },
    'workflow-nodes.json': WORKFLOW_NODES_REFERENCE
  }
}

const files = await generate()
const checking = process.argv.includes('--check')
let stale = false
for (const [name, value] of Object.entries(files)) {
  const file = path.join(OUT, name)
  const text = JSON.stringify(value, null, 2) + '\n'
  if (checking) {
    let current = ''
    try {
      current = readFileSync(file, 'utf8')
    } catch {
      // Missing reads as stale.
    }
    if (current !== text) {
      console.error(`${path.relative(root, file)} is out of date: run node scripts/gen-mcp-tools.mjs`)
      stale = true
    }
  } else {
    writeFileSync(file, text)
    console.log(`wrote ${path.relative(root, file)}`)
  }
}
// The server modules leave handles open (the database's watcher among them).
process.exit(stale ? 1 : 0)
