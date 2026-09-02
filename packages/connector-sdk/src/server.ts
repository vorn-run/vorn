import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { z, type ZodTypeAny } from 'zod'
import { resolveConfig } from './define'
import { runAction, runOptions, runPoll } from './runtime'
import {
  MANIFEST_TOOL,
  OPTIONS_TOOL,
  PREFLIGHT_TOOL,
  connectorManifest,
  pollToolName
} from './setup'
import type { ActionInputField, ActionOutputField, Connector, ConnectorConfig } from './types'

function json(value: Record<string, unknown>): {
  content: Array<{ type: 'text'; text: string }>
  structuredContent: Record<string, unknown>
} {
  return {
    // Vorn reads `structuredContent` to build step output and to find the
    // `items` array a poll returned; the text block keeps the result readable
    // in any generic MCP client.
    content: [{ type: 'text', text: JSON.stringify(value, null, 2) }],
    structuredContent: value
  }
}

function failure(error: unknown): {
  content: Array<{ type: 'text'; text: string }>
  isError: true
} {
  return {
    content: [{ type: 'text', text: error instanceof Error ? error.message : String(error) }],
    isError: true
  }
}

/**
 * What the tool schema says a field is, in the words a caller reads.
 *
 * The manifest and the served schema describe the same `ActionInputField`, so
 * they are generated from it together rather than drifting into two accounts
 * of the same argument.
 */
function describeInput(input: ActionInputField): string {
  const base = input.description ?? input.label
  if (input.loadOptions !== undefined) {
    return `${base}. Choices come from this connector's "${input.loadOptions}" list.`
  }
  if (input.type === 'json') return `${base}. Takes JSON.`
  return base
}

function inputShape(inputs: ActionInputField[]): Record<string, ZodTypeAny> {
  const shape: Record<string, ZodTypeAny> = {}
  for (const input of inputs) {
    const choices = (input.options ?? [])
      .map((option) => option.value)
      .filter((value) => typeof value === 'string' && value !== '')
    // Vorn renders action arguments from templates, so every value arrives as
    // a string; the declared type is applied by `runAction` instead. Fixed
    // choices are still stated, because a caller that knows them draws a
    // picker instead of a text box.
    const base =
      choices.length > 0
        ? z.enum(choices as [string, ...string[]]).describe(describeInput(input))
        : z.string().describe(describeInput(input))
    shape[input.key] = input.required ? base : base.optional()
  }
  return shape
}

function scalar(type: ActionOutputField['type']): ZodTypeAny {
  if (type === 'number') return z.number()
  if (type === 'boolean') return z.boolean()
  return z.string()
}

/**
 * Output schemas are always loose. An action returns whatever the upstream API
 * gave it, and a strict schema would make the MCP client reject the call for
 * the crime of returning an extra field.
 */
function outputSchema(outputs: ActionOutputField[]): ZodTypeAny {
  const shape: Record<string, ZodTypeAny> = {}
  for (const output of outputs) {
    shape[output.key] = scalar(output.type)
      .optional()
      .describe(output.description ?? output.key)
  }
  return z.looseObject(shape)
}

export interface ConnectorServerOptions {
  /** Resolved connector configuration. Defaults to reading `process.env`. */
  config?: ConnectorConfig
  now?: () => string
}

/**
 * Expose a connector as an MCP server.
 *
 * Each trigger becomes a `poll_<type>` tool returning the normalized page,
 * each action becomes a tool of the same name, and `vorn_connector_manifest`
 * reports everything needed to configure the connection. That is the entire
 * contract — Vorn's generic MCP connector consumes it with no host changes.
 */
export function createConnectorServer(
  connector: Connector,
  options: ConnectorServerOptions = {}
): McpServer {
  const server = new McpServer(
    { name: connector.id, version: connector.version },
    { capabilities: { tools: {} } }
  )

  // Resolved lazily so a missing environment variable surfaces as a tool error
  // the user can read, rather than killing the process during MCP handshake.
  let cached: ConnectorConfig | undefined = options.config
  const config = (): ConnectorConfig => (cached ??= resolveConfig(connector))

  server.registerTool(
    MANIFEST_TOOL,
    {
      description: `Describe the ${connector.name} connector and how to configure it`,
      inputSchema: {},
      outputSchema: z.looseObject({})
    },
    () => json(connectorManifest(connector) as unknown as Record<string, unknown>)
  )

  // Registered only when declared, so a caller can tell "this connector has
  // nothing to check" from "the check passed" by whether the tool exists.
  if (connector.preflight) {
    const preflight = connector.preflight.bind(connector)
    server.registerTool(
      PREFLIGHT_TOOL,
      {
        description: `Check whether ${connector.name} can run right now`,
        inputSchema: {},
        // Declared rather than left open like the manifest's: this shape is
        // fixed, so a caller can validate against it. Still loose, because a
        // connector adding a field of its own should not fail the call.
        outputSchema: z.looseObject({
          ok: z.boolean().describe('Whether the connector could run right now'),
          message: z.string().optional().describe('What to do about it, when it could not')
        })
      },
      async () => {
        // A throw is the connector saying "broken", not "not set up yet", and
        // it must not read to the user as a passing check. Reporting it as a
        // failed preflight with the message keeps the distinction the caller
        // can act on: ok:false is always something a person can fix.
        try {
          return json({ ...(await preflight()) })
        } catch (error) {
          return json({
            ok: false,
            message: error instanceof Error ? error.message : String(error)
          })
        }
      }
    )
  }

  // Registered only when the connector serves a set, so its absence means
  // "nothing here loads its choices" rather than "the list came back empty".
  const optionSets = Object.keys(connector.options ?? {})
  if (optionSets.length > 0) {
    server.registerTool(
      OPTIONS_TOOL,
      {
        description: `List what one of ${connector.name}'s fields can be set to`,
        inputSchema: {
          name: z.enum(optionSets as [string, ...string[]]).describe('Which options set to list')
        },
        outputSchema: z.looseObject({
          options: z
            .array(z.looseObject({ value: z.string(), label: z.string().optional() }))
            .describe('The choices, each a value to send and words to show')
        })
      },
      async (args) => {
        try {
          return json({
            options: await runOptions(connector, args.name, {
              config: config(),
              ...(options.now && { now: options.now })
            })
          })
        } catch (error) {
          return failure(error)
        }
      }
    )
  }

  for (const trigger of connector.triggers) {
    server.registerTool(
      pollToolName(trigger.type),
      {
        description: trigger.description ?? `Poll ${connector.name} for ${trigger.label}`,
        inputSchema: {
          since: z
            .string()
            .optional()
            .describe('Only return items changed after this ISO timestamp'),
          cursor: z.string().optional().describe('Opaque cursor from a previous page'),
          limit: z.string().optional().describe('Maximum number of items to return')
        },
        outputSchema: z.looseObject({
          items: z.array(z.looseObject({})).describe('Normalized items'),
          nextCursor: z.string().optional().describe('Cursor for the next page'),
          hasMore: z.boolean().describe('Whether another page is immediately available')
        })
      },
      async (args) => {
        try {
          const limit = args.limit === undefined ? undefined : Number(args.limit)
          if (limit !== undefined && !Number.isFinite(limit)) {
            throw new Error(`Invalid limit "${args.limit}"`)
          }
          return json(
            (await runPoll(connector, trigger.type, {
              config: config(),
              ...(args.since !== undefined && { since: args.since }),
              ...(args.cursor !== undefined && { cursor: args.cursor }),
              ...(limit !== undefined && { limit }),
              ...(options.now && { now: options.now })
            })) as unknown as Record<string, unknown>
          )
        } catch (error) {
          return failure(error)
        }
      }
    )
  }

  for (const action of connector.actions) {
    const base = action.description ?? `${action.label} in ${connector.name}`
    // An agent retrying a failed step has no other way to know whether it is
    // about to create a second issue.
    const retryHint =
      action.idempotent === undefined
        ? ''
        : action.idempotent
          ? ' Safe to retry: repeating this call with the same arguments has no additional effect.'
          : ' Not idempotent: repeating this call performs the operation again.'
    server.registerTool(
      action.type,
      {
        // Carries the authored label, so a picker can name the action rather than its tool.
        title: action.label,
        description: `${base}${retryHint}`,
        inputSchema: inputShape(action.inputs ?? []),
        outputSchema: outputSchema(action.outputs ?? [])
      },
      async (args) => {
        try {
          return json(
            await runAction(connector, action.type, args as Record<string, unknown>, {
              config: config(),
              ...(options.now && { now: options.now })
            })
          )
        } catch (error) {
          return failure(error)
        }
      }
    )
  }

  return server
}

/** Serve a connector on stdio. This is the one line a connector's bin needs. */
export async function serveConnector(
  connector: Connector,
  options: ConnectorServerOptions = {}
): Promise<void> {
  const server = createConnectorServer(connector, options)
  await server.connect(new StdioServerTransport())
}
