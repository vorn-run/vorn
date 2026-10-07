import type { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify'
import { isLoopbackAddress } from './ws-handler'
import { VORN_PEER_HEADER } from './vornd-relay'
import log from './logger'

/**
 * The work model's pages, answered by vornd: an artifact's version, a gate's
 * review page and a workflow's webhook.
 *
 * Their addresses name this server's port, as every one printed or saved so
 * far does, so they are relayed to vornd here rather than moved. A webhook is
 * taken from this machine only; anything else is refused before it is relayed.
 */

const RELAY_TIMEOUT_MS = 30_000

/** Headers a page or a webhook depends on. */
const FORWARDED = ['content-type', 'idempotency-key', 'x-vorn-delivery', 'user-agent']

function bodyOf(req: FastifyRequest): string | undefined {
  if (req.method === 'GET' || req.body === undefined || req.body === null) return undefined
  return typeof req.body === 'string' ? req.body : JSON.stringify(req.body)
}

/** The path relayed: rebuilt from what the router matched, never the raw URL, so vornd sees what matched here. */
function relayedPath(req: FastifyRequest, prefix: string, names: string[]): string {
  const params = req.params as Record<string, string>
  const query = req.url.includes('?') ? req.url.slice(req.url.indexOf('?')) : ''
  return `/${prefix}/${names.map((n) => encodeURIComponent(params[n] ?? '')).join('/')}${query}`
}

async function relay(
  req: FastifyRequest,
  reply: FastifyReply,
  vorndPort: () => number | null,
  path: string
): Promise<unknown> {
  const port = vorndPort()
  if (port === null) return reply.code(503).send({ error: 'vornd is not running' })
  const headers: Record<string, string> = { [VORN_PEER_HEADER]: req.ip }
  for (const [name, value] of Object.entries(req.headers)) {
    if (value === undefined || name.toLowerCase() === VORN_PEER_HEADER) continue
    const lower = name.toLowerCase()
    if (FORWARDED.includes(lower) || lower.startsWith('x-')) {
      headers[lower] = Array.isArray(value) ? value.join(', ') : value
    }
  }
  try {
    const res = await fetch(`http://127.0.0.1:${port}${path}`, {
      method: req.method,
      headers,
      body: bodyOf(req),
      signal: AbortSignal.timeout(RELAY_TIMEOUT_MS)
    })
    reply.code(res.status)
    res.headers.forEach((value, name) => {
      if (name !== 'content-length' && name !== 'transfer-encoding' && name !== 'connection') {
        reply.header(name, value)
      }
    })
    return reply.send(Buffer.from(await res.arrayBuffer()))
  } catch (err) {
    log.warn({ err, url: req.url }, '[vornd] could not relay a work page to vornd')
    return reply.code(502).send({ error: 'vornd did not answer' })
  }
}

export function registerWorkRoutes(app: FastifyInstance, vorndPort: () => number | null): void {
  app.get('/artifact/:artifactId/:version', (req, reply) =>
    relay(req, reply, vorndPort, relayedPath(req, 'artifact', ['artifactId', 'version']))
  )
  app.get('/gate-view/:runId/:nodeId', (req, reply) =>
    relay(req, reply, vorndPort, relayedPath(req, 'gate-view', ['runId', 'nodeId']))
  )
  app.route({
    method: ['GET', 'POST'],
    url: '/wf-hooks/:workflowId/:token',
    handler: async (req, reply) => {
      if (!isLoopbackAddress(req.socket.remoteAddress)) {
        return reply.code(403).send({ error: 'Local machine only' })
      }
      return relay(req, reply, vorndPort, relayedPath(req, 'wf-hooks', ['workflowId', 'token']))
    }
  })
}
