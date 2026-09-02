/**
 * One-shot probe of a connector package built with `@vornrun/connector-sdk`.
 *
 * Such a connector is an ordinary MCP stdio server that also serves a
 * `vorn_connector_manifest` tool describing itself: its name, its triggers,
 * the environment variables it needs, and the exact filter values a Vorn
 * connection must carry to poll it correctly. Reading that manifest before the
 * connection exists is what lets Vorn fill the connection form in rather than
 * asking a person to transcribe a dozen field names from a README.
 *
 * Deliberately separate from `mcp-clients.ts`: that cache is keyed by
 * connection id and keeps children alive for the life of the process, which is
 * right for polling and wrong for a probe of something the user may not
 * install. This spawns, asks, and exits.
 */
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import type {
  ConnectorAuthRung,
  SdkActionInput,
  SdkConnectorAuth,
  SdkConnectorIcon,
  SdkConnectorManifest,
  SdkEnvVar,
  SdkProbeRequest,
  SdkProbeResult,
  SdkTrigger,
  TaskStatus
} from '@vornrun/shared/types'

// Re-exported because register-methods imports it from here rather than from
// shared, and a type-only import is not itself an export.
export type { SdkProbeRequest }
import { getSafeEnv } from '../process-utils'
import log from '../logger'

import { MANIFEST_TOOL } from './sdk-tools'

export { MANIFEST_TOOL }

/**
 * Give up rather than leave a child running. `npx -y <pkg>` downloads the
 * package on first use, so the budget has to cover a cold install, but an
 * unresponsive server must not wedge the settings UI.
 */
const PROBE_TIMEOUT_MS = 90_000

export async function probeSdkConnector(
  request: SdkProbeRequest,
  options: { timeoutMs?: number } = {}
): Promise<SdkProbeResult> {
  const command = request.command?.trim()
  if (!command) return { ok: false, error: 'A command is required' }

  const transport = new StdioClientTransport({
    command,
    args: request.args ?? [],
    // Same sanitized base as every other child process, so ambient tokens do
    // not leak into a package the user is merely inspecting.
    env: { ...getSafeEnv(), ...(request.env ?? {}) }
  })
  const client = new Client({ name: 'vorn', version: '0.1.0' }, { capabilities: {} })

  try {
    return await withTimeout(
      probe(client, transport),
      options.timeoutMs ?? PROBE_TIMEOUT_MS,
      `Timed out after ${Math.round((options.timeoutMs ?? PROBE_TIMEOUT_MS) / 1000)}s waiting for ${command}`
    )
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  } finally {
    // Always reached, including on timeout: the probe promise keeps running
    // after `withTimeout` rejects, and its child must not outlive this call.
    try {
      await client.close()
    } catch {
      /* the transport may already be gone */
    }
    try {
      await transport.close()
    } catch {
      /* ignore */
    }
  }
}

async function probe(client: Client, transport: StdioClientTransport): Promise<SdkProbeResult> {
  await client.connect(transport)

  const tools = await client.listTools()
  if (!(tools.tools ?? []).some((tool) => tool.name === MANIFEST_TOOL)) {
    return {
      ok: false,
      error:
        `This MCP server does not describe itself (no ${MANIFEST_TOOL} tool), so its ` +
        `connection settings cannot be filled in automatically. Configure it as a plain ` +
        `MCP connection instead.`
    }
  }

  const result = await client.callTool({ name: MANIFEST_TOOL, arguments: {} })
  if (result.isError) {
    return { ok: false, error: textContent(result) ?? `${MANIFEST_TOOL} failed` }
  }

  const payload = manifestPayload(result)
  if (!payload) return { ok: false, error: `${MANIFEST_TOOL} returned no manifest` }

  try {
    return { ok: true, manifest: toManifest(payload) }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

function withTimeout<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: NodeJS.Timeout
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), ms)
    })
  ])
}

function textContent(result: unknown): string | undefined {
  const content = (result as { content?: Array<{ type?: string; text?: string }> })?.content
  if (!Array.isArray(content)) return undefined
  for (const block of content) {
    if (block?.type === 'text' && typeof block.text === 'string') return block.text
  }
  return undefined
}

/**
 * Prefer `structuredContent`, falling back to parsing the text block. The SDK
 * always sends both, but a hand-written server may only send text.
 */
function manifestPayload(result: unknown): Record<string, unknown> | undefined {
  const structured = (result as { structuredContent?: unknown }).structuredContent
  if (isRecord(structured)) return structured
  const text = textContent(result)
  if (!text) return undefined
  try {
    const parsed: unknown = JSON.parse(text)
    return isRecord(parsed) ? parsed : undefined
  } catch {
    return undefined
  }
}

/**
 * Characters that appear in SVG path data. The SDK rejects anything else at
 * definition time, but this payload comes from a package the user just named,
 * so it is checked again here rather than trusted to have used the SDK.
 */
const PATH_DATA_PATTERN = /^[MmZzLlHhVvCcSsQqTtAa0-9\s,.\-+eE]+$/
const VIEW_BOX_PATTERN = /^-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+$/
/** Enough glyph to be recognizable; past this it is not an icon. */
const MAX_ICON_PATHS = 24
const MAX_PATH_LENGTH = 8_000

/**
 * A malformed icon costs the connector its glyph, nothing more — it must not
 * fail an otherwise usable install, so this returns undefined rather than
 * throwing.
 */
function toIcon(value: unknown): SdkConnectorIcon | undefined {
  if (!isRecord(value)) return undefined
  const paths = Array.isArray(value.paths) ? value.paths : []
  if (paths.length === 0 || paths.length > MAX_ICON_PATHS) return undefined
  const safe = paths.filter(
    (path): path is string =>
      typeof path === 'string' && path.length <= MAX_PATH_LENGTH && PATH_DATA_PATTERN.test(path)
  )
  // Dropping only the bad paths would draw a mangled glyph, so a single
  // rejected path discards the whole icon.
  if (safe.length !== paths.length) return undefined
  const viewBox = str(value.viewBox).trim()
  return { viewBox: VIEW_BOX_PATTERN.test(viewBox) ? viewBox : '0 0 24 24', paths: safe }
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value)

const str = (value: unknown, fallback = ''): string =>
  typeof value === 'string' ? value : fallback

const AUTH_RUNGS: ConnectorAuthRung[] = ['none', 'cli', 'key', 'oauth']

/**
 * A bare executable name, which is all a probe command is allowed to be.
 *
 * The host resolves this on PATH and runs it without a shell, so a path or a
 * shell metacharacter is never something this build would honour — it is
 * either a mistake or an attempt to have the app run something else. Either
 * way the answer is to refuse the declaration, not to sanitise it.
 */
const EXECUTABLE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/

const strings = (raw: unknown): string[] =>
  Array.isArray(raw) ? raw.filter((entry): entry is string => typeof entry === 'string') : []

/** The declared probe arguments, or nothing when any of them is not a string. */
function toProbeArgs(raw: unknown): string[] | undefined {
  if (raw === undefined) return []
  if (!Array.isArray(raw)) return undefined
  return raw.every((entry) => typeof entry === 'string') ? (raw as string[]) : undefined
}

/**
 * Read the arguments an action declares.
 *
 * The config panel maps over these to draw fields, so an input without a key
 * is dropped rather than drawn as a nameless box.
 */
function toActionInputs(value: unknown): SdkActionInput[] {
  return (Array.isArray(value) ? value : [])
    .filter(isRecord)
    .filter((input) => str(input.key) !== '')
    .map((input) => {
      const options = toActionOptions(input.options)
      return {
        key: str(input.key),
        label: str(input.label, str(input.key)),
        type: str(input.type, 'string'),
        required: input.required === true,
        // A select is undrawable without its choices, or without the name of
        // the set that supplies them.
        ...(options.length > 0 && { options }),
        ...(str(input.loadOptions) !== '' && { loadOptions: str(input.loadOptions) })
      }
    })
}

/** The choices a `select` offers; one that selects nothing is not a choice. */
function toActionOptions(value: unknown): Array<{ value: string; label?: string }> {
  return (Array.isArray(value) ? value : [])
    .filter(isRecord)
    .filter((option) => str(option.value) !== '')
    .map((option) => ({
      value: str(option.value),
      ...(typeof option.label === 'string' && { label: option.label })
    }))
}

/**
 * Read a declared auth block, or say nothing about how it signs in.
 *
 * An unknown rung is dropped rather than shown: the whole point of the field
 * is to tell someone what setting this up will ask of them, and a rung this
 * build cannot name answers that question wrongly.
 */
function toAuth(value: unknown): SdkConnectorAuth | undefined {
  if (!isRecord(value)) return undefined
  const rung = value.rung
  if (typeof rung !== 'string' || !AUTH_RUNGS.includes(rung as ConnectorAuthRung)) return undefined

  const probe = isRecord(value.probe) ? value.probe : undefined
  const command = str(probe?.command).trim()
  const args = toProbeArgs(probe?.args)
  const usable = EXECUTABLE_NAME.test(command) && args !== undefined

  // A rung is a promise about what setting this up will ask of you, and `cli`
  // promises there is a command to ask who you are. One that cannot be run is
  // the promise unbacked — better to say nothing than to offer a Sign in that
  // could not work.
  if (rung === 'cli' && !usable) return undefined

  const borrow = isRecord(value.borrow) ? value.borrow : undefined
  const env = strings(borrow?.env)
  const tokenArgs = strings(borrow?.tokenArgs)
  const keys = strings(value.keys)

  return {
    rung: rung as ConnectorAuthRung,
    ...(usable && args !== undefined && { probe: { command, ...(args.length > 0 && { args }) } }),
    ...((env.length > 0 || tokenArgs.length > 0) && {
      borrow: { ...(env.length > 0 && { env }), ...(tokenArgs.length > 0 && { tokenArgs }) }
    }),
    ...(keys.length > 0 && { keys })
  }
}

/**
 * Validate the manifest into the shape the UI relies on.
 *
 * The payload crosses a process boundary from third-party code, so every field
 * the form reads is checked here rather than trusted downstream — a connector
 * that omits `triggers` should be a clear message, not a render crash.
 */
export function toManifest(payload: Record<string, unknown>): SdkConnectorManifest {
  const id = str(payload.id).trim()
  const name = str(payload.name).trim()
  if (!id || !name) throw new Error('Connector manifest is missing an id or a name')

  /** Local statuses a connector is allowed to suggest. */
  const LOCAL_STATUSES = ['todo', 'in_progress', 'in_review', 'done', 'cancelled'] as const

  /**
   * Read a connector's suggested status mapping.
   *
   * This arrives from a third-party package, so an unrecognised local status is
   * dropped rather than written into a connection where it would fail a
   * constraint later, far from the connector that supplied it.
   */
  function readStatusMapping(
    raw: unknown
  ): Array<{ upstream: string; suggestedLocal: TaskStatus }> | undefined {
    if (!Array.isArray(raw)) return undefined
    const mapped = raw.filter(isRecord).flatMap((entry) => {
      const upstream = str(entry.upstream).trim()
      const local = str(entry.suggestedLocal).trim()
      if (!upstream) return []
      if (!(LOCAL_STATUSES as readonly string[]).includes(local)) return []
      return [{ upstream, suggestedLocal: local as TaskStatus }]
    })
    return mapped.length > 0 ? mapped : undefined
  }

  /** Read a connector's suggested polling workflow, if it declared a usable one. */
  function readDefaultWorkflow(
    raw: unknown
  ): { name: string; defaultCronFromMinutes: number } | undefined {
    if (!isRecord(raw)) return undefined
    const name = str(raw.name).trim()
    const minutes = Number(raw.defaultCronFromMinutes)
    // A zero or fractional interval would produce a cron that never fires or
    // fires constantly; neither is worth guessing a correction for.
    if (!name || !Number.isInteger(minutes) || minutes < 1 || minutes > 1440) return undefined
    return { name, defaultCronFromMinutes: minutes }
  }

  const rawTriggers = Array.isArray(payload.triggers) ? payload.triggers : []
  const triggers: SdkTrigger[] = []
  const env = new Map<string, SdkEnvVar>()

  for (const raw of rawTriggers) {
    if (!isRecord(raw)) continue
    const type = str(raw.type).trim()
    if (!type) continue
    const setup = isRecord(raw.setup) ? raw.setup : {}
    const filters = isRecord(setup.filters) ? setup.filters : {}

    const statusMapping = readStatusMapping(raw.statusMapping)
    const defaultWorkflow = readDefaultWorkflow(raw.defaultWorkflow)

    triggers.push({
      type,
      label: str(raw.label, type),
      ...(typeof raw.description === 'string' && { description: raw.description }),
      ...(statusMapping && { statusMapping }),
      ...(defaultWorkflow && { defaultWorkflow }),
      filters: {
        pollTool: str(filters.pollTool, `poll_${type}`),
        itemsPath: str(filters.itemsPath, 'items'),
        idField: str(filters.idField, 'externalId'),
        timestampField: str(filters.timestampField, 'updatedAt'),
        titleField: str(filters.titleField, 'title'),
        urlField: str(filters.urlField, 'url'),
        cursorArg: str(filters.cursorArg, 'cursor'),
        cursorPath: str(filters.cursorPath, 'nextCursor')
      }
    })

    // Every trigger reports the same connector-wide config, so the union is
    // collected rather than the first trigger's copy being assumed complete.
    for (const entry of Array.isArray(setup.env) ? setup.env : []) {
      if (!isRecord(entry)) continue
      const varName = str(entry.name).trim()
      if (!varName || env.has(varName)) continue
      env.set(varName, {
        name: varName,
        required: entry.required === true,
        secret: entry.secret === true,
        ...(typeof entry.description === 'string' && { description: entry.description })
      })
    }
  }

  const actions = (Array.isArray(payload.actions) ? payload.actions : [])
    .filter(isRecord)
    .map((action) => ({
      type: str(action.type),
      label: str(action.label, str(action.type)),
      ...(typeof action.description === 'string' && { description: action.description }),
      // Carried so a step can name its arguments without the connector running.
      ...(action.inputs !== undefined && { inputs: toActionInputs(action.inputs) })
    }))
    .filter((action) => action.type !== '')

  if (triggers.length === 0 && actions.length === 0) {
    throw new Error(`Connector ${name} reports no triggers and no actions`)
  }

  const icon = toIcon(payload.icon)
  const auth = toAuth(payload.auth)

  log.info(`[sdk-probe] ${id}@${str(payload.version, '0.0.0')}: ${triggers.length} trigger(s)`)

  return {
    id,
    name,
    version: str(payload.version, '0.0.0'),
    ...(typeof payload.description === 'string' && { description: payload.description }),
    ...(icon && { icon }),
    ...(auth && { auth }),
    triggers,
    actions,
    env: [...env.values()]
  }
}
