import { envNameFor } from './define'
import type {
  ActionInputOption,
  ActivationPredicate,
  Connector,
  ConnectorAuth,
  ConnectorIcon,
  ConnectorKind,
  DefaultWorkflow,
  ExtensionPermission,
  StatusSuggestion
} from './types'

/** MCP tool name a trigger is served under. */
export function pollToolName(triggerType: string): string {
  return `poll_${triggerType}`
}

/** MCP tool name a footer is recomputed under. */
export function footerToolName(footerId: string): string {
  return `vorn_footer_${footerId}`
}

/** MCP tool name a link handler is run under. */
export function handlerToolName(handlerId: string): string {
  return `vorn_handler_${handlerId}`
}

/** Tool that reports the connector's manifest and setup hints. */
export const MANIFEST_TOOL = 'vorn_connector_manifest'

/**
 * Tool that reports whether the connector can run right now. Present only when
 * the connector declares a `preflight`, so its absence means "nothing to
 * check" rather than "check passed".
 */
export const PREFLIGHT_TOOL = 'vorn_connector_preflight'

/**
 * Tool that lists the choices for one dynamic field. Present only when the
 * connector serves an options set, for the same reason preflight is.
 */
export const OPTIONS_TOOL = 'vorn_connector_options'

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
    cursorArg: 'cursor'
    cursorPath: 'nextCursor'
  }
  /** Environment variable names the connector reads. */
  env: Array<{
    name: string
    required: boolean
    secret: boolean
    description?: string
    /** For whoever is building a connector like this one, not for whoever runs it. */
    builderHint?: string
  }>
}

/**
 * Describe how to wire one trigger into a Vorn MCP connection.
 *
 * Every SDK connector normalizes to the same field names, so this mapping is
 * fixed; it is generated rather than documented so a rename in the SDK cannot
 * drift away from the setup instructions users copy. `cursorArg` hands the
 * connector back its own cursor each poll, which is what lets its dedupe
 * strategy — rather than Vorn's timestamp comparison — decide what is new.
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
      urlField: 'url',
      cursorArg: 'cursor',
      cursorPath: 'nextCursor'
    },
    env: connector.config.map((field) => ({
      name: envNameFor(field.key, field.env),
      required: field.required === true,
      secret: field.secret === true,
      ...(field.description !== undefined && { description: field.description }),
      ...(field.builderHint !== undefined && { builderHint: field.builderHint })
    }))
  }
}

/** A contribution as the manifest carries it: everything but the code that runs it. */
interface ManifestContribution {
  id: string
  title: string
  description?: string
  when?: ActivationPredicate
}

export interface ManifestContributions {
  panes?: Array<ManifestContribution & { web?: string; command?: string[] }>
  footers?: Array<ManifestContribution & { every: number }>
  linkHandlers?: Array<ManifestContribution & { pattern: string }>
}

export interface ConnectorManifest {
  id: string
  name: string
  version: string
  /** Absent on a manifest written before extensions, which reads as a connector. */
  kind?: ConnectorKind
  description?: string
  icon?: ConnectorIcon
  /** How the connector signs in, so the app can say so before installing it. */
  auth?: ConnectorAuth
  /** What an extension adds to a card. Present only on an extension. */
  contributes?: ManifestContributions
  /** What an extension may ask the host for. Present only on an extension. */
  permissions?: ExtensionPermission[]
  /** Where an extension shows at all. Present only on an extension. */
  activates?: ActivationPredicate
  triggers: Array<{
    type: string
    label: string
    description?: string
    /** Seeds a connection's status mapping; absent when the connector was silent. */
    statusMapping?: StatusSuggestion[]
    /** Seeds the polling workflow created with the connection. */
    defaultWorkflow?: DefaultWorkflow
    setup: ConnectionSetup
  }>
  actions: Array<{
    type: string
    label: string
    description?: string
    inputs: Array<{
      key: string
      label: string
      type: string
      required: boolean
      options?: ActionInputOption[]
      /** An options set the connector serves, resolved against a live connection. */
      loadOptions?: string
      /** For whoever is building a connector like this one, not for whoever runs it. */
      builderHint?: string
    }>
    /**
     * Fields the action is known to return. Absent when the connector declared
     * none, which is not the same as saying it returns nothing.
     */
    outputs?: Array<{ key: string; type?: string; description?: string }>
    /** Arguments a live check may call it with, when the author named some. */
    sample?: Record<string, string>
  }>
}

/** Strip the running code off a contribution, leaving what a manifest can carry. */
function manifestContributions(connector: Connector): ManifestContributions | undefined {
  const contributes = connector.contributes
  if (!contributes) return undefined
  const shared = (contribution: ManifestContribution): ManifestContribution => ({
    id: contribution.id,
    title: contribution.title,
    ...(contribution.description !== undefined && { description: contribution.description }),
    ...(contribution.when !== undefined && { when: contribution.when })
  })
  return {
    ...(contributes.panes !== undefined && {
      panes: contributes.panes.map((pane) => ({
        ...shared(pane),
        ...(pane.web !== undefined && { web: pane.web }),
        ...(pane.command !== undefined && { command: pane.command })
      }))
    }),
    ...(contributes.footers !== undefined && {
      footers: contributes.footers.map((footer) => ({ ...shared(footer), every: footer.every }))
    }),
    ...(contributes.linkHandlers !== undefined && {
      linkHandlers: contributes.linkHandlers.map((handler) => ({
        ...shared(handler),
        pattern: handler.pattern
      }))
    })
  }
}

/** Full machine-readable description of a connector, served over MCP and printed by the CLI. */
export function connectorManifest(connector: Connector): ConnectorManifest {
  const contributes = manifestContributions(connector)
  return {
    id: connector.id,
    name: connector.name,
    version: connector.version,
    kind: connector.kind,
    ...(connector.description !== undefined && { description: connector.description }),
    ...(connector.icon !== undefined && { icon: connector.icon }),
    ...(connector.auth !== undefined && { auth: connector.auth }),
    ...(contributes !== undefined && { contributes }),
    ...(connector.permissions !== undefined && { permissions: connector.permissions }),
    ...(connector.activates !== undefined && { activates: connector.activates }),
    triggers: connector.triggers.map((trigger) => ({
      type: trigger.type,
      label: trigger.label,
      ...(trigger.description !== undefined && { description: trigger.description }),
      // Carried through so the app can seed a connection's status mapping and
      // its polling workflow. Absent when the connector said nothing, which is
      // different from saying there is nothing.
      ...(trigger.statusMapping !== undefined && { statusMapping: trigger.statusMapping }),
      ...(trigger.defaultWorkflow !== undefined && { defaultWorkflow: trigger.defaultWorkflow }),
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
        required: input.required === true,
        ...(input.options !== undefined && { options: input.options }),
        ...(input.loadOptions !== undefined && { loadOptions: input.loadOptions }),
        ...(input.builderHint !== undefined && { builderHint: input.builderHint })
      })),
      ...(action.outputs !== undefined && { outputs: action.outputs }),
      ...(action.sample !== undefined && { sample: action.sample })
    }))
  }
}
