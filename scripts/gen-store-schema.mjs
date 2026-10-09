// Writes the JSON Schema the native store's serde types are generated from:
// the records in packages/shared/src/types.ts and the store's own shapes in
// packages/shared/src/store-types.ts. The vorn-protocol crate turns it into
// Rust when it builds, so this is the only step that reads TypeScript.
//
//   node scripts/gen-store-schema.mjs           write the schema
//   node scripts/gen-store-schema.mjs --check   fail if it is out of date
//
// Two rules make the types fit a store rather than a validator:
//
// - A column that holds JSON the store writes and reads back whole (a
//   workflow's nodes, a run's inputs) is any value. The store never looks
//   inside, and a typed field would drop whatever the type does not name.
// - String unions are plain strings. The TypeScript store casts what it reads
//   and stores what it is given, so a status this build does not know is
//   still a row, not an error.
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createGenerator } from 'ts-json-schema-generator'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const OUT = path.join(root, 'packages/core/crates/protocol/schema/store.json')

const SOURCES = [
  {
    path: 'packages/shared/src/types.ts',
    types: [
      'AgentCommandConfig',
      'Artifact',
      'ArtifactComment',
      'ArtifactVersion',
      'ConnectorItemContext',
      'DeviceToken',
      'NodeExecutionState',
      'ProjectConfig',
      'RemoteHost',
      'ScheduleLogEntry',
      'SessionEvent',
      'SessionGroupConfig',
      'SourceConnection',
      'SSHKey',
      'SSHKeyMeta',
      'TaskConfig',
      'TaskSourceLink',
      'TerminalSession',
      'User',
      'WorkflowDefinition',
      'WorkflowExecution',
      'WorkspaceConfig'
    ]
  },
  {
    path: 'packages/shared/src/store-types.ts',
    types: [
      'ArtifactCommentChange',
      'ArtifactCommentFilter',
      'ArtifactFilter',
      'ConnectorInboxClaim',
      'ConnectorInboxItem',
      'ConnectorInboxRetry',
      'ConnectorPollError',
      'ConnectorPollPage',
      'DeviceTokenSecret',
      'NewArtifact',
      'NewArtifactComment',
      'NewDeviceToken',
      'WebhookEvent'
    ]
  }
]

/** Fields kept as any JSON value: stored as JSON text, or passed through whole. */
const OPAQUE = {
  AgentCommandConfig: ['args', 'headlessArgs', 'fallbackArgs'],
  ArtifactComment: ['anchor'],
  ArtifactCommentChange: ['anchor'],
  ConnectorInboxItem: ['connectorItem'],
  NewArtifactComment: ['anchor'],
  NodeExecutionState: ['structuredOutput', 'feedback'],
  ProjectConfig: ['preferredAgents', 'hostIds'],
  SessionEvent: ['metadata'],
  SourceConnection: ['filters', 'statusMapping'],
  WebhookEvent: ['item'],
  WorkflowDefinition: ['nodes', 'edges'],
  WorkflowExecution: ['inputs', 'connectorItem', 'definition', 'triggerSession']
}

/** The page's events carry connector items the same way. */
const OPAQUE_NESTED = { ConnectorPollPage: { events: ['connectorItem'] } }

function generate() {
  const definitions = {}
  for (const source of SOURCES) {
    const generator = createGenerator({
      path: path.join(root, source.path),
      tsconfig: path.join(root, 'packages/shared/tsconfig.json'),
      skipTypeCheck: true,
      additionalProperties: true,
      expose: 'export',
      topRef: true,
      jsDoc: 'basic',
      sortProps: true
    })
    for (const type of source.types) {
      const schema = generator.createSchema(type)
      Object.assign(definitions, schema.definitions ?? {})
    }
  }

  for (const [type, fields] of Object.entries(OPAQUE)) {
    for (const field of fields) {
      const props = definitions[type]?.properties
      if (!props?.[field]) throw new Error(`${type}.${field} is not in the schema`)
      const { description } = props[field]
      props[field] = description === undefined ? {} : { description }
    }
  }
  for (const [type, nested] of Object.entries(OPAQUE_NESTED)) {
    for (const [field, inner] of Object.entries(nested)) {
      const items = definitions[type]?.properties?.[field]?.items
      if (!items?.properties) throw new Error(`${type}.${field} has no item shape`)
      for (const name of inner) items.properties[name] = {}
    }
  }

  const reachable = new Set()
  const visit = (node) => {
    if (Array.isArray(node)) return node.forEach(visit)
    if (!node || typeof node !== 'object') return
    if (typeof node.$ref === 'string') {
      const name = decodeURIComponent(node.$ref.replace('#/definitions/', ''))
      if (!reachable.has(name)) {
        reachable.add(name)
        visit(definitions[name])
      }
    }
    Object.values(node).forEach(visit)
  }
  for (const source of SOURCES) {
    for (const type of source.types) {
      reachable.add(type)
      visit(definitions[type])
    }
  }

  const kept = {}
  for (const name of [...reachable].sort()) kept[name] = openStrings(definitions[name])
  // Then a union of strings and string types (`AiAgentType | 'shell'`) is a string too.
  const isString = (node) =>
    node?.type === 'string' ||
    (typeof node?.$ref === 'string' &&
      kept[decodeURIComponent(node.$ref.replace('#/definitions/', ''))]?.type === 'string')
  const collapse = (node) => {
    if (Array.isArray(node)) return node.map(collapse)
    if (!node || typeof node !== 'object') return node
    if (Array.isArray(node.anyOf) && node.anyOf.every(isString)) {
      const { anyOf: _anyOf, ...rest } = node
      return { ...rest, type: 'string' }
    }
    const out = {}
    for (const [key, value] of Object.entries(node)) out[key] = collapse(value)
    return out
  }
  for (const name of Object.keys(kept)) kept[name] = collapse(kept[name])
  return { $schema: 'http://json-schema.org/draft-07/schema#', definitions: kept }
}

/** A string union becomes a string; anything else is kept as it is. */
function openStrings(node) {
  if (Array.isArray(node)) return node.map(openStrings)
  if (!node || typeof node !== 'object') return node
  if (node.type === 'string' && (node.enum || node.const !== undefined)) {
    const { enum: _enum, const: _const, ...rest } = node
    return rest
  }
  const out = {}
  for (const [key, value] of Object.entries(node)) out[key] = openStrings(value)
  return out
}

const text = JSON.stringify(generate(), null, 2) + '\n'
if (process.argv.includes('--check')) {
  let current = ''
  try {
    current = readFileSync(OUT, 'utf8')
  } catch {
    // Missing reads as stale.
  }
  if (current !== text) {
    console.error(
      `${path.relative(root, OUT)} is out of date: run node scripts/gen-store-schema.mjs`
    )
    process.exit(1)
  }
} else {
  writeFileSync(OUT, text)
  console.log(`wrote ${path.relative(root, OUT)}`)
}
