import { connectionConnectorId } from './types'
import type { SourceConnection, WorkflowDefinition, WorkflowNode } from './types'

/**
 * Making a workflow portable, and putting it back.
 *
 * A workflow is stored with absolute paths baked into its node configs, so it
 * describes one directory on one machine. That is why a workflow cannot be
 * committed beside the code it drives, reviewed in a diff, or run by anyone
 * else. Export rewrites the machine-specific parts to placeholders; import
 * resolves them against a named project.
 *
 * Node configs are an untyped passthrough (`config: z.record(...)`), so there
 * is no flat list of paths to walk — every rewrite has to know the node type it
 * is looking at. That is the whole reason this file exists rather than a
 * two-line `JSON.stringify`.
 *
 * Connections are the other machine-specific thing, and they are handled
 * differently from paths: a connection id cannot be rewritten to a placeholder
 * because there is nothing on the far side to resolve it against. Export drops
 * the id and records what it stood for; import rebinds it when this machine has
 * an unambiguous answer, and leaves the step unconfigured when it does not.
 */

/** Stands in for the importing project's directory. */
export const PROJECT_PATH_TOKEN = '{{project.path}}'
/** Stands in for the importing project's name. */
export const PROJECT_NAME_TOKEN = '{{project.name}}'

export const PORTABLE_FORMAT_VERSION = 1

/** The built-in connector whose connections are HTTP auth profiles. */
export const HTTP_PROFILE_CONNECTOR = 'http'

/** What a step needs bound before it can run, named so any machine can match it. */
export type PortableRequirement =
  | {
      kind: 'connection'
      nodeId: string
      /** Empty when the exporting machine could not name the connector. */
      connectorId: string
      name: string
      event?: string
    }
  | { kind: 'httpProfile'; nodeId: string; name: string }

export type PortableWorkflow = {
  version: number
  slug: string
  name: string
  icon?: string
  iconColor?: string
  staggerDelayMs?: number
  /** Absent when nothing in the workflow was bound to a connection. */
  requires?: PortableRequirement[]
  nodes: WorkflowNode[]
  edges: WorkflowDefinition['edges']
}

/** Enough of a connection to match one; a whole `SourceConnection` satisfies it. */
export interface PortableConnection {
  id: string
  name: string
  connectorId: string
  filters?: SourceConnection['filters']
}

/**
 * Id for an imported workflow, derived from portable inputs only.
 *
 * `connectorSeededWorkflowId` is the pattern being followed — derive, look up,
 * update in place — but its input is a locally generated UUID, so the same
 * connector on two machines produces two different ids. Deriving from the
 * bundle and slug instead means re-importing anywhere updates the workflow it
 * already created rather than adding a second copy.
 */
export function importedWorkflowId(bundle: string, slug: string): string {
  return `import:${bundle}:${slug}`
}

/**
 * The id an import should take, given what this machine already has.
 *
 * Deriving from the slug is what makes re-importing a file update the workflow
 * it made last time. Two different names can slugify the same, though — "Deploy!"
 * and "Deploy?" — and the second would silently replace the first. A derived id
 * already held by a workflow of another name steps aside instead.
 */
export function importedWorkflowIdFor(
  bundle: string,
  slug: string,
  name: string,
  existing: Array<{ id: string; name: string }>
): string {
  for (let attempt = 1; ; attempt++) {
    const id = importedWorkflowId(bundle, attempt === 1 ? slug : `${slug}-${attempt}`)
    const held = existing.find((workflow) => workflow.id === id)
    if (!held || held.name === name) return id
  }
}

export function parseImportedWorkflowId(id: string): { bundle: string; slug: string } | null {
  if (!id.startsWith('import:')) return null
  const rest = id.slice('import:'.length)
  const colon = rest.indexOf(':')
  if (colon === -1) return null
  return { bundle: rest.slice(0, colon), slug: rest.slice(colon + 1) }
}

export function slugify(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 60) || 'workflow'
  )
}

/** The connector a connection belongs to, packaged connectors included. */
function connectorOf(connection: PortableConnection): string {
  return connectionConnectorId({
    connectorId: connection.connectorId,
    filters: connection.filters ?? {}
  })
}

/** Whether this node runs against a connector connection. */
export function boundConnectionKey(
  node: WorkflowNode,
  config: Record<string, unknown>
): string | null {
  if (node.type === 'trigger' && config.triggerType === 'connectorPoll') return 'connectionId'
  if (node.type === 'callConnectorAction') return 'connectionId'
  if (node.type === 'httpRequest') return 'profileConnectionId'
  if (node.type === 'script') return 'secretsFrom'
  return null
}

/** Keys a step may simply not have: unset is a finished step, not a question. */
const OPTIONAL_CONNECTION_KEYS = new Set(['profileConnectionId', 'secretsFrom'])

/**
 * The connection a requirement should bind to here, if this machine has one answer.
 *
 * A name match wins over a bare type match, so re-importing where the file was
 * written rebinds what it was actually pointed at. Anything ambiguous is left
 * for a person, because guessing binds a workflow to the wrong account.
 */
export function resolveRequirement(
  requirement: PortableRequirement,
  connections: PortableConnection[]
): string | undefined {
  const candidates = connections.filter((connection) =>
    requirement.kind === 'httpProfile'
      ? connectorOf(connection) === HTTP_PROFILE_CONNECTOR
      : requirement.connectorId !== '' && connectorOf(connection) === requirement.connectorId
  )
  if (candidates.length === 0) return undefined
  if (requirement.name !== '') {
    const named = candidates.filter((connection) => connection.name === requirement.name)
    if (named.length === 1) return named[0].id
  }
  return candidates.length === 1 ? candidates[0].id : undefined
}

/** Replace this machine's paths with placeholders and unbind its connections. */
export function toPortable(
  workflow: WorkflowDefinition,
  projectPath: string,
  connections: PortableConnection[] = []
): PortableWorkflow {
  const slug = slugify(workflow.name)
  const requires: PortableRequirement[] = []

  const nodes = workflow.nodes.map((node) => {
    const config = { ...(node.config as Record<string, unknown>) }

    // The token is the only thing guarding this install's hook route, so it
    // does not belong in a file meant to be committed and shared.
    if (node.type === 'trigger' && config.triggerType === 'webhook') config.token = ''

    if (node.type === 'launchAgent' || node.type === 'script') {
      for (const key of ['projectPath', 'cwd', 'existingWorktreePath']) {
        const value = config[key]
        if (typeof value === 'string' && value) {
          config[key] = replacePath(value, projectPath)
        }
      }
      if (typeof config.projectName === 'string' && config.projectName) {
        config[`projectName`] = PROJECT_NAME_TOKEN
      }
      // Points at a host registered in this install's remote_hosts table; the
      // id means nothing elsewhere, so the import runs locally rather than
      // against a host the importer never configured.
      delete config.remoteHostId
    }

    const key = boundConnectionKey(node, config)
    const bound = key === null ? '' : config[key]
    // A step bound to a connection here, and one that was never bound at all, both arrive elsewhere needing the same.
    const unbound = key !== null && !OPTIONAL_CONNECTION_KEYS.has(key) && bound === ''
    if (key !== null && ((typeof bound === 'string' && bound !== '') || unbound)) {
      const source = connections.find((connection) => connection.id === bound)
      const event = config.event
      const declared = config.connectorId
      requires.push(
        key === 'profileConnectionId'
          ? { kind: 'httpProfile', nodeId: node.id, name: source?.name ?? '' }
          : {
              kind: 'connection',
              nodeId: node.id,
              connectorId: source
                ? connectorOf(source)
                : typeof declared === 'string'
                  ? declared
                  : '',
              name: source?.name ?? '',
              ...(typeof event === 'string' && event !== '' && { event })
            }
      )
      // Optional on the node, so the step reads as simply having no key.
      if (OPTIONAL_CONNECTION_KEYS.has(key)) delete config[key]
      else config[key] = ''
    }

    return { ...node, config } as WorkflowNode
  })

  return {
    version: PORTABLE_FORMAT_VERSION,
    slug,
    name: workflow.name,
    ...(workflow.icon && { icon: workflow.icon }),
    ...(workflow.iconColor && { iconColor: workflow.iconColor }),
    ...(workflow.staggerDelayMs !== undefined && { staggerDelayMs: workflow.staggerDelayMs }),
    ...(requires.length > 0 && { requires }),
    nodes,
    edges: workflow.edges
  }
}

/**
 * Compare paths without caring which separator the machine writes.
 *
 * Vorn ships a Windows installer, and a workflow authored there carries
 * `C:\\Users\\...` in its node configs. Exact POSIX string matching would
 * leave those untouched and the export would claim to be portable while
 * naming a directory on one laptop.
 */
function normalizeForCompare(p: string): string {
  return p.replace(/\\/g, '/').replace(/\/+$/, '')
}

function replacePath(value: string, projectPath: string): string {
  const v = normalizeForCompare(value)
  const root = normalizeForCompare(projectPath)
  // No project to be relative to: leaving the path alone beats rewriting every
  // absolute path as if it sat inside an empty root.
  if (root === '') return value
  if (v === root) return PROJECT_PATH_TOKEN
  // A path inside the project keeps its tail, so a worktree or a subdirectory
  // survives the round trip instead of collapsing onto the project root.
  if (v.startsWith(`${root}/`)) return `${PROJECT_PATH_TOKEN}/${v.slice(root.length + 1)}`
  return value
}

/** Which of this file's requirements this machine cannot answer on its own. */
export function unresolvedRequirements(
  portable: PortableWorkflow,
  connections: PortableConnection[]
): PortableRequirement[] {
  const present = new Set(portable.nodes.map((node) => node.id))
  return (portable.requires ?? []).filter(
    (requirement) =>
      present.has(requirement.nodeId) && resolveRequirement(requirement, connections) === undefined
  )
}

/** Resolve placeholders against the project this workflow is being imported into. */
export function fromPortable(
  portable: PortableWorkflow,
  bundle: string,
  project: { name: string; path: string },
  connections: PortableConnection[] = [],
  mintToken: () => string = () => crypto.randomUUID()
): WorkflowDefinition {
  const bindings = new Map<string, PortableRequirement[]>()
  for (const requirement of portable.requires ?? []) {
    bindings.set(requirement.nodeId, [...(bindings.get(requirement.nodeId) ?? []), requirement])
  }

  const nodes = portable.nodes.map((node) => {
    const config = { ...(node.config as Record<string, unknown>) }

    // The id names a row in the writer's table, so only a requirement this
    // machine resolved below may bind one; a carried id is dropped rather than
    // handing the step whichever key happens to hold it here.
    if (node.type === 'script') delete config.secretsFrom

    for (const [key, value] of Object.entries(config)) {
      if (typeof value !== 'string') continue
      config[key] = value
        // Trailing separator of either flavour: a Windows project path ending
        // in a backslash would otherwise produce a doubled separator.
        .split(PROJECT_PATH_TOKEN)
        .join(project.path.replace(/[/\\]+$/, ''))
        .split(PROJECT_NAME_TOKEN)
        .join(project.name)
    }

    for (const requirement of bindings.get(node.id) ?? []) {
      const resolved = resolveRequirement(requirement, connections)
      if (resolved === undefined) continue
      if (requirement.kind === 'httpProfile') config.profileConnectionId = resolved
      else if (node.type === 'script') config.secretsFrom = resolved
      else config.connectionId = resolved
    }

    // Export blanks it; this install gets a hook secret of its own.
    if (node.type === 'trigger' && config.triggerType === 'webhook' && !config.token) {
      config.token = mintToken()
    }

    return { ...node, config } as WorkflowNode
  })

  return {
    id: importedWorkflowId(bundle, portable.slug),
    name: portable.name,
    icon: portable.icon ?? 'Zap',
    iconColor: portable.iconColor ?? '#6366f1',
    // A file cannot ask to be running: a dropped cron workflow would start
    // firing before anyone had read it. Callers restore what they had.
    enabled: false,
    ...(portable.staggerDelayMs !== undefined && { staggerDelayMs: portable.staggerDelayMs }),
    nodes,
    edges: portable.edges
  }
}

/**
 * What is still machine-specific after export. Empty is the goal.
 *
 * Windows shapes are checked too — a drive letter or a UNC share — because a
 * detector that only knows POSIX reports a Windows-authored workflow as
 * portable when it is not, which is worse than not checking at all.
 */
const MACHINE_PATH = /(^|["'\s])(\/(Users|home)\/|[A-Za-z]:[\\/]|\\\\[^\\/\s]+[\\/])/

export function residualAbsolutePaths(portable: PortableWorkflow): string[] {
  const found: string[] = []
  for (const node of portable.nodes) {
    for (const [key, value] of Object.entries(node.config as Record<string, unknown>)) {
      if (typeof value === 'string' && MACHINE_PATH.test(value)) {
        found.push(`${node.id}.${key}`)
      }
    }
  }
  return found
}
