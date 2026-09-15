import fs from 'node:fs'
import type { FastifyInstance } from 'fastify'
import { GATE_VIEW_CSP } from './workflows/gate-views'

/** Serve a gate's review page to whoever holds this round's token, under a policy that lets it reach nothing. */
export function registerGateViewRoute(
  app: FastifyInstance,
  pageFor: (runId: string, nodeId: string, token: string) => string | null
): void {
  app.get('/gate-view/:runId/:nodeId', async (req, reply) => {
    const { runId, nodeId } = req.params as { runId: string; nodeId: string }
    const { t } = req.query as { t?: unknown }
    const file = typeof t === 'string' && t ? pageFor(runId, nodeId, t) : null
    if (!file || !fs.existsSync(file)) {
      return reply.code(404).send({ error: 'No review page here' })
    }
    reply
      .header('Content-Type', 'text/html; charset=utf-8')
      .header('Content-Security-Policy', GATE_VIEW_CSP)
      .header('X-Content-Type-Options', 'nosniff')
      .header('Referrer-Policy', 'no-referrer')
      .header('Cache-Control', 'no-store')
    return reply.send(fs.createReadStream(file))
  })
}
