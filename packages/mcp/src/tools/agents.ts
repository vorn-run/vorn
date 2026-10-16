import { rpcCall } from '../rpc-client'

/**
 * The error a tool answers when one of `agents` is no agent that is on in
 * Vorn (`agent:list`), or null when every one is. A server that cannot list
 * its agents lets the call through, and vornd decides.
 */
export async function agentsOff(
  agents: readonly string[]
): Promise<{ content: { type: 'text'; text: string }[]; isError: true } | null> {
  if (agents.length === 0) return null
  let listed: unknown
  try {
    listed = await rpcCall<unknown>('agent:list')
  } catch {
    return null
  }
  const on = (Array.isArray(listed) ? listed : [])
    .map((a) => (a && typeof a === 'object' ? (a as { id?: unknown }).id : undefined))
    .filter((id): id is string => typeof id === 'string')
  const off = agents.find((a) => !on.includes(a))
  if (off === undefined) return null
  const which =
    on.length === 0
      ? 'No agent is on; switch one on in Settings › Extensions.'
      : `Agents that are on: ${on.join(', ')}.`
  return {
    content: [{ type: 'text', text: `Error: agent "${off}" is not on in Vorn. ${which}` }],
    isError: true
  }
}
