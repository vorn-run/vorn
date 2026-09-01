import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import type { TriggerConfig, WebhookTriggerConfig } from '@vornrun/shared/types'
import { dbGetWorkflow, dbEnqueueWebhookEvent } from './database'
import log from './logger'

export const WEBHOOK_CONNECTOR_ID = 'webhook'

/** Auth-bearing headers stay out of run records; everything else comes through. */
const HEADER_DENYLIST = new Set(['authorization', 'cookie', 'proxy-authorization', 'x-api-key'])

function isLoopback(ip: string | undefined): boolean {
  return ip === '127.0.0.1' || ip === '::1' || ip === '::ffff:127.0.0.1'
}

function webhookTrigger(triggerConfig: TriggerConfig | null): WebhookTriggerConfig | null {
  return triggerConfig?.triggerType === 'webhook' ? triggerConfig : null
}

/**
 * One localhost route starts workflows over HTTP. Events land in the durable
 * connector inbox, so a request received with no window open still runs when
 * a client next connects.
 */
export function registerWebhookRoute(app: FastifyInstance, onEnqueued: () => void): void {
  app.route({
    method: ['GET', 'POST'],
    url: '/wf-hooks/:workflowId/:token',
    handler: async (req, reply) => {
      if (!isLoopback(req.ip)) return reply.code(403).send({ error: 'Local machine only' })

      const { workflowId, token } = req.params as { workflowId: string; token: string }
      const workflow = dbGetWorkflow(workflowId)
      const triggerNode = workflow?.nodes.find((n) => n.type === 'trigger')
      const trigger = webhookTrigger((triggerNode?.config as TriggerConfig) ?? null)
      // One answer for every miss - wrong method included - so the route
      // confirms nothing about ids, tokens, or configuration.
      if (
        !workflow ||
        !workflow.enabled ||
        !trigger ||
        trigger.token !== token ||
        req.method !== trigger.method
      ) {
        return reply.code(404).send({ error: 'Not found' })
      }

      const headers: Record<string, string> = {}
      for (const [key, value] of Object.entries(req.headers)) {
        if (HEADER_DENYLIST.has(key.toLowerCase()) || value === undefined) continue
        headers[key] = Array.isArray(value) ? value.join(', ') : value
      }
      const query: Record<string, string> = {}
      for (const [key, value] of Object.entries((req.query as Record<string, unknown>) ?? {})) {
        if (typeof value === 'string') query[key] = value
      }
      const eventId = randomUUID()
      dbEnqueueWebhookEvent({
        workflowId,
        eventId,
        receivedAt: new Date().toISOString(),
        item: {
          connectionId: `${WEBHOOK_CONNECTOR_ID}:${workflowId}`,
          connectorId: WEBHOOK_CONNECTOR_ID,
          externalId: eventId,
          title: `Webhook ${trigger.method}`,
          raw: {
            body: req.body ?? null,
            headers,
            query,
            method: req.method,
            receivedAt: new Date().toISOString()
          }
        }
      })
      log.info(`[webhook] queued event for workflow ${workflowId}`)
      onEnqueued()
      return reply.code(202).send({ accepted: true })
    }
  })
}
