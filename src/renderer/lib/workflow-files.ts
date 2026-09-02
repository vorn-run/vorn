import type { SourceConnection, WorkflowDefinition } from '../../shared/types'
import {
  toPortable,
  fromPortable,
  importedWorkflowIdFor,
  residualAbsolutePaths,
  unresolvedRequirements,
  slugify,
  PORTABLE_FORMAT_VERSION,
  type PortableRequirement,
  type PortableWorkflow
} from '../../shared/workflow-portability'

/**
 * A workflow as a file, on the way out and on the way back.
 *
 * The reading and writing are deliberately split from the dialogs around them:
 * everything a person can get wrong — a file that is not JSON, a version this
 * build does not read, a connection this machine does not have — is decided by
 * a plain function a test can call, and only the file handles need a browser.
 */

/** Names the file for what it is, so a directory of them reads. */
export const WORKFLOW_FILE_SUFFIX = '.vorn-workflow.json'

export type ImportedWorkflowFile =
  | { ok: true; definition: WorkflowDefinition; unresolved: PortableRequirement[] }
  | { ok: false; error: string }

/**
 * The project a workflow's paths are relative to.
 *
 * Its own steps answer first. A workflow that names none still has paths worth
 * tokenizing — a script step carries a `cwd` without a `projectName` — so the
 * project in view stands in rather than leaving the export machine-specific.
 */
export function projectForWorkflow(
  workflow: WorkflowDefinition,
  projects: { name: string; path: string }[],
  activeProject?: string | null
): { name: string; path: string } | undefined {
  const named = workflow.nodes
    .map((node) => (node.config as Record<string, unknown>).projectName)
    .find((name): name is string => typeof name === 'string' && name.length > 0)
  return (
    projects.find((project) => project.name === named) ??
    projects.find((project) => project.name === activeProject)
  )
}

export function workflowFileName(workflow: WorkflowDefinition): string {
  return `${slugify(workflow.name)}${WORKFLOW_FILE_SUFFIX}`
}

/** The bytes to write, plus whatever refused to become portable. */
export function fileFromWorkflow(
  workflow: WorkflowDefinition,
  projectPath: string,
  connections: SourceConnection[]
): { name: string; contents: string; residual: string[] } {
  const portable = toPortable(workflow, projectPath, connections)
  return {
    name: workflowFileName(workflow),
    contents: `${JSON.stringify(portable, null, 2)}\n`,
    residual: residualAbsolutePaths(portable)
  }
}

/** As much text as a workflow can reasonably be, matching what an agent may hand in. */
export const MAX_WORKFLOW_FILE_BYTES = 500_000

/** Read a file back into a workflow for this machine, or say why it cannot be. */
export function definitionFromFile(
  text: string,
  project: { name: string; path: string },
  bundle: string,
  connections: SourceConnection[],
  existingWorkflows: Array<{ id: string; name: string }> = []
): ImportedWorkflowFile {
  if (text.length > MAX_WORKFLOW_FILE_BYTES) {
    return { ok: false, error: 'That file is too large to be a workflow' }
  }

  let parsed: PortableWorkflow
  try {
    parsed = JSON.parse(text)
  } catch {
    return { ok: false, error: 'That file is not valid JSON' }
  }

  if (parsed?.version !== PORTABLE_FORMAT_VERSION) {
    return {
      ok: false,
      error: `That file is version ${parsed?.version ?? 'unknown'}; this build reads version ${PORTABLE_FORMAT_VERSION}`
    }
  }
  if (!parsed.name || !Array.isArray(parsed.nodes) || !Array.isArray(parsed.edges)) {
    return { ok: false, error: 'That file is missing a name, nodes or edges' }
  }

  const graph = graphComplaint(parsed)
  if (graph) return { ok: false, error: graph }

  const portable = { ...parsed, slug: parsed.slug ?? slugify(parsed.name) }
  const definition = fromPortable(portable, bundle, project, connections)
  return {
    ok: true,
    definition: {
      ...definition,
      id: importedWorkflowIdFor(bundle, portable.slug, portable.name, existingWorkflows)
    },
    unresolved: unresolvedRequirements(portable, connections)
  }
}

/**
 * What is wrong with the graph, if anything.
 *
 * Array-ness alone is not a graph: `nodes: ["oops"]` reaches the canvas and the
 * engine as something neither can read, and an edge naming a node that is not
 * in the file draws from nowhere.
 */
function graphComplaint(portable: PortableWorkflow): string | null {
  const ids = new Set<string>()
  for (const node of portable.nodes) {
    const candidate = node as { id?: unknown; type?: unknown } | null
    if (typeof candidate?.id !== 'string' || typeof candidate.type !== 'string') {
      return 'That file has a step this build cannot read'
    }
    if (ids.has(candidate.id)) return 'That file carries the same step twice'
    ids.add(candidate.id)
  }

  for (const edge of portable.edges) {
    const candidate = edge as { source?: unknown; target?: unknown } | null
    if (typeof candidate?.source !== 'string' || typeof candidate.target !== 'string') {
      return 'That file has a connection this build cannot read'
    }
    if (!ids.has(candidate.source) || !ids.has(candidate.target)) {
      return 'That file connects a step it does not carry'
    }
  }

  return null
}

/**
 * Keep a re-imported workflow where it already lived, and as it was left.
 *
 * A workflow that was running keeps running; one arriving for the first time
 * stays off, because a file is a description rather than a decision to run.
 */
export function placeImportedWorkflow(
  definition: WorkflowDefinition,
  existing: WorkflowDefinition | undefined,
  activeWorkspace: string
): WorkflowDefinition {
  return {
    ...definition,
    enabled: existing ? existing.enabled : definition.enabled,
    workspaceId: existing?.workspaceId ?? activeWorkspace
  }
}

/** What an unbound step is still waiting for, in a sentence a person can act on. */
export function describeRequirement(requirement: PortableRequirement): string {
  if (requirement.kind === 'httpProfile') {
    return requirement.name ? `an HTTP profile like "${requirement.name}"` : 'an HTTP profile'
  }
  const connector = requirement.connectorId || 'connector'
  return requirement.name
    ? `a ${connector} connection like "${requirement.name}"`
    : `a ${connector} connection`
}

function isWorkflowFile(file: File): boolean {
  return file.name.toLowerCase().endsWith('.json')
}

/** Refuses by size before reading, so a huge file is never held in memory. */
async function readFile(file: File): Promise<{ name: string; contents: string }> {
  if (file.size > MAX_WORKFLOW_FILE_BYTES) {
    throw new Error('That file is too large to be a workflow')
  }
  return { name: file.name, contents: await file.text() }
}

/** The first workflow file in a drop, ignoring anything else that came with it. */
export async function readDroppedWorkflowFile(
  files: FileList | null
): Promise<{ name: string; contents: string } | null> {
  const file = Array.from(files ?? []).find(isWorkflowFile)
  return file ? readFile(file) : null
}

/**
 * Ask for a file and hand back its text.
 *
 * A renderer file input rather than a dialog channel: it opens the native
 * picker in the desktop app and the browser's own in server mode, and both
 * give back the contents, which is the thing that gets imported.
 */
export function pickWorkflowFile(): Promise<{ name: string; contents: string } | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.json,application/json'
    input.style.display = 'none'
    document.body.appendChild(input)

    let settled = false
    const done = (): boolean => {
      if (settled) return true
      settled = true
      input.remove()
      window.removeEventListener('focus', onFocus)
      return false
    }
    const finish = (value: { name: string; contents: string } | null): void => {
      if (!done()) resolve(value)
    }
    const fail = (err: unknown): void => {
      if (!done()) reject(err)
    }

    // Not every browser fires `cancel`, and a picker that closed with nothing
    // chosen would otherwise leave this promise — and the input — hanging.
    function onFocus(): void {
      setTimeout(() => {
        if (!input.files?.length) finish(null)
      }, 300)
    }

    input.addEventListener('change', () => {
      const file = input.files?.[0]
      if (!file) return finish(null)
      void readFile(file).then(finish, fail)
    })
    input.addEventListener('cancel', () => finish(null))
    window.addEventListener('focus', onFocus)
    input.click()
  })
}
