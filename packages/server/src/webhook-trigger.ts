import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import type { TriggerConfig, WebhookTriggerConfig } from '@vornrun/shared/types'
import { dbGetWorkflow, dbEnqueueWebhookEvent } from './database'
import log from './logger'

export const WEBHOOK_CONNECTOR_ID = 'webhook'

/** Headers worth exposing to templates; auth-bearing ones stay out of run records. */
const HEADER_ALLOWLIST = new Set(['content-type', 'user-agent', 'x-event', 'x-request-id'])

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
      // One answer for every miss, so the route confirms nothing about ids or tokens.
      if (!workflow || !workflow.enabled || !trigger || trigger.token !== token) {
        return reply.code(404).send({ error: 'Not found' })
      }
      if (req.method !== trigger.method) {
        return reply.code(405).send({ error: `This webhook accepts ${trigger.method}` })
      }

      const headers: Record<string, string> = {}
      for (const [key, value] of Object.entries(req.headers)) {
        if (HEADER_ALLOWLIST.has(key) && typeof value === 'string') headers[key] = value
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
