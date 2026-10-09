/** The store's own shapes, generated into vorn-store's types by `scripts/gen-store-schema.mjs`. */
import type { ArtifactAnchor, ArtifactComment, ArtifactKind, ConnectorItemContext } from './types'

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

/** What verification needs: not a superset of `DeviceToken`, so `tokenHash` cannot pass for one. */
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
