import type { FastifyInstance } from 'fastify'
import { GATE_VIEW_CSP } from './workflows/gate-views'

/** Serve one version of an artifact to whoever holds its token, under the same policy as a gate's review page. */
export function registerArtifactRoute(
  app: FastifyInstance,
  pageFor: (artifactId: string, version: number, token: string) => string | null
): void {
  app.get('/artifact/:artifactId/:version', async (req, reply) => {
    const { artifactId, version } = req.params as { artifactId: string; version: string }
    const { t } = req.query as { t?: unknown }
    const n = Number(version)
    const html =
      typeof t === 'string' && t && Number.isInteger(n) && n > 0 ? pageFor(artifactId, n, t) : null
    if (html === null) return reply.code(404).send({ error: 'No artifact here' })
    reply
      .header('Content-Type', 'text/html; charset=utf-8')
      .header('Content-Security-Policy', GATE_VIEW_CSP)
      .header('X-Content-Type-Options', 'nosniff')
      .header('Referrer-Policy', 'no-referrer')
      .header('Cache-Control', 'no-store')
    return reply.send(html)
  })
}
