import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

/** Where vornd's hook endpoint is, as the files it registers it with say; undefined until it has. */
export function hookEndpoint(home = os.homedir()): { port: number; token: string } | undefined {
  try {
    const dir = path.join(home, '.vorn')
    const port = Number(fs.readFileSync(path.join(dir, 'port'), 'utf-8').trim())
    const token = fs.readFileSync(path.join(dir, 'token'), 'utf-8').trim()
    return port > 0 && token ? { port, token } : undefined
  } catch {
    return undefined
  }
}

/** Posts a hook event as an agent's hook does; the response is the hook's answer. */
export function postHook(
  endpoint: { port: number; token: string },
  event: Record<string, unknown>,
  init: { terminal?: string; token?: string; method?: string } = {}
): Promise<Response> {
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    authorization: `Bearer ${init.token ?? endpoint.token}`
  }
  if (init.terminal !== undefined) headers['x-vorn-terminal'] = init.terminal
  return fetch(`http://127.0.0.1:${endpoint.port}/hooks`, {
    method: init.method ?? 'POST',
    headers,
    body: init.method === 'GET' ? undefined : JSON.stringify(event)
  })
}
