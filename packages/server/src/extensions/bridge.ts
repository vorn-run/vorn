import { createReadStream, existsSync, realpathSync, statSync } from 'node:fs'
import { dirname, extname, resolve, sep } from 'node:path'
import type { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify'
import type {
  ExtensionPermission,
  InstalledConnectorPack,
  TerminalSession
} from '@vornrun/shared/types'
import { WEB_FILE_TYPES, installedPack } from '../connectors/packs'
import { getGitDiffText, getGitStatusPorcelain } from '../git-utils'
import { ptyManager } from '../pty-manager'
import { grantFor } from './panes'
import { hostByToken } from './hosts'
import { requestSelection } from './selection'
import { usageFor } from './usage'
import { isLoopbackAddress } from '../ws-handler'
import { bearerFrom } from '../ws-auth'
import log from '../logger'

/**
 * The half of the bridge the host serves.
 *
 * The SDK's client posts one method per call with the session it is asking
 * about; this decides whether the caller is who it says, whether that session
 * is one of its project's, and whether the manifest asked for what the method
 * costs. A call failing any of those is refused rather than answered thinly, so
 * an extension learns the rule once instead of finding an empty string.
 */

/** What each method costs, mirroring the table the SDK's check gates against. */
const METHOD_PERMISSIONS: Record<string, ExtensionPermission> = {
  diff: 'git.read',
  status: 'git.read',
  output: 'terminal.read',
  selection: 'terminal.selection',
  send: 'terminal.send',
  rename: 'card.rename',
  usage: 'agent.usage'
}

/** How each carried file type is served; the set of them is the pack gate's, so the two cannot drift. */
const MEDIA_TYPES: Record<string, string> = {
  html: 'text/html; charset=utf-8',
  css: 'text/css; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
  mjs: 'text/javascript; charset=utf-8',
  json: 'application/json; charset=utf-8',
  svg: 'image/svg+xml',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  webp: 'image/webp',
  gif: 'image/gif',
  woff: 'font/woff',
  woff2: 'font/woff2',
  txt: 'text/plain; charset=utf-8',
  md: 'text/plain; charset=utf-8'
}

const PAGE_TYPES: Record<string, string> = Object.fromEntries(
  WEB_FILE_TYPES.map((type) => [`.${type}`, MEDIA_TYPES[type] ?? 'application/octet-stream'])
)

/** A refusal the caller cannot argue with, rather than a failure it should retry. */
class RefusedError extends Error {}

const DEFAULT_OUTPUT_LINES = 200
const MAX_OUTPUT_LINES = 5000
/** A line has no length of its own, so the reply is held to bytes as well as lines. */
const MAX_OUTPUT_BYTES = 256 * 1024

/**
 * Whether a browser has told us this came from somewhere else.
 *
 * The extension's own process sends neither header, so it is unaffected; a page
 * sends both, and the only page entitled to speak here is the one this origin
 * served. A form post from a hostile page cannot forge either.
 */
function fromElsewhere(req: FastifyRequest): boolean {
  const site = req.headers['sec-fetch-site']
  if (typeof site === 'string' && site !== '' && site !== 'same-origin' && site !== 'none') {
    return true
  }
  const origin = req.headers.origin
  return typeof origin === 'string' && origin !== '' && origin !== `http://${req.headers.host}`
}

/** Who is calling: the extension's own process by its token, or a pane's page by its nonce. */
interface Caller {
  extensionId: string
  projectPath: string
}

function sessionOf(sessionId: unknown): TerminalSession | undefined {
  if (typeof sessionId !== 'string' || sessionId === '') return undefined
  return ptyManager.getLiveSessions().find((session) => session.id === sessionId)
}

function refuse(reply: FastifyReply, code: number, reason: string): FastifyReply {
  // Plain text: the SDK reads a refusal's body verbatim into the error it raises.
  return reply.code(code).type('text/plain; charset=utf-8').send(reason)
}

/** The extension's own reads, once the caller and the session are settled. */
async function answer(
  method: string,
  session: TerminalSession,
  body: Record<string, unknown>,
  deps: ExtensionRouteDeps
): Promise<{ result?: unknown }> {
  const worktreePath = session.worktreePath ?? session.projectPath
  switch (method) {
    case 'diff':
      return { result: getGitDiffText(worktreePath) }
    case 'status':
      return { result: getGitStatusPorcelain(worktreePath) }
    case 'output': {
      const asked = typeof body.lines === 'number' ? body.lines : DEFAULT_OUTPUT_LINES
      const lines = Math.min(Math.max(Math.trunc(asked), 1), MAX_OUTPUT_LINES)
      const text = ptyManager.getOutput(session.id, lines).join('\n')
      // Trimmed from the front: a reader asking for output wants how it ended.
      return { result: text.length > MAX_OUTPUT_BYTES ? text.slice(-MAX_OUTPUT_BYTES) : text }
    }
    case 'selection':
      return { result: await requestSelection(session.id) }
    case 'send': {
      const text = body.text
      if (typeof text !== 'string') throw new Error('send takes the text to type')
      ptyManager.writeToPty(session.id, text)
      return {}
    }
    case 'rename': {
      const name = body.name
      if (typeof name !== 'string' || name.trim() === '') {
        throw new Error('rename takes the name to show')
      }
      if (session.renamedByPerson) {
        throw new RefusedError('that card was named by the person using it')
      }
      ptyManager.renameSession(session.id, name.trim(), false)
      // Named the way the rename method does, so the name survives a restart and
      // every window sees it rather than only the one that asked.
      deps.sessionRenamed(session.id, name.trim())
      return {}
    }
    case 'usage':
      return { result: usageFor(session) }
    default:
      throw new Error(`The host serves no method "${method}"`)
  }
}

/** A page is served only from inside the directory its own pane named. */
function pageFile(pack: InstalledConnectorPack, paneId: string, rest: string): string | undefined {
  const pane = pack.contributes?.panes?.find((one) => one.id === paneId)
  if (!pane?.web) return undefined
  const root = resolve(pack.path, dirname(pane.web))
  const wanted = rest === '' || rest.endsWith('/') ? `${rest}index.html` : rest
  const candidate = resolve(root, wanted)
  if (!candidate.startsWith(root + sep)) return undefined
  if (!existsSync(candidate) || !statSync(candidate).isFile()) return undefined
  if (!PAGE_TYPES[extname(candidate).toLowerCase()]) return undefined
  // Compared as real paths on both sides: a link out of the directory survives
  // `resolve`, and the pack's own directory may itself sit under one.
  try {
    return realpathSync(candidate).startsWith(realpathSync(root) + sep) ? candidate : undefined
  } catch {
    return undefined
  }
}

export interface ExtensionRouteDeps {
  /** Origins the app is served from, which are the only ones that may frame a pane. */
  frameAncestors: () => string[]
  /** Saves a rename and tells every window, exactly as the rename method does. */
  sessionRenamed: (sessionId: string, displayName: string) => void
}

/** One handler for both ways in; only how the caller proved itself differs. */
async function serve(
  caller: Caller | undefined,
  method: string,
  body: unknown,
  reply: FastifyReply,
  deps: ExtensionRouteDeps,
  boundSessionId?: string
): Promise<FastifyReply> {
  if (!caller) return refuse(reply, 401, 'This bridge does not know that caller')
  const pack = installedPack(caller.extensionId)
  if (!pack || pack.kind !== 'extension') {
    return refuse(reply, 404, `No extension "${caller.extensionId}" is installed`)
  }
  const permission = METHOD_PERMISSIONS[method]
  if (!permission) return refuse(reply, 404, `The host serves no method "${method}"`)
  if (!(pack.permissions ?? []).includes(permission)) {
    // The same sentence the check's stub gives, so an extension meets one rule rather than two.
    return refuse(reply, 403, `this extension does not ask for ${permission}`)
  }

  const params = (body ?? {}) as Record<string, unknown>
  const sessionId = boundSessionId ?? params.sessionId
  const session = sessionOf(sessionId)
  if (!session) return refuse(reply, 404, 'That session is not running')
  // A project's extension answers about that project's sessions and no others.
  if (session.projectPath !== caller.projectPath) {
    return refuse(reply, 403, 'That session belongs to another project')
  }

  try {
    const answered = await answer(method, session, params, deps)
    if (!('result' in answered)) return reply.code(204).send()
    return reply.code(200).send({ result: answered.result })
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err)
    if (err instanceof RefusedError) return refuse(reply, 403, message)
    log.warn(`[extensions] ${caller.extensionId} ${method} failed: ${message}`)
    return refuse(reply, 500, message)
  }
}

/** The extension's own process, which holds the token it was started with. */
export function registerExtensionBridge(app: FastifyInstance, deps: ExtensionRouteDeps): void {
  app.post('/extensions/:id/bridge/:method', async (req, reply) => {
    if (!isLoopbackAddress(req.ip)) return refuse(reply, 403, 'Local machine only')
    if (fromElsewhere(req)) return refuse(reply, 403, 'That request came from another site')
    const { id, method } = req.params as { id: string; method: string }
    const token = bearerFrom(req.headers.authorization)
    const host = token ? hostByToken(id, token) : undefined
    const caller = host
      ? { extensionId: host.extensionId, projectPath: host.projectPath }
      : undefined
    return serve(caller, method, req.body, reply, deps)
  })
  log.info('[extensions] bridge route registered')
}

/**
 * A pane's pages, and the bridge those pages call.
 *
 * Registered on an origin of their own rather than beside the app: a page here
 * shares no storage and no socket with the web client, so a page that is fed a
 * hostile script still holds nothing but the nonce in its own URL.
 */
export function registerExtensionPages(app: FastifyInstance, deps: ExtensionRouteDeps): void {
  app.post('/extensions/:id/pane/:paneId/:nonce/bridge/:method', async (req, reply) => {
    if (!isLoopbackAddress(req.ip)) return refuse(reply, 403, 'Local machine only')
    if (fromElsewhere(req)) return refuse(reply, 403, 'That request came from another site')
    const { id, paneId, nonce, method } = req.params as {
      id: string
      paneId: string
      nonce: string
      method: string
    }
    const grant = grantFor(nonce)
    const caller =
      grant && grant.extensionId === id && grant.paneId === paneId
        ? { extensionId: grant.extensionId, projectPath: grant.projectPath }
        : undefined
    // The session comes from the grant, not the body: a page speaks for the one pane it was opened as.
    return serve(caller, method, req.body, reply, deps, grant?.sessionId)
  })

  app.get('/extensions/:id/pane/:paneId/:nonce/*', async (req, reply) => {
    if (!isLoopbackAddress(req.ip)) return refuse(reply, 403, 'Local machine only')
    const { id, paneId, nonce } = req.params as { id: string; paneId: string; nonce: string }
    const rest = ((req.params as Record<string, string>)['*'] ?? '').replace(/^\/+/, '')
    const grant = grantFor(nonce)
    if (!grant || grant.extensionId !== id || grant.paneId !== paneId) {
      return refuse(reply, 404, 'Not found')
    }
    const pack = installedPack(id)
    if (!pack || pack.kind !== 'extension') return refuse(reply, 404, 'Not found')
    const file = pageFile(pack, paneId, rest)
    if (!file) return refuse(reply, 404, 'Not found')

    const ancestors = deps.frameAncestors()
    reply.header('Content-Type', PAGE_TYPES[extname(file).toLowerCase()])
    reply.header('X-Content-Type-Options', 'nosniff')
    reply.header('Cache-Control', 'private, no-store')
    // A path carries the nonce, so nothing may take it to another site.
    reply.header('Referrer-Policy', 'no-referrer')
    reply.header('Cross-Origin-Opener-Policy', 'same-origin')
    // Its own files and its own bridge, framed by the app and by nothing else.
    reply.header(
      'Content-Security-Policy',
      `default-src 'self'; connect-src 'self'; frame-ancestors ${ancestors.join(' ') || "'none'"}`
    )
    return reply.send(createReadStream(file))
  })

  log.info('[extensions] pane page routes registered')
}

export { METHOD_PERMISSIONS }
