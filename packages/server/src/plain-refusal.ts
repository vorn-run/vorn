import type { FastifyReply } from 'fastify'

/** Plain text: the SDK reads a refusal's body verbatim into the error it raises. */
export function refuse(reply: FastifyReply, code: number, reason: string): FastifyReply {
  return reply.code(code).type('text/plain; charset=utf-8').send(reason)
}
