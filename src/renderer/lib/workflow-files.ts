import type { SourceConnection, WorkflowDefinition } from '../../shared/types'
import {
  toPortable,
  fromPortable,
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

/** The project a workflow's own steps point at, which its paths are relative to. */
export function projectForWorkflow(
  workflow: WorkflowDefinition,
  projects: { name: string; path: string }[]
): { name: string; path: string } | undefined {
  const named = workflow.nodes
    .map((node) => (node.config as Record<string, unknown>).projectName)
    .find((name): name is string => typeof name === 'string' && name.length > 0)
  return projects.find((project) => project.name === named)
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

/** Read a file back into a workflow for this machine, or say why it cannot be. */
export function definitionFromFile(
  text: string,
  project: { name: string; path: string },
  bundle: string,
  connections: SourceConnection[]
): ImportedWorkflowFile {
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

  const portable = { ...parsed, slug: parsed.slug ?? slugify(parsed.name) }
  return {
    ok: true,
    definition: fromPortable(portable, bundle, project, connections),
    unresolved: unresolvedRequirements(portable, connections)
  }
}

/** Keep a re-imported workflow where it already lived; a new one joins the workspace in view. */
export function placeImportedWorkflow(
  definition: WorkflowDefinition,
  existing: WorkflowDefinition | undefined,
  activeWorkspace: string
): WorkflowDefinition {
  return { ...definition, workspaceId: existing?.workspaceId ?? activeWorkspace }
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

async function readFile(file: File): Promise<{ name: string; contents: string }> {
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
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.json,application/json'
    input.style.display = 'none'
    document.body.appendChild(input)

    let settled = false
    const finish = (value: { name: string; contents: string } | null): void => {
      if (settled) return
      settled = true
      input.remove()
      resolve(value)
    }

    input.addEventListener('change', () => {
      const file = input.files?.[0]
      if (!file) return finish(null)
      void readFile(file).then(finish, () => finish(null))
    })
    input.addEventListener('cancel', () => finish(null))
    input.click()
  })
}
