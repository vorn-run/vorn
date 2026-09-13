import { envNameFor } from './define'
import { PROTOCOL_VERSION } from './protocol'
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

export interface ConnectionSetup {
  connectorId: string
  triggerType: string
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

/** What a Vorn connection needs for one trigger: which trigger, and the environment the connector reads. */
export function connectionSetup(connector: Connector, triggerType: string): ConnectionSetup {
  const trigger = connector.triggers.find((entry) => entry.type === triggerType)
  if (!trigger) {
    throw new Error(`Connector ${connector.id} has no trigger "${triggerType}"`)
  }
  return {
    connectorId: connector.id,
    triggerType,
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
  panes?: Array<ManifestContribution & { icon?: ConnectorIcon; web?: string; command?: string[] }>
  footers?: Array<ManifestContribution & { every: number }>
  linkHandlers?: Array<ManifestContribution & { pattern: string; example: string }>
}

export interface ConnectorManifest {
  /** The connector protocol the pack speaks; Vorn refuses a pack without one. */
  protocol: number
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
    /** Whether repeating a call with the same arguments has no further effect; absent when unsaid. */
    idempotent?: boolean
    inputs: Array<{
      key: string
      label: string
      type: string
      required: boolean
      description?: string
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
        ...(pane.icon !== undefined && { icon: pane.icon }),
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
        pattern: handler.pattern,
        example: handler.example
      }))
    })
  }
}

/** Full machine-readable description of a connector, served over stdio, written into a pack and printed by the CLI. */
export function connectorManifest(connector: Connector): ConnectorManifest {
  const contributes = manifestContributions(connector)
  return {
    protocol: PROTOCOL_VERSION,
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
      ...(action.idempotent !== undefined && { idempotent: action.idempotent }),
      inputs: (action.inputs ?? []).map((input) => ({
        key: input.key,
        label: input.label,
        type: input.type ?? 'string',
        required: input.required === true,
        ...(input.description !== undefined && { description: input.description }),
        ...(input.options !== undefined && { options: input.options }),
        ...(input.loadOptions !== undefined && { loadOptions: input.loadOptions }),
        ...(input.builderHint !== undefined && { builderHint: input.builderHint })
      })),
      ...(action.outputs !== undefined && { outputs: action.outputs }),
      ...(action.sample !== undefined && { sample: action.sample })
    }))
  }
}
