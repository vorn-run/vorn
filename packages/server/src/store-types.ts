/**
 * The store's own shapes: what its calls take and give back beyond the
 * records in `@vornrun/shared/types`.
 *
 * Both stores read these. The native one gets them as serde types generated
 * from this file and the shared types (`scripts/gen-store-schema.mjs`), so a
 * change here has to be regenerated, which a test checks.
 */
import type {
  ArtifactAnchor,
  ArtifactComment,
  ArtifactKind,
  ConnectorItemContext
} from '@vornrun/shared/types'

export interface ConnectorInboxItem {
  id: number
  leaseToken: string
  workflowId: string
  connectionId: string
  connectorId: string
  eventId: string
  eventType: string
  eventTimestamp: string
  connectorItem: ConnectorItemContext
  attempts: number
}

/**
 * What verification needs, and nothing else.
 *
 * Deliberately NOT a superset of `DeviceToken`. An `extends DeviceToken` shape
 * would be structurally assignable to it, so returning one from a handler typed
 * `DeviceToken` would compile and ship `tokenHash` over the wire — and
 * `./database` is an export of this package, so `packages/mcp` can reach it.
 * Keeping the shapes incompatible makes the compiler enforce what would
 * otherwise be a comment.
 */
export interface DeviceTokenSecret {
  id: string
  userId: string
  tokenHash: string
  revokedAt: string | null
}

/** Fields needed to persist a new token. */
export interface NewDeviceToken {
  id: string
  userId: string
  name: string
  tokenHash: string
  createdAt: string
}

export interface NewArtifact {
  kind: ArtifactKind
  title: string
  sessionId: string | null
  projectName: string | null
  gateRunId?: string
  gateNodeId?: string
}

/** One webhook request, which becomes one durable inbox row. */
export interface WebhookEvent {
  workflowId: string
  eventId: string
  receivedAt: string
  item: ConnectorItemContext
}

/** One remote page of a connector poll and the cursor after it. */
export interface ConnectorPollPage {
  workflowId: string
  connectionId: string
  connectorId: string
  cursor?: string
  polledAt: string
  events: Array<{
    eventId: string
    eventType: string
    eventTimestamp: string
    connectorItem: ConnectorItemContext
  }>
}

export interface ConnectorPollError {
  workflowId: string
  connectionId: string
  error: string
  polledAt: string
}

export interface ConnectorInboxClaim {
  now: string
  leaseUntil: string
  limit: number
}

export interface ConnectorInboxRetry {
  id: number
  leaseToken: string
  error: string
  now: string
}

export interface ArtifactFilter {
  sessionId?: string
  projectName?: string
}

export interface ArtifactCommentFilter {
  version?: number
  state?: ArtifactComment['state']
  batchId?: string
}

export interface NewArtifactComment {
  artifactId: string
  version: number
  anchor: ArtifactAnchor | null
  body: string
}

export interface ArtifactCommentChange {
  body?: string
  anchor?: ArtifactAnchor | null
}
