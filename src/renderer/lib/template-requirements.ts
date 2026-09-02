import type {
  ConnectorManifest,
  SourceConnection,
  WorkflowEdge,
  WorkflowNode,
  WorkflowTemplate
} from '../../shared/types'
import { connectionConnectorId } from '../../shared/types'
import {
  fromPortable,
  resolveRequirement,
  unresolvedRequirements,
  type PortableRequirement
} from '../../shared/workflow-portability'
import type { ConnectorListing } from './connector-browse'
import { canAddConnection, packStateFor } from './pack-status'

/**
 * What a template needs before it will run here, and what it becomes when used.
 *
 * Kept apart from the panel that draws it: whether a connection answers a
 * requirement is a question about this machine's data, not about a list, and
 * the answer decides both what the rows say and what lands on the canvas.
 */

export interface TemplateRequirement {
  requirement: PortableRequirement
  /** The connection this machine will bind, when it has exactly one answer. */
  connectionId?: string
}

export interface TemplateSeed {
  name: string
  icon?: string
  iconColor?: string
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  /** Steps that landed unbound, so the editor can say what is still wanted. */
  unresolved: PortableRequirement[]
}

/** A workflow a connected connector already knows how to build. */
export interface ConnectorSuggestion {
  key: string
  connectionId: string
  connectionName: string
  event: string
  name: string
}

/**
 * The workflows this machine's connections ship with.
 *
 * These are not templates: the server builds them from the connector's own
 * manifest against a connection that already exists, so they are offered
 * beside the templates and taken by a different route.
 */
export function connectorSuggestions(
  connections: SourceConnection[],
  connectors: Array<{ id: string; manifest: ConnectorManifest }>
): ConnectorSuggestion[] {
  const manifests = new Map(connectors.map((connector) => [connector.id, connector.manifest]))
  return connections.flatMap((connection) => {
    const manifest = manifests.get(connectionConnectorId(connection))
    return (manifest?.defaultWorkflows ?? []).map((seeded) => ({
      key: `${connection.id}:${seeded.event}`,
      connectionId: connection.id,
      connectionName: connection.name,
      event: seeded.event,
      name: seeded.name
    }))
  })
}

/**
 * What can be done about a requirement this machine cannot answer yet.
 *
 * A row that only names what is missing sends someone to Settings and back; the
 * same row can carry the fix, and every one of these routes already exists.
 */
export type RequirementAction =
  | { kind: 'install'; listing: ConnectorListing }
  | { kind: 'addConnection'; listing: ConnectorListing }
  | { kind: 'createProfile'; name: string }
  | { kind: 'none' }

export function requirementAction(
  requirement: TemplateRequirement,
  listings: ConnectorListing[]
): RequirementAction {
  if (requirement.connectionId !== undefined) return { kind: 'none' }
  // The HTTP connector is built in, so a profile is always one form away.
  if (requirement.requirement.kind === 'httpProfile') {
    return { kind: 'createProfile', name: requirement.requirement.name }
  }

  const connectorId = requirement.requirement.connectorId
  // An export whose connection was already gone names no connector to offer.
  if (connectorId === '') return { kind: 'none' }
  const listing = listings.find((candidate) => candidate.id === connectorId)
  if (!listing) return { kind: 'none' }

  const state = packStateFor({ installed: listing.pack })
  const route = { source: listing.source, hasLegacyLaunch: !!listing.catalogItem?.packageName }
  return canAddConnection(state, route)
    ? { kind: 'addConnection', listing }
    : { kind: 'install', listing }
}

export function templateRequirements(
  template: WorkflowTemplate,
  connections: SourceConnection[]
): TemplateRequirement[] {
  return (template.portable.requires ?? []).map((requirement) => {
    const connectionId = resolveRequirement(requirement, connections)
    return connectionId === undefined ? { requirement } : { requirement, connectionId }
  })
}

/** Whether every connection this template names is already answered here. */
export function templateIsReady(
  template: WorkflowTemplate,
  connections: SourceConnection[]
): boolean {
  return unresolvedRequirements(template.portable, connections).length === 0
}

/**
 * The canvas a template starts you with.
 *
 * A published token would be the same secret on every machine that ever used
 * the template, so a webhook trigger is given its own here instead.
 */
export function templateSeed(
  template: WorkflowTemplate,
  project: { name: string; path: string } | undefined,
  connections: SourceConnection[],
  mintToken: () => string = () => crypto.randomUUID()
): TemplateSeed {
  const definition = fromPortable(
    template.portable,
    'template',
    project ?? { name: '', path: '' },
    connections
  )

  const nodes = definition.nodes.map((node) => {
    const config = node.config as Record<string, unknown>
    if (config.triggerType !== 'webhook') return node
    // Always this machine's own token: one the catalog carried would be a
    // secret shared with everyone who ever used the template.
    return { ...node, config: { ...config, token: mintToken() } } as WorkflowNode
  })

  return {
    name: template.name,
    ...(definition.icon && { icon: definition.icon }),
    ...(definition.iconColor && { iconColor: definition.iconColor }),
    nodes,
    edges: definition.edges,
    unresolved: unresolvedRequirements(template.portable, connections)
  }
}
