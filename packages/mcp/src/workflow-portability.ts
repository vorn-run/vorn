import type { WorkflowDefinition, WorkflowNode } from '@vornrun/shared/types'

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
 */

/** Stands in for the importing project's directory. */
export const PROJECT_PATH_TOKEN = '{{project.path}}'
/** Stands in for the importing project's name. */
export const PROJECT_NAME_TOKEN = '{{project.name}}'

export const PORTABLE_FORMAT_VERSION = 1

export type PortableWorkflow = {
  version: number
  slug: string
  name: string
  icon?: string
  iconColor?: string
  staggerDelayMs?: number
  nodes: WorkflowNode[]
  edges: WorkflowDefinition['edges']
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

/**
 * Why a workflow cannot travel, if it cannot.
 *
 * A connector-bound workflow carries a `connectionId` that is a UUID minted on
 * this machine. Rewriting it to a placeholder would produce a file that
 * imports cleanly and then fails at run time against a connection that does
 * not exist — so this refuses instead, the way install_connector refuses
 * secrets rather than guessing at them.
 */
export function portabilityBlockers(workflow: WorkflowDefinition): string[] {
  const blockers: string[] = []

  for (const node of workflow.nodes) {
    const config = node.config as Record<string, unknown>

    if (node.type === 'trigger' && config.triggerType === 'connectorPoll') {
      blockers.push(`the trigger polls a connector connection, which exists only on this machine`)
    }
    if (node.type === 'callConnectorAction') {
      blockers.push(`step "${node.label}" calls a connector action bound to a local connection`)
    }
  }

  return blockers
}

/** Replace this machine's paths with placeholders. */
export function toPortable(workflow: WorkflowDefinition, projectPath: string): PortableWorkflow {
  const slug = slugify(workflow.name)

  const nodes = workflow.nodes.map((node) => {
    const config = { ...(node.config as Record<string, unknown>) }

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

    return { ...node, config } as WorkflowNode
  })

  return {
    version: PORTABLE_FORMAT_VERSION,
    slug,
    name: workflow.name,
    ...(workflow.icon && { icon: workflow.icon }),
    ...(workflow.iconColor && { iconColor: workflow.iconColor }),
    ...(workflow.staggerDelayMs !== undefined && { staggerDelayMs: workflow.staggerDelayMs }),
    nodes,
    edges: workflow.edges
  }
}

function replacePath(value: string, projectPath: string): string {
  if (value === projectPath) return PROJECT_PATH_TOKEN
  // A path inside the project keeps its tail, so a worktree or a subdirectory
  // survives the round trip instead of collapsing onto the project root.
  const prefix = projectPath.endsWith('/') ? projectPath : `${projectPath}/`
  if (value.startsWith(prefix)) return PROJECT_PATH_TOKEN + '/' + value.slice(prefix.length)
  return value
}

/** Resolve placeholders against the project this workflow is being imported into. */
export function fromPortable(
  portable: PortableWorkflow,
  bundle: string,
  project: { name: string; path: string }
): WorkflowDefinition {
  const nodes = portable.nodes.map((node) => {
    const config = { ...(node.config as Record<string, unknown>) }

    for (const [key, value] of Object.entries(config)) {
      if (typeof value !== 'string') continue
      config[key] = value
        .split(PROJECT_PATH_TOKEN)
        .join(project.path.replace(/\/$/, ''))
        .split(PROJECT_NAME_TOKEN)
        .join(project.name)
    }

    return { ...node, config } as WorkflowNode
  })

  return {
    id: importedWorkflowId(bundle, portable.slug),
    name: portable.name,
    icon: portable.icon ?? 'Zap',
    iconColor: portable.iconColor ?? '#6366f1',
    enabled: true,
    ...(portable.staggerDelayMs !== undefined && { staggerDelayMs: portable.staggerDelayMs }),
    nodes,
    edges: portable.edges
  }
}

/** What is still machine-specific after export. Empty is the goal. */
export function residualAbsolutePaths(portable: PortableWorkflow): string[] {
  const found: string[] = []
  for (const node of portable.nodes) {
    for (const [key, value] of Object.entries(node.config as Record<string, unknown>)) {
      if (typeof value === 'string' && /(^|["'\s])\/(Users|home)\//.test(value)) {
        found.push(`${node.id}.${key}`)
      }
    }
  }
  return found
}
