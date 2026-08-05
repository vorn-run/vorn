import { envNameFor } from './define'
import type { Connector } from './types'

/** MCP tool name a trigger is served under. */
export function pollToolName(triggerType: string): string {
  return `poll_${triggerType}`
}

/** Tool that reports the connector's manifest and setup hints. */
export const MANIFEST_TOOL = 'vorn_connector_manifest'

export interface ConnectionSetup {
  connectorId: string
  triggerType: string
  /** Values to paste into Vorn's MCP connection form. */
  filters: {
    pollTool: string
    itemsPath: 'items'
    idField: 'externalId'
    timestampField: 'updatedAt'
    titleField: 'title'
    urlField: 'url'
  }
  /** Environment variable names the connector reads. */
  env: Array<{ name: string; required: boolean; secret: boolean; description?: string }>
}

/**
 * Describe how to wire one trigger into a Vorn MCP connection.
 *
 * Every SDK connector normalizes to the same field names, so this mapping is
 * fixed; it is generated rather than documented so a rename in the SDK cannot
 * drift away from the setup instructions users copy.
 */
export function connectionSetup(connector: Connector, triggerType: string): ConnectionSetup {
  const trigger = connector.triggers.find((entry) => entry.type === triggerType)
  if (!trigger) {
    throw new Error(`Connector ${connector.id} has no trigger "${triggerType}"`)
  }
  return {
    connectorId: connector.id,
    triggerType,
    filters: {
      pollTool: pollToolName(triggerType),
      itemsPath: 'items',
      idField: 'externalId',
      timestampField: 'updatedAt',
      titleField: 'title',
      urlField: 'url'
    },
    env: connector.config.map((field) => ({
      name: envNameFor(field.key, field.env),
      required: field.required === true,
      secret: field.secret === true,
      ...(field.description !== undefined && { description: field.description })
    }))
  }
}

export interface ConnectorManifest {
  id: string
  name: string
  version: string
  description?: string
  triggers: Array<{ type: string; label: string; description?: string; setup: ConnectionSetup }>
  actions: Array<{
    type: string
    label: string
    description?: string
    inputs: Array<{ key: string; label: string; type: string; required: boolean }>
  }>
}

/** Full machine-readable description of a connector, served over MCP and printed by the CLI. */
export function connectorManifest(connector: Connector): ConnectorManifest {
  return {
    id: connector.id,
    name: connector.name,
    version: connector.version,
    ...(connector.description !== undefined && { description: connector.description }),
    triggers: connector.triggers.map((trigger) => ({
      type: trigger.type,
      label: trigger.label,
      ...(trigger.description !== undefined && { description: trigger.description }),
      setup: connectionSetup(connector, trigger.type)
    })),
    actions: connector.actions.map((action) => ({
      type: action.type,
      label: action.label,
      ...(action.description !== undefined && { description: action.description }),
      inputs: (action.inputs ?? []).map((input) => ({
        key: input.key,
        label: input.label,
        type: input.type ?? 'string',
        required: input.required === true
      }))
    }))
  }
}
