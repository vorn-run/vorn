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
  SdkConnectorIcon,
  SdkConnectorManifest,
  SdkEnvVar,
  SdkProbeRequest,
  SdkProbeResult,
  SdkTrigger
} from '@vornrun/shared/types'
import { getSafeEnv } from '../process-utils'
import log from '../logger'

/** Tool an SDK connector serves to describe itself. */
export const MANIFEST_TOOL = 'vorn_connector_manifest'

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

/**
 * Validate the manifest into the shape the UI relies on.
 *
 * The payload crosses a process boundary from third-party code, so every field
 * the form reads is checked here rather than trusted downstream — a connector
 * that omits `triggers` should be a clear message, not a render crash.
 */
function toManifest(payload: Record<string, unknown>): SdkConnectorManifest {
  const id = str(payload.id).trim()
  const name = str(payload.name).trim()
  if (!id || !name) throw new Error('Connector manifest is missing an id or a name')

  const rawTriggers = Array.isArray(payload.triggers) ? payload.triggers : []
  const triggers: SdkTrigger[] = []
  const env = new Map<string, SdkEnvVar>()

  for (const raw of rawTriggers) {
    if (!isRecord(raw)) continue
    const type = str(raw.type).trim()
    if (!type) continue
    const setup = isRecord(raw.setup) ? raw.setup : {}
    const filters = isRecord(setup.filters) ? setup.filters : {}

    triggers.push({
      type,
      label: str(raw.label, type),
      ...(typeof raw.description === 'string' && { description: raw.description }),
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
      ...(typeof action.description === 'string' && { description: action.description })
    }))
    .filter((action) => action.type !== '')

  if (triggers.length === 0 && actions.length === 0) {
    throw new Error(`Connector ${name} reports no triggers and no actions`)
  }

  const icon = toIcon(payload.icon)

  log.info(`[sdk-probe] ${id}@${str(payload.version, '0.0.0')}: ${triggers.length} trigger(s)`)

  return {
    id,
    name,
    version: str(payload.version, '0.0.0'),
    ...(typeof payload.description === 'string' && { description: payload.description }),
    ...(icon && { icon }),
    triggers,
    actions,
    env: [...env.values()]
  }
}
